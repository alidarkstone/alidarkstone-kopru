//! Anahtar yardımcıları: Ed25519 (kalıcı cihaz kimliği), base64url, parmak izi, HKDF.
//! Referans: `reference/electron/src/core/keys.js`.

use std::fs;
use std::io;
use std::path::Path;

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey, KeypairBytes};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

/// base64url, dolgusuz (Node `toString('base64url')` ile aynı çıktı).
const B64URL: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Standart base64, dolgulu yazar; okurken dolgu isteğe bağlı (Node `Buffer.from(s,'base64')` kadar hoşgörülü değil
/// ama Node'un ürettiği her şeyi okur).
pub const B64STD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

pub fn b64url(data: &[u8]) -> String {
    B64URL.encode(data)
}

/// Referanstaki `unb64`: yalnızca `[A-Za-z0-9_-]+` kabul edilir.
pub fn unb64url(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return None;
    }
    B64URL.decode(s).ok()
}

/// Ham açık anahtarın SHA-256'sının ilk 8 baytı, 16 onaltılık hane.
pub fn fingerprint(raw_pub: &[u8]) -> String {
    let d = Sha256::digest(raw_pub);
    hex::encode(&d[..8])
}

pub fn hkdf(secret: &[u8], salt: &[u8], info: &str, out: &mut [u8]) {
    let hk = Hkdf::<Sha256>::new(Some(salt), secret);
    // Uzunluk her zaman 32 veya 4: HKDF sınırının (255*32) çok altında.
    hk.expand(info.as_bytes(), out).expect("hkdf uzunluğu geçerli");
}

/// Kalıcı cihaz kimliği (Ed25519).
pub struct Identity {
    /// Açık anahtarın parmak izi (16 onaltılık hane).
    pub id: String,
    /// Ham 32 baytlık Ed25519 açık anahtarı.
    pub raw: [u8; 32],
    key: SigningKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("id", &self.id).finish_non_exhaustive()
    }
}

impl Identity {
    fn from_key(key: SigningKey) -> Self {
        let raw = key.verifying_key().to_bytes();
        Identity { id: fingerprint(&raw), raw, key }
    }

    pub fn generate() -> Self {
        Self::from_key(SigningKey::generate(&mut rand::rngs::OsRng))
    }

    /// PKCS#8 PEM ("-----BEGIN PRIVATE KEY-----"); Node'un `export({format:'pem',type:'pkcs8'})` çıktısını okur.
    pub fn from_pkcs8_pem(pem: &str) -> io::Result<Self> {
        SigningKey::from_pkcs8_pem(pem.trim())
            .map(Self::from_key)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("kimlik anahtarı okunamadı: {e}")))
    }

    /// PKCS#8 v1 PEM (Node ile aynı biçim, açık anahtar gömülü değil).
    pub fn to_pkcs8_pem(&self) -> String {
        let kp = KeypairBytes { secret_key: self.key.to_bytes(), public_key: None };
        kp.to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
            .map(|z| z.to_string())
            .unwrap_or_default()
    }

    /// Açık anahtar, base64url (güvenilenler listesinde saklanan biçim).
    pub fn raw_b64(&self) -> String {
        b64url(&self.raw)
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.key.sign(msg).to_bytes()
    }

    /// Kimliği dosyadan yükler; yoksa üretip kaydeder. Dosya PEM ya da Electron sürümünün
    /// `identity.json` biçimi (`{"enc":"none","data":"<pem>"}`) olabilir. Bozuk dosya
    /// `<ad>.bad` olarak kenara alınır ve yeni kimlik üretilir.
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                if let Some(id) = Self::parse_stored(&text) {
                    return Ok(id);
                }
                let mut bad = path.as_os_str().to_owned();
                bad.push(".bad");
                let _ = fs::rename(path, &bad);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let id = Self::generate();
        id.save(path)?;
        Ok(id)
    }

    fn parse_stored(text: &str) -> Option<Self> {
        let t = text.trim();
        if t.starts_with("-----BEGIN") {
            return Self::from_pkcs8_pem(t).ok();
        }
        let v: serde_json::Value = serde_json::from_str(t).ok()?;
        if v.get("enc")?.as_str()? == "none" {
            return Self::from_pkcs8_pem(v.get("data")?.as_str()?).ok();
        }
        None
    }

    /// PEM olarak atomik yazar (geçici dosya + yeniden adlandırma); Unix'te 0600 izinli.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        write_private(&tmp, self.to_pkcs8_pem().as_bytes())?;
        fs::rename(&tmp, path)
    }
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    fs::write(path, data)
}

/// Ham Ed25519 açık anahtarıyla imza doğrular.
pub fn verify(raw_pub: &[u8; 32], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(raw_pub) else { return false };
    let Ok(sig) = Signature::from_slice(sig) else { return false };
    vk.verify_strict(msg, &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_roundtrip_keeps_fingerprint() {
        let a = Identity::generate();
        let pem = a.to_pkcs8_pem();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        let b = Identity::from_pkcs8_pem(&pem).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.raw, b.raw);
        assert_eq!(a.id.len(), 16);
        assert!(Identity::from_pkcs8_pem("çöp").is_err());
    }

    #[test]
    fn reads_node_pem() {
        // Node: crypto.generateKeyPairSync('ed25519').privateKey.export({format:'pem',type:'pkcs8'})
        let pem = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIO3zJF9dlGIxivvOmpt7Nyv8/O+Qq1NZeNWNtE/p5KMy\n-----END PRIVATE KEY-----\n";
        let id = Identity::from_pkcs8_pem(pem).unwrap();
        assert_eq!(id.id, "5b680a3631ffc848"); // Node'un hesapladığı parmak izi
        // Bizim yazdığımız PEM, Node'unkiyle bayt bayt aynı olmalı (v1, açık anahtarsız).
        assert_eq!(id.to_pkcs8_pem(), pem);
    }

    #[test]
    fn b64url_rules() {
        let raw = [0xfbu8, 0xff, 0x00, 0x10];
        let s = b64url(&raw);
        assert_eq!(s, "-_8AEA");
        assert_eq!(unb64url(&s).unwrap(), raw);
        assert!(unb64url("").is_none());
        assert!(unb64url("ab+c").is_none());
        assert!(unb64url("abc=").is_none());
    }

    #[test]
    fn sign_verify() {
        let a = Identity::generate();
        let sig = a.sign(b"x");
        assert!(verify(&a.raw, b"x", &sig));
        assert!(!verify(&a.raw, b"y", &sig));
        assert!(!verify(&a.raw, b"x", &sig[..10]));
    }

    #[test]
    fn load_or_create_persists_and_recovers() {
        let dir = std::env::temp_dir().join(format!("kopru-id-{}", hex::encode(rand::random::<[u8; 6]>())));
        let p = dir.join("identity.pem");
        let a = Identity::load_or_create(&p).unwrap();
        let b = Identity::load_or_create(&p).unwrap();
        assert_eq!(a.id, b.id);
        // Electron biçimi
        let j = serde_json::json!({"enc":"none","data":a.to_pkcs8_pem()}).to_string();
        fs::write(&p, j).unwrap();
        assert_eq!(Identity::load_or_create(&p).unwrap().id, a.id);
        // Bozuk dosya kenara alınır
        fs::write(&p, "bozuk").unwrap();
        let c = Identity::load_or_create(&p).unwrap();
        assert_ne!(c.id, a.id);
        assert!(dir.join("identity.pem.bad").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
