//! Şifreli kanal el sıkışması ve AES-256-GCM çerçeve şifrelemesi.
//! Referans: `reference/electron/src/core/channel.js` (tel düzeyinde birebir aynı).
//!
//! 0) COMMIT (açık, yalnızca istemci, ilk çerçeve): `{t:"commit",app,v,h}` — h = SHA-256(istemci HELLO baytları)
//! 1) HELLO (açık): `{t,app,v,role,name,id,eph,nonce}` — ham anahtarlar base64url (dolgusuz).
//!    Sunucu HELLO'sunu hemen gönderir; istemci kendi HELLO'sunu ancak sunucununkini aldıktan sonra gönderir.
//! 2) AUTH  (açık): `{t:"auth", sig}` — Ed25519 imzası(transkript || rol)
//! 3) Sonrası: AES-256-GCM; her yönün ayrı anahtarı ve 4 sıfır bayt + 8 bayt BE sayaçtan oluşan nonce'u var.
//!
//! Transkript = SHA-256("kopru-handshake-v2" || istemci HELLO baytları || sunucu HELLO baytları).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::framing::{read_frame, write_frame, DeadlineReader, MAX_FRAME};
use super::keys::{b64url, fingerprint, hkdf, unb64url, verify, Identity};

pub const APP: &str = "kopru";
/// 2: istemci taahhüdü (v1, ortadakinin kodu denemeyle eşitlemesine açıktı).
pub const VERSION: u64 = 2;
pub const LABEL: &[u8] = b"kopru-handshake-v2";
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const TAG: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Client => "client",
            Role::Server => "server",
        }
    }
    pub fn other(self) -> Role {
        match self {
            Role::Client => Role::Server,
            Role::Server => Role::Client,
        }
    }
}

/// El sıkışma sonucu karşı taraf bilgisi (referanstaki 'ready' olayı).
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    pub peer_id: String,
    /// Ham Ed25519 açık anahtarı, base64url.
    pub peer_raw: String,
    pub peer_name: String,
    pub sas: String,
    pub address: String,
}

/// Bir yönün şifreleyicisi.
pub struct Sealer {
    aead: Aes256Gcm,
    seq: u64,
}

/// Bir yönün çözücüsü.
pub struct Opener {
    aead: Aes256Gcm,
    seq: u64,
}

fn nonce(seq: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&seq.to_be_bytes());
    n
}

impl Sealer {
    pub fn seal(&mut self, plain: &[u8]) -> Result<Vec<u8>, String> {
        if plain.len() + TAG > MAX_FRAME {
            return Err("mesaj çok büyük".into());
        }
        let seq = self.seq;
        self.seq = self.seq.checked_add(1).ok_or("sayaç taştı")?;
        self.aead.encrypt(Nonce::from_slice(&nonce(seq)), plain).map_err(|_| "şifreleme hatası".to_string())
    }
}

impl Opener {
    pub fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() < TAG {
            return Err("kısa paket".into());
        }
        let plain = self
            .aead
            .decrypt(Nonce::from_slice(&nonce(self.seq)), frame)
            .map_err(|_| "şifreli paket doğrulanamadı".to_string())?;
        self.seq = self.seq.checked_add(1).ok_or("sayaç taştı")?;
        Ok(plain)
    }
}

/// Kontrol karakterlerini (U+0000–U+001F) atar ve 64 karakterle sınırlar.
pub fn clean_name(s: &str) -> String {
    s.chars().filter(|c| (*c as u32) > 0x1f).take(64).collect()
}

fn str_field<'a>(m: &'a Value, k: &str) -> Result<&'a str, String> {
    m.get(k).and_then(Value::as_str).ok_or_else(|| "geçersiz base64".to_string())
}

fn key32(s: &str) -> Result<[u8; 32], String> {
    let v = unb64url(s).ok_or("geçersiz base64")?;
    v.try_into().map_err(|_| "geçersiz açık anahtar".to_string())
}

/// `t`, `app` ve `v` alanlarını sınar (eski sürümün HELLO'su da anlaşılır bir sürüm hatası verir).
fn check_app_version(m: &Value, t: &str) -> Result<(), String> {
    let mt = m.get("t").and_then(Value::as_str);
    if (mt != Some(t) && mt != Some("hello")) || m.get("app").and_then(Value::as_str) != Some(APP) {
        return Err("bu bir Köprü cihazı değil".into());
    }
    if m.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return Err(format!("uyumsuz sürüm: {}", m.get("v").cloned().unwrap_or(Value::Null)));
    }
    if mt != Some(t) {
        return Err("bu bir Köprü cihazı değil".into());
    }
    Ok(())
}

fn parse_hello(frame: &[u8], role: Role) -> Result<Value, String> {
    let m: Value = serde_json::from_slice(frame).map_err(|_| "bu bir Köprü cihazı değil".to_string())?;
    check_app_version(&m, "hello")?;
    if m.get("role").and_then(Value::as_str) != Some(role.other().as_str()) {
        return Err("rol hatası".into());
    }
    Ok(m)
}

/// Bir TCP akışı üzerinde el sıkışmayı yapar. Tüm adım `HANDSHAKE_TIMEOUT` içinde bitmeli.
pub fn handshake(
    stream: &TcpStream,
    identity: &Identity,
    name: &str,
    role: Role,
) -> Result<(PeerInfo, Sealer, Opener), String> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut rd = DeadlineReader { stream, deadline };
    let mut wr = stream;
    let _ = stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT));
    let address = stream
        .peer_addr()
        .map(|a| match a.ip() {
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(|v4| v4.to_string()).unwrap_or_else(|| v6.to_string()),
            ip => ip.to_string(),
        })
        .unwrap_or_default();
    let r = handshake_io(&mut rd, &mut wr, identity, name, role, address);
    let _ = stream.set_read_timeout(None);
    r
}

/// Akıştan bağımsız el sıkışma (testlerde de kullanılır).
pub fn handshake_io<R: Read, W: Write>(
    rd: &mut R,
    wr: &mut W,
    identity: &Identity,
    name: &str,
    role: Role,
    address: String,
) -> Result<(PeerInfo, Sealer, Opener), String> {
    let io_err = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            "karşı taraf bağlantıyı kapattı".to_string()
        } else {
            e.to_string()
        }
    };

    // 1) HELLO. İstemci önce yalnızca HELLO'sunun özetini (taahhüt) gönderir, sunucunun HELLO'sunu
    // aldıktan sonra kendi HELLO'sunu açıklar. Böylece hiçbir taraf (ve ortadaki) kendi değerlerini
    // karşı tarafınkini görerek seçemez; 6 haneli kod denenerek eşitlenemez.
    let eph = EphemeralSecret::random_from_rng(rand::rngs::OsRng);
    let eph_pub = PublicKey::from(&eph);
    let my_nonce: [u8; 16] = rand::random();
    let name = {
        let n = clean_name(name);
        if n.is_empty() { "Bilgisayar".to_string() } else { n }
    };
    let hello = json!({
        "t": "hello", "app": APP, "v": VERSION, "role": role.as_str(), "name": name,
        "id": b64url(&identity.raw), "eph": b64url(eph_pub.as_bytes()), "nonce": b64url(&my_nonce),
    });
    let my_hello = serde_json::to_vec(&hello).map_err(|e| e.to_string())?;

    let (frame, m) = match role {
        Role::Client => {
            let commit = json!({"t": "commit", "app": APP, "v": VERSION, "h": b64url(&Sha256::digest(&my_hello))});
            write_frame(wr, &serde_json::to_vec(&commit).map_err(|e| e.to_string())?).map_err(io_err)?;
            let frame = read_frame(rd).map_err(io_err)?;
            let m = parse_hello(&frame, role)?;
            write_frame(wr, &my_hello).map_err(io_err)?;
            (frame, m)
        }
        Role::Server => {
            write_frame(wr, &my_hello).map_err(io_err)?;
            let c = read_frame(rd).map_err(io_err)?;
            let cm: Value = serde_json::from_slice(&c).map_err(|_| "bu bir Köprü cihazı değil".to_string())?;
            check_app_version(&cm, "commit")?;
            let h = unb64url(str_field(&cm, "h")?).ok_or("geçersiz base64")?;
            let frame = read_frame(rd).map_err(io_err)?;
            if Sha256::digest(&frame)[..] != h[..] {
                return Err("taahhüt tutmuyor".into());
            }
            let m = parse_hello(&frame, role)?;
            (frame, m)
        }
    };
    let peer_raw = key32(str_field(&m, "id")?)?;
    let peer_eph = key32(str_field(&m, "eph")?)?;
    unb64url(str_field(&m, "nonce")?).ok_or("geçersiz base64")?;
    if ed25519_dalek::VerifyingKey::from_bytes(&peer_raw).is_err() {
        return Err("geçersiz açık anahtar".into());
    }
    if peer_raw == identity.raw {
        return Err("kendine bağlanılamaz".into());
    }
    let peer_name = match m.get("name") {
        Some(Value::String(s)) if !s.is_empty() => clean_name(s),
        _ => "Bilinmeyen".to_string(),
    };

    let (client_hello, server_hello) = match role {
        Role::Client => (&my_hello[..], &frame[..]),
        Role::Server => (&frame[..], &my_hello[..]),
    };
    let transcript: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(LABEL);
        h.update(client_hello);
        h.update(server_hello);
        h.finalize().into()
    };

    let shared = eph.diffie_hellman(&PublicKey::from(peer_eph));
    if !shared.was_contributory() {
        return Err("geçersiz anahtar değişimi".into());
    }
    let shared = shared.as_bytes();
    let (mut c2s, mut s2c, mut sas4) = ([0u8; 32], [0u8; 32], [0u8; 4]);
    hkdf(shared, &transcript, "kopru c2s", &mut c2s);
    hkdf(shared, &transcript, "kopru s2c", &mut s2c);
    hkdf(shared, &transcript, "kopru sas", &mut sas4);
    let sas = format!("{:06}", u32::from_be_bytes(sas4) % 1_000_000);
    let (send_key, recv_key) = match role {
        Role::Client => (c2s, s2c),
        Role::Server => (s2c, c2s),
    };

    // 2) AUTH
    let mut signed = transcript.to_vec();
    signed.extend_from_slice(role.as_str().as_bytes());
    let sig = identity.sign(&signed);
    let auth = serde_json::to_vec(&json!({"t": "auth", "sig": b64url(&sig)})).map_err(|e| e.to_string())?;
    write_frame(wr, &auth).map_err(io_err)?;

    let frame = read_frame(rd).map_err(io_err)?;
    let m: Value = serde_json::from_slice(&frame).map_err(|_| "kimlik doğrulama bekleniyordu".to_string())?;
    if m.get("t").and_then(Value::as_str) != Some("auth") {
        return Err("kimlik doğrulama bekleniyordu".into());
    }
    let peer_sig = unb64url(m.get("sig").and_then(Value::as_str).unwrap_or("")).ok_or("geçersiz base64")?;
    let mut peer_signed = transcript.to_vec();
    peer_signed.extend_from_slice(role.other().as_str().as_bytes());
    if !verify(&peer_raw, &peer_signed, &peer_sig) {
        return Err("karşı tarafın imzası geçersiz".into());
    }

    let mk = |k: [u8; 32]| Aes256Gcm::new_from_slice(&k).map_err(|_| "anahtar hatası".to_string());
    Ok((
        PeerInfo { peer_id: fingerprint(&peer_raw), peer_raw: b64url(&peer_raw), peer_name, sas, address },
        Sealer { aead: mk(send_key)?, seq: 0 },
        Opener { aead: mk(recv_key)?, seq: 0 },
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    pub fn tcp_pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        (c, s)
    }

    type Side = Result<(PeerInfo, Sealer, Opener), String>;

    fn pair_on(c: TcpStream, s: TcpStream, a: Arc<Identity>, b: Arc<Identity>) -> (Side, Side) {
        let b2 = b.clone();
        let t = thread::spawn(move || handshake(&s, &b2, "B", Role::Server));
        let ra = handshake(&c, &a, "A", Role::Client);
        (ra, t.join().unwrap())
    }

    #[test]
    fn sas_agrees_and_ids_match() {
        let (a, b) = (Arc::new(Identity::generate()), Arc::new(Identity::generate()));
        let (c, s) = tcp_pair();
        let (ra, rb) = pair_on(c, s, a.clone(), b.clone());
        let (ia, mut sa, mut oa) = ra.unwrap();
        let (ib, mut sb, mut ob) = rb.unwrap();
        assert_eq!(ia.sas, ib.sas);
        assert_eq!(ia.sas.len(), 6);
        assert!(ia.sas.bytes().all(|c| c.is_ascii_digit()));
        assert_eq!(ia.peer_id, b.id);
        assert_eq!(ib.peer_id, a.id);
        assert_eq!(ib.peer_name, "A");
        assert_eq!(ia.peer_name, "B");
        assert_eq!(ia.address, "127.0.0.1");
        // Şifreli gidiş-dönüş, iki yön, birden çok mesaj (sayaçlar ilerler).
        for i in 0..5 {
            let ct = sa.seal(format!("gizli-{i}").as_bytes()).unwrap();
            assert_eq!(ob.open(&ct).unwrap(), format!("gizli-{i}").as_bytes());
            let ct = sb.seal(b"yanit").unwrap();
            assert_eq!(oa.open(&ct).unwrap(), b"yanit");
        }
    }

    #[test]
    fn tampered_replayed_or_reordered_rejected() {
        let (a, b) = (Arc::new(Identity::generate()), Arc::new(Identity::generate()));
        let (c, s) = tcp_pair();
        let (ra, rb) = pair_on(c, s, a, b);
        let (_, mut sa, _) = ra.unwrap();
        let (_, _, mut ob) = rb.unwrap();
        let c1 = sa.seal(b"bir").unwrap();
        let c2 = sa.seal(b"iki").unwrap();
        let mut bad = c1.clone();
        bad[0] ^= 1;
        assert!(ob.open(&bad).is_err());
        assert!(ob.open(&c2).is_err(), "sıra değiştirme reddedilmeli");
        assert_eq!(ob.open(&c1).unwrap(), b"bir");
        assert!(ob.open(&c1).is_err(), "tekrar oynatma reddedilmeli");
        assert!(ob.open(&[0u8; 5]).is_err());
    }

    #[test]
    fn self_connect_rejected() {
        let a = Arc::new(Identity::generate());
        let (c, s) = tcp_pair();
        let (ra, rb) = pair_on(c, s, a.clone(), a);
        assert_eq!(ra.err().unwrap(), "kendine bağlanılamaz");
        assert_eq!(rb.err().unwrap(), "kendine bağlanılamaz");
    }

    /// Geçerli HELLO'yu taklit eden ama kötü alanlar gönderen sahte uç.
    fn fake_peer(hello: Value) -> String {
        let a = Identity::generate();
        let (c, s) = tcp_pair();
        let t = thread::spawn(move || {
            let mut s = s;
            let _ = write_frame(&mut s, &serde_json::to_vec(&hello).unwrap());
            let _ = read_frame(&mut s);
            thread::sleep(Duration::from_millis(100));
        });
        let r = handshake(&c, &a, "A", Role::Client);
        let _ = t.join();
        r.err().expect("el sıkışma başarısız olmalı")
    }

    fn good_hello() -> Value {
        let b = Identity::generate();
        json!({"t":"hello","app":"kopru","v":VERSION,"role":"server","name":"B","id":b64url(&b.raw),
               "eph":b64url(PublicKey::from(&EphemeralSecret::random_from_rng(rand::rngs::OsRng)).as_bytes()),
               "nonce":b64url(&[1u8;16])})
    }

    #[test]
    fn hello_validation() {
        let mut h = good_hello();
        h["app"] = json!("x");
        assert_eq!(fake_peer(h), "bu bir Köprü cihazı değil");
        let mut h = good_hello();
        h["v"] = json!(VERSION + 1);
        assert!(fake_peer(h).starts_with("uyumsuz sürüm"));
        let mut h = good_hello();
        h["role"] = json!("client");
        assert_eq!(fake_peer(h), "rol hatası");
        let mut h = good_hello();
        h["id"] = json!("abc");
        assert_eq!(fake_peer(h), "geçersiz açık anahtar");
        let mut h = good_hello();
        h["eph"] = json!(5);
        assert_eq!(fake_peer(h), "geçersiz base64");
        let mut h = good_hello();
        h["nonce"] = json!("a+b");
        assert_eq!(fake_peer(h), "geçersiz base64");
        // Sıfır (düşük dereceli) geçici anahtar => ortak sır sıfır => reddedilir.
        let mut h = good_hello();
        h["eph"] = json!(b64url(&[0u8; 32]));
        assert_eq!(fake_peer(h), "geçersiz anahtar değişimi");
        assert_eq!(fake_peer(json!([1, 2])), "bu bir Köprü cihazı değil");
    }

    #[test]
    fn handshake_times_out_on_silent_peer() {
        let a = Identity::generate();
        let (c, s) = tcp_pair();
        let start = Instant::now();
        // Süreyi kısaltmak için doğrudan DeadlineReader'ı kısa süreyle kullanıyoruz.
        let mut rd = DeadlineReader { stream: &c, deadline: Instant::now() + Duration::from_millis(300) };
        let mut wr = &c;
        let r = handshake_io(&mut rd, &mut wr, &a, "A", Role::Client, String::new());
        assert!(r.err().unwrap().contains("zaman aşımı"));
        assert!(start.elapsed() < Duration::from_secs(3));
        drop(s);
    }

    /// MITM'e karşı: istemci geçici anahtarını sunucunun HELLO'sunu görmeden açıklamamalı.
    /// Aksi halde ortadaki, istemcinin değerlerini gördükten sonra kendi `nonce`unu deneyerek
    /// (~2^20 HKDF, birkaç saniye) iki bacaktaki kodları eşitleyebilir.
    #[test]
    fn client_commits_before_revealing() {
        let a = Identity::generate();
        let (c, s) = tcp_pair();
        let t = thread::spawn(move || handshake(&c, &a, "A", Role::Client));
        let mut s = s;
        let first: Value = serde_json::from_slice(&read_frame(&mut s).unwrap()).unwrap();
        assert!(first.get("eph").is_none() && first.get("nonce").is_none(), "ilk çerçeve geçici anahtarı açıklıyor: {first}");
        // Sunucu HELLO'su gelmeden istemci başka bir şey göndermemeli.
        s.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        assert!(read_frame(&mut s).is_err(), "istemci sunucuyu beklemeden açıkladı");
        drop(s);
        let _ = t.join();
    }

    /// Sunucu, taahhüde uymayan istemci HELLO'sunu reddeder.
    #[test]
    fn commitment_mismatch_rejected() {
        let b = Identity::generate();
        let (c, s) = tcp_pair();
        let t = thread::spawn(move || handshake(&s, &b, "B", Role::Server));
        let mut c = c;
        let _server_hello = read_frame(&mut c).unwrap();
        let mut h = good_hello();
        h["role"] = json!("client");
        let commit = json!({"t":"commit","app":"kopru","v":VERSION,"h":b64url(&[9u8; 32])});
        write_frame(&mut c, &serde_json::to_vec(&commit).unwrap()).unwrap();
        write_frame(&mut c, &serde_json::to_vec(&h).unwrap()).unwrap();
        assert_eq!(t.join().unwrap().err().as_deref(), Some("taahhüt tutmuyor"));
    }

    #[test]
    fn bad_signature_rejected() {
        // Sunucu doğru HELLO gönderir ama başka bir anahtarla imzalar.
        let a = Identity::generate();
        let (c, s) = tcp_pair();
        let got = Arc::new(Mutex::new(None));
        let t = thread::spawn(move || {
            let mut s = s;
            let mut h = good_hello();
            let _ = read_frame(&mut s);
            h["name"] = json!("B");
            write_frame(&mut s, &serde_json::to_vec(&h).unwrap()).unwrap();
            let _ = read_frame(&mut s);
            let other = Identity::generate();
            let sig = other.sign(b"whatever");
            write_frame(&mut s, &serde_json::to_vec(&json!({"t":"auth","sig":b64url(&sig)})).unwrap()).unwrap();
            thread::sleep(Duration::from_millis(100));
        });
        let r = handshake(&c, &a, "A", Role::Client);
        *got.lock().unwrap() = r.err();
        t.join().unwrap();
        assert_eq!(got.lock().unwrap().as_deref(), Some("karşı tarafın imzası geçersiz"));
    }
}
