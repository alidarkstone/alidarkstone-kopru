//! Rust `Link` ↔ Electron referans çekirdeği (`reference/electron/src/core/link.js`) birlikte çalışma testi.
//! `src/net/mod.rs` içinden `#[cfg(test)]` modülü olarak derlenir; `cargo test` ile çalışır.
//! Node bulunamazsa (KOPRU_NODE, /opt/node22/bin/node, PATH'teki `node`) test atlanır.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::link::tests::{node as rust_node, wait, Node as RustNode};
use super::link::{LinkEvent, MsgHandler};
use crate::features::files::{safe_name, FileShare};
use crate::msg::Msg;
use crate::state::{LinkPhase, TransferDir, TransferStatus};

const TIMEOUT: Duration = Duration::from_secs(20);

fn node_bin() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("KOPRU_NODE") {
        cands.push(p.into());
    }
    cands.push("/opt/node22/bin/node".into());
    cands.push("node".into());
    cands.into_iter().find(|c| {
        Command::new(c).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    })
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/interop/peer.js")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kopru-interop-{tag}-{}", hex::encode(rand::random::<[u8; 6]>())));
    fs::create_dir_all(&d).unwrap();
    d
}

struct NodePeer {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    backlog: Vec<Value>,
}

impl NodePeer {
    fn spawn(node: &Path, dl: &Path) -> Self {
        let mut child = Command::new(node)
            .arg(script())
            .arg(dl)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("node başlatılamadı");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        NodePeer { child, stdin, rx, backlog: Vec::new() }
    }

    fn cmd(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Koşulu sağlayan ilk olayı döner (önce birikmiş olaylara bakar).
    fn wait(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.backlog.iter().position(&pred) {
            return self.backlog.remove(i);
        }
        let t = Instant::now();
        loop {
            let left = TIMEOUT.checked_sub(t.elapsed()).unwrap_or_else(|| panic!("node olayı gelmedi: {what}; birikmiş: {:?}", self.backlog));
            match self.rx.recv_timeout(left) {
                Ok(v) if pred(&v) => return v,
                Ok(v) => self.backlog.push(v),
                Err(_) => panic!("node olayı gelmedi: {what}; birikmiş: {:?}", self.backlog),
            }
        }
    }

    fn wait_state(&mut self, state: &str) -> Value {
        self.wait(&format!("durum {state}"), |v| v["ev"] == "state" && v["s"]["state"] == state)
    }

    fn clear(&mut self) {
        while let Ok(v) = self.rx.try_recv() {
            self.backlog.push(v);
        }
        self.backlog.clear();
    }
}

impl Drop for NodePeer {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{}", json!({"cmd":"quit"}));
        thread::sleep(Duration::from_millis(50));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn next_message(rx: &Receiver<LinkEvent>) -> Msg {
    let t = Instant::now();
    loop {
        let left = TIMEOUT.checked_sub(t.elapsed()).expect("Rust tarafına mesaj gelmedi");
        if let Ok(LinkEvent::Message(m)) = rx.recv_timeout(left) {
            return m;
        }
    }
}

fn local(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn rust_sas(n: &RustNode) -> Option<String> {
    match n.link.phase() {
        LinkPhase::Pairing { sas, .. } => Some(sas),
        _ => None,
    }
}

/// Rust tarafı: Link + FileShare (işleyici okuyucu iş parçacığında).
fn rust_with_files(dl: &Path) -> (RustNode, FileShare) {
    let n = rust_node("Rust Ucu");
    let l = n.link.clone();
    let files = FileShare::new(Arc::new(move |m: &Msg| l.send(m)), Arc::new(RwLock::new(dl.to_path_buf())));
    let f2 = files.clone();
    let h: MsgHandler = Arc::new(move |m: &Msg| f2.on_message(m));
    n.link.set_handler(Some(h));
    (n, files)
}

fn random_bytes(n: usize) -> Vec<u8> {
    (0..n).map(|_| rand::random::<u8>()).collect()
}

#[test]
fn interop_with_electron_reference() {
    let Some(node) = node_bin() else {
        eprintln!("node bulunamadı; birlikte çalışma testi atlandı");
        return;
    };
    let (node_dl, rust_dl, src) = (tmpdir("node-dl"), tmpdir("rust-dl"), tmpdir("src"));
    let mut np = NodePeer::spawn(&node, &node_dl);
    let ready = np.wait("ready", |v| v["ev"] == "ready");
    let node_port = ready["port"].as_u64().unwrap() as u16;
    let node_id = ready["id"].as_str().unwrap().to_string();

    let (r, files) = rust_with_files(&rust_dl);
    let rust_port = r.link.listen(0).unwrap();

    // 1) Node istemci, Rust sunucu: yeni cihaz => iki tarafta aynı kod, iki taraf onaylar.
    np.cmd(json!({"cmd":"connect","port":rust_port}));
    let st = np.wait_state("pairing");
    let node_sas = st["s"]["pairing"]["sas"].as_str().unwrap().to_string();
    wait("rust eşleştirme", || rust_sas(&r).is_some());
    assert_eq!(rust_sas(&r).unwrap(), node_sas, "SAS iki uygulamada aynı olmalı");
    assert_eq!(st["s"]["peer"]["id"], r.id.id.as_str());
    assert_eq!(st["s"]["peer"]["name"], "Rust Ucu");
    assert_eq!(r.link.peer().unwrap().id, node_id);
    assert_eq!(r.link.peer().unwrap().name, "Node Ucu");
    r.link.confirm_pairing(true);
    np.wait("node karşı onayı görür", |v| v["ev"] == "state" && v["s"]["pairing"]["theirs"] == true);
    np.cmd(json!({"cmd":"confirm","ok":true}));
    let st = np.wait_state("connected");
    assert_eq!(st["trusted"], json!([r.id.id]));
    wait("rust bağlandı", || r.link.is_connected());
    assert!(r.trust.list().iter().any(|t| t.id == node_id));

    // 2) clip.text iki yönde
    let text = "Rust'tan merhaba ğüşİ 🚀\n\"tırnak\"";
    assert!(r.link.send(&Msg::ClipText { text: text.into() }));
    let m = np.wait("node clip.text", |v| v["ev"] == "message" && v["m"]["t"] == "clip.text");
    assert_eq!(m["m"]["text"], text);
    np.cmd(json!({"cmd":"send","msg":{"t":"clip.text","text":"Node’dan selam ✓"}}));
    assert_eq!(next_message(&r.rx), Msg::ClipText { text: "Node’dan selam ✓".into() });
    // Bilinmeyen tür ve bozuk alanlı mesaj bağlantıyı koparmaz.
    np.cmd(json!({"cmd":"send","msg":{"t":"input.move","x":1}}));
    np.cmd(json!({"cmd":"send","msg":{"t":"clip.text","text":42}}));
    np.cmd(json!({"cmd":"send","msg":{"t":"clip.text","text":"sonra"}}));
    assert_eq!(next_message(&r.rx), Msg::ClipText { text: "sonra".into() });

    // 3) Dosya Rust -> Node (birden çok 256 KB parça)
    let data = random_bytes(700_001);
    let p = src.join("rust dosyası.bin");
    fs::write(&p, &data).unwrap();
    let id = files.offer(&p).unwrap();
    let off = np.wait("node offer", |v| v["ev"] == "offer");
    assert_eq!(off["id"], id.as_str());
    assert_eq!(off["name"], "rust dosyası.bin");
    assert_eq!(off["size"], 700_001);
    np.cmd(json!({"cmd":"accept","id":id}));
    let rec = np.wait("node received", |v| v["ev"] == "received");
    assert_eq!(fs::read(rec["path"].as_str().unwrap()).unwrap(), data);
    wait("rust gönderildi", || files.list().iter().any(|t| t.id == id && t.status == TransferStatus::Sent));

    // 4) Dosya Node -> Rust
    let data2 = random_bytes(600_000);
    let p2 = src.join("node dosyası.dat");
    fs::write(&p2, &data2).unwrap();
    np.cmd(json!({"cmd":"offer","path":p2}));
    let nid = np.wait("node offered", |v| v["ev"] == "offered")["id"].as_str().unwrap().to_string();
    wait("rust offer", || files.list().iter().any(|t| t.id == nid && t.dir == TransferDir::In && t.status == TransferStatus::Offered));
    files.accept(&nid);
    wait("rust received", || files.list().iter().any(|t| t.id == nid && t.status == TransferStatus::Received));
    assert_eq!(fs::read(rust_dl.join("node dosyası.dat")).unwrap(), data2);
    np.wait("node sent", |v| v["ev"] == "out" && v["id"] == nid.as_str() && v["status"] == "sent");

    // 5) Rust istemci, Node sunucu: iki taraf da tanıyor => kod sorulmadan bağlanır.
    r.link.disconnect();
    np.wait_state("idle");
    np.clear();
    r.link.connect(local(node_port));
    np.wait_state("connected");
    wait("rust doğrudan bağlandı", || r.link.is_connected());
    assert!(!np.backlog.iter().any(|v| v["s"]["state"] == "pairing"), "tanıdık cihazda eşleştirme olmamalı");
    np.cmd(json!({"cmd":"send","msg":{"t":"clip.text","text":"tekrar"}}));
    assert_eq!(next_message(&r.rx), Msg::ClipText { text: "tekrar".into() });
    assert!(r.link.send(&Msg::ClipText { text: "geri".into() }));
    let m = np.wait("node clip 2", |v| v["ev"] == "message" && v["m"]["text"] == "geri");
    assert_eq!(m["m"]["t"], "clip.text");

    // 6) Node bizi unuttu => tek taraf tanısa da iki taraf kod onayına geçer.
    r.link.disconnect();
    np.wait_state("idle");
    np.cmd(json!({"cmd":"forget"}));
    np.clear();
    r.link.connect(local(node_port));
    let st = np.wait_state("pairing");
    wait("rust eşleştirme 2", || rust_sas(&r).is_some());
    assert_eq!(rust_sas(&r).unwrap(), st["s"]["pairing"]["sas"].as_str().unwrap());
    np.cmd(json!({"cmd":"confirm","ok":true}));
    r.link.confirm_pairing(true);
    np.wait_state("connected");
    wait("rust bağlandı 2", || r.link.is_connected());
    r.link.disconnect();
    np.wait_state("idle");

    // 7) Reddetme: Node reddeder, sonra Rust reddeder.
    let r2 = rust_node("Yabancı");
    np.clear();
    r2.link.connect(local(node_port));
    np.wait_state("pairing");
    wait("r2 eşleştirme", || rust_sas(&r2).is_some());
    np.cmd(json!({"cmd":"confirm","ok":false}));
    wait("r2 reddedildi", || r2.link.phase() == LinkPhase::Idle);
    assert_eq!(r2.link.last_error(), "karşı taraf eşleştirmeyi reddetti");
    np.wait_state("idle");
    np.clear();
    r2.link.connect(local(node_port));
    np.wait_state("pairing");
    wait("r2 eşleştirme 2", || rust_sas(&r2).is_some());
    r2.link.confirm_pairing(false);
    let st = np.wait_state("idle");
    assert_eq!(st["s"]["error"], "karşı taraf eşleştirmeyi reddetti");
    assert_eq!(r2.link.last_error(), "eşleştirme reddedildi");

    r.link.close();
    r2.link.close();
    drop(np);
    for d in [node_dl, rust_dl, src] {
        let _ = fs::remove_dir_all(d);
    }
}

#[test]
fn safe_name_matches_reference() {
    let Some(node) = node_bin() else {
        eprintln!("node bulunamadı; safeName karşılaştırması atlandı");
        return;
    };
    let mut cases: Vec<String> = [
        "rapor.pdf", "../../etc/passwd", "..\\..\\windows\\system32\\x.dll", "C:\\Users\\a\\b.txt", "a/b/", "///", "", ".",
        "..", "...", "con", "CON.txt", "com1.tar.gz", "lpt9", "coma", "nul.", "aux .", "  boşluk  ", "a<b>c:d\"e|f?g*h",
        "tab\there\nnew", "dosya. . .", ".gizli", "\u{a0}nbsp\u{a0}", "a\u{85}", "\u{feff}bom", "C:x.txt", "con.\u{2028}x",
        "x\\", "name.", "/abs/path/ok.txt", "trail/", "Prn.\u{3000}", "\u{3000}\u{3000}", "a\u{7f}b", "çok.nokta.lı.ad.tar.gz",
        ". .", "..gizli", "AUX.", "COM0.x", "lpt", "\u{1}\u{2}", "ad\u{2028}", "x.\u{2029}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    cases.push(format!("{}.uzantı", "x".repeat(250)));
    cases.push(format!("{}.{}", "y".repeat(230), "z".repeat(30)));
    cases.push("ğ".repeat(210));
    cases.push(format!("{}.png", "😀".repeat(150)));
    cases.push(format!("{}.{}", "a".repeat(195), "b".repeat(10)));
    cases.push(format!(".{}", "q".repeat(300)));
    let out = Command::new(node).arg(script()).arg("--safename").arg(serde_json::to_string(&cases).unwrap()).output().unwrap();
    assert!(out.status.success());
    let pairs: Vec<(String, String)> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(pairs.len(), cases.len());
    for (input, expected) in pairs {
        assert_eq!(safe_name(&input), expected, "girdi {input:?}");
    }
}

/// Gerçek iki Rust Link üzerinden dosya aktarımı (okuyucu iş parçacığındaki işleyiciyle).
#[test]
fn rust_to_rust_file_over_link() {
    let (a_dl, b_dl, src) = (tmpdir("a"), tmpdir("b"), tmpdir("s"));
    let (a, fa) = rust_with_files(&a_dl);
    let (b, fb) = rust_with_files(&b_dl);
    let port = a.link.listen(0).unwrap();
    b.link.connect(local(port));
    wait("eşleştirme", || rust_sas(&a).is_some() && rust_sas(&b).is_some());
    a.link.confirm_pairing(true);
    b.link.confirm_pairing(true);
    wait("bağlandı", || a.link.is_connected() && b.link.is_connected());
    // 8 MB: yazma kuyruğu sınırının (4 MB) üstünde => geri basınç devrede.
    let data = random_bytes(8 * 1024 * 1024 + 7);
    let p = src.join("büyük.iso");
    fs::write(&p, &data).unwrap();
    let id = fb.offer(&p).unwrap();
    wait("öneri", || fa.list().iter().any(|t| t.id == id));
    fa.accept(&id);
    let t0 = Instant::now();
    while !fa.list().iter().any(|t| t.id == id && t.status == TransferStatus::Received) {
        // Hata ayıklama derlemesinde şifreleme yavaş (~3 MB/sn); bu yüzden uzun süre.
        thread::sleep(Duration::from_millis(20));
        assert!(t0.elapsed() < Duration::from_secs(60), "zaman aşımı: alındı");
    }
    assert_eq!(fs::read(a_dl.join("büyük.iso")).unwrap(), data);
    // Aktarım sırasında bağlantı koparsa .part kalmaz.
    let id2 = fb.offer(&p).unwrap();
    wait("öneri2", || fa.list().iter().any(|t| t.id == id2));
    fa.accept(&id2);
    wait("ilerliyor", || fa.list().iter().any(|t| t.id == id2 && t.done > 0));
    b.link.disconnect();
    wait("a ayrıldı", || a.link.phase() == LinkPhase::Idle);
    fa.abort_all();
    fb.abort_all();
    assert!(!a_dl.join("büyük (2).iso.part").exists());
    assert_eq!(fa.list().iter().find(|t| t.id == id2).unwrap().status, TransferStatus::Cancelled);
    a.link.close();
    b.link.close();
    for d in [a_dl, b_dl, src] {
        let _ = fs::remove_dir_all(d);
    }
}
