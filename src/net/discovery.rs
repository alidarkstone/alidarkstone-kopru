//! Yerel ağda cihaz bulma: her 3 saniyede bir UDP yayını (`{"app":"kopru","v":1,"id","name","port"}`).
//! Bu bilgi yalnızca listeyi doldurmak içindir, güvenilmez; kimlik bağlantıda el sıkışmayla doğrulanır.
//! Referans: `reference/electron/src/core/discovery.js`.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub const DISCOVERY_PORT: u16 = 47911;
pub const EVERY: Duration = Duration::from_secs(3);
pub const EXPIRE: Duration = Duration::from_secs(10);
const MAX_DATAGRAM: usize = 1024;

/// Keşfedilen cihaz: (kimlik, ad, adres, tcp portu).
pub type Found = (String, String, String, u16);

#[derive(Debug, Clone, PartialEq)]
struct Peer {
    id: String,
    name: String,
    address: String,
    port: u16,
    seen: Instant,
}

/// Bir duyuruyu katı biçimde doğrular. Geçersizse veya kendimizden geldiyse None.
pub fn parse_announce(buf: &[u8], from: IpAddr, my_id: &str) -> Option<Found> {
    if buf.len() > MAX_DATAGRAM {
        return None;
    }
    let m: Value = serde_json::from_slice(buf).ok()?;
    if m.get("app")?.as_str()? != "kopru" || m.get("v")?.as_u64()? != 1 {
        return None;
    }
    let id = m.get("id")?.as_str()?;
    if id == my_id || id.len() != 16 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let port = match m.get("port")? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0).map(|f| f as u64))?,
        Value::String(s) => s.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    if !(1..=65535).contains(&port) {
        return None;
    }
    let address = match from {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(from),
        ip => ip,
    }
    .to_string();
    let name = match m.get("name") {
        Some(Value::String(s)) => s.chars().filter(|c| (*c as u32) > 0x1f).take(64).collect::<String>(),
        _ => String::new(),
    };
    let name = if name.is_empty() { address.clone() } else { name };
    Some((id.to_string(), name, address, port as u16))
}

/// 255.255.255.255 ve her IPv4 arayüzünün yönlendirilmiş yayın adresi.
pub fn broadcast_addresses() -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::BROADCAST];
    if let Ok(list) = if_addrs::get_if_addrs() {
        for i in list {
            if let if_addrs::IfAddr::V4(a) = i.addr {
                if a.ip.is_loopback() || a.netmask.is_unspecified() {
                    continue;
                }
                let ip = u32::from(a.ip);
                let mask = u32::from(a.netmask);
                let b = Ipv4Addr::from((ip & mask) | !mask);
                if !out.contains(&b) {
                    out.push(b);
                }
            }
        }
    }
    out
}

pub struct DiscoveryConfig {
    pub udp_port: u16,
    /// Yayın adreslerine ek olarak duyurunun gönderileceği adresler (testler için).
    pub extra_targets: Vec<SocketAddr>,
    pub broadcast: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        DiscoveryConfig { udp_port: DISCOVERY_PORT, extra_targets: Vec::new(), broadcast: true }
    }
}

struct Shared {
    peers: Mutex<HashMap<String, Peer>>,
    stop: AtomicBool,
}

pub struct Discovery {
    shared: Arc<Shared>,
}

impl Discovery {
    /// UDP 47911'de dinlemeye ve duyurmaya başlar. `name` canlı okunur (ad değişince bir sonraki duyuruda gider).
    pub fn start(id: String, name: Arc<RwLock<String>>, tcp_port: u16) -> io::Result<Self> {
        Self::start_with(id, name, tcp_port, DiscoveryConfig::default())
    }

    pub fn start_with(id: String, name: Arc<RwLock<String>>, tcp_port: u16, cfg: DiscoveryConfig) -> io::Result<Self> {
        let sock = UdpSocket::bind(("0.0.0.0", cfg.udp_port))?;
        let _ = sock.set_broadcast(true);
        sock.set_read_timeout(Some(Duration::from_millis(250)))?;
        let shared = Arc::new(Shared { peers: Mutex::new(HashMap::new()), stop: AtomicBool::new(false) });
        let sh = shared.clone();
        thread::Builder::new().name("darkdrop-discovery".into()).spawn(move || {
            let mut buf = [0u8; 2048];
            let mut last: Option<Instant> = None;
            while !sh.stop.load(Ordering::SeqCst) {
                if last.map(|t| t.elapsed() >= EVERY).unwrap_or(true) {
                    last = Some(Instant::now());
                    let n = name.read().map(|n| n.clone()).unwrap_or_default();
                    let msg = json!({"app":"kopru","v":1,"id":id,"name":n,"port":tcp_port}).to_string();
                    if cfg.broadcast {
                        for a in broadcast_addresses() {
                            let _ = sock.send_to(msg.as_bytes(), (a, cfg.udp_port));
                        }
                    }
                    for t in &cfg.extra_targets {
                        let _ = sock.send_to(msg.as_bytes(), t);
                    }
                    let mut p = sh.peers.lock().unwrap_or_else(|e| e.into_inner());
                    p.retain(|_, v| v.seen.elapsed() <= EXPIRE);
                }
                if let Ok((n, from)) = sock.recv_from(&mut buf) {
                    // 1024'ten büyük datagramlar (kesilmiş olsa bile) reddedilir.
                    if n > MAX_DATAGRAM {
                        continue;
                    }
                    if let Some((pid, pname, address, port)) = parse_announce(&buf[..n], from.ip(), &id) {
                        let mut p = sh.peers.lock().unwrap_or_else(|e| e.into_inner());
                        // Sahte duyurularla belleğin şişmesini önle.
                        if p.len() >= 256 && !p.contains_key(&pid) {
                            continue;
                        }
                        p.insert(pid.clone(), Peer { id: pid, name: pname, address, port, seen: Instant::now() });
                    }
                }
            }
        })?;
        Ok(Discovery { shared })
    }

    /// Son 10 saniyede duyuru yapan cihazlar, ada göre sıralı.
    pub fn list(&self) -> Vec<Found> {
        let p = self.shared.peers.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<Found> = p
            .values()
            .filter(|x| x.seen.elapsed() <= EXPIRE)
            .map(|x| (x.id.clone(), x.name.clone(), x.address.clone(), x.port))
            .collect();
        v.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
        v
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "00112233445566ff";
    fn ip() -> IpAddr {
        "192.168.1.5".parse().unwrap()
    }

    #[test]
    fn strict_validation() {
        let ok = br#"{"app":"kopru","v":1,"id":"0123456789abcdef","name":"Mac\u0001Book","port":47910}"#;
        assert_eq!(
            parse_announce(ok, ip(), ME),
            Some(("0123456789abcdef".into(), "MacBook".into(), "192.168.1.5".into(), 47910))
        );
        let bad: &[&[u8]] = &[
            br#"{"app":"kopru","v":1,"id":"00112233445566ff","name":"me","port":47910}"#, // kendimiz
            br#"{"app":"x","v":1,"id":"0123456789abcdef","port":47910}"#,
            br#"{"app":"kopru","v":2,"id":"0123456789abcdef","port":47910}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789ABCDEF","port":47910}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789abcde","port":47910}"#,
            br#"{"app":"kopru","v":1,"id":5,"port":47910}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789abcdef","port":0}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789abcdef","port":65536}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789abcdef","port":1.5}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789abcdef","port":-1}"#,
            br#"{"app":"kopru","v":1,"id":"0123456789abcdef"}"#,
            b"not json",
            b"[]",
        ];
        for b in bad {
            assert_eq!(parse_announce(b, ip(), ME), None, "{}", String::from_utf8_lossy(b));
        }
        // Ad yoksa adres kullanılır; uzun ad kesilir; port metin olabilir (Number() gibi).
        let r = parse_announce(br#"{"app":"kopru","v":1,"id":"0123456789abcdef","port":"80"}"#, ip(), ME).unwrap();
        assert_eq!((r.1.as_str(), r.3), ("192.168.1.5", 80));
        let long = format!(r#"{{"app":"kopru","v":1,"id":"0123456789abcdef","name":"{}","port":1}}"#, "ç".repeat(100));
        assert_eq!(parse_announce(long.as_bytes(), ip(), ME).unwrap().1.chars().count(), 64);
        let huge = format!(r#"{{"app":"kopru","v":1,"id":"0123456789abcdef","name":"{}","port":1}}"#, "a".repeat(1100));
        assert_eq!(parse_announce(huge.as_bytes(), ip(), ME), None);
    }

    #[test]
    fn two_instances_find_each_other() {
        // Her örnek kendi portunu dinler ve diğerinin portuna duyurur (aynı makinede iki örnek).
        let pa = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let pb = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let na = Arc::new(RwLock::new("Alfa".to_string()));
        let a = Discovery::start_with(
            "aaaaaaaaaaaaaaaa".into(),
            na.clone(),
            1111,
            DiscoveryConfig { udp_port: pa, extra_targets: vec![SocketAddr::from(([127, 0, 0, 1], pb))], broadcast: false },
        )
        .unwrap();
        let b = Discovery::start_with(
            "bbbbbbbbbbbbbbbb".into(),
            Arc::new(RwLock::new("Beta".into())),
            2222,
            DiscoveryConfig { udp_port: pb, extra_targets: vec![SocketAddr::from(([127, 0, 0, 1], pa))], broadcast: false },
        )
        .unwrap();
        let t = Instant::now();
        while (a.list().is_empty() || b.list().is_empty()) && t.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(a.list(), vec![("bbbbbbbbbbbbbbbb".into(), "Beta".into(), "127.0.0.1".into(), 2222)]);
        assert_eq!(b.list(), vec![("aaaaaaaaaaaaaaaa".into(), "Alfa".into(), "127.0.0.1".into(), 1111)]);
        // Çöp datagram hiçbir şeyi bozmaz.
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.send_to(&[0xffu8; 1500], ("127.0.0.1", pa)).unwrap();
        s.send_to(b"{}", ("127.0.0.1", pa)).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(a.list().len(), 1);
        a.stop();
        b.stop();
    }
}
