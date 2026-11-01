//! Dosya gönderme/alma. Referans: `reference/electron/src/features/files.js`.
//!
//!   Gönderen: `file.offer {id,name,size}` → alıcı kullanıcı onaylarsa `file.accept {id}`
//!   Gönderen: `file.chunk {id,data(base64)}` … `file.done {id,sha256}`
//!   Herhangi biri: `file.cancel {id}`
//!
//! Alıcı, kullanıcı onaylamadan diske hiçbir şey yazmaz. Dosya önce `<ad>.part` olarak yazılır;
//! boyut ve SHA-256 tutarsa asıl adına taşınır. Ad temizlenir (klasör dışına yazma engellenir).
//! Gönderen dosyayı 256 KB'lık parçalarla okur; `send` geri basınç uyguladığı için dosyanın
//! tamamı hiçbir zaman bellekte tutulmaz.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use sha2::{Digest, Sha256};

use super::Sender;
use crate::msg::Msg;
use crate::net::keys::B64STD;
use crate::state::{TransferDir, TransferStatus, TransferView};

pub const CHUNK: usize = 256 * 1024;
/// JavaScript `Number.isSafeInteger` üst sınırı (referans bundan büyük boyutu reddeder).
const MAX_SAFE_INT: u64 = (1 << 53) - 1;
/// Aynı anda tutulan gelen kayıt sınırı (kötü niyetli eşin listeyi şişirmesini önler).
const MAX_INCOMING: usize = 1000;

// ---------------------------------------------------------------------------------------------
// Ad temizleme: `safeName` ile birebir aynı (UTF-16 uzunluklarıyla).

fn is_js_space(c: u16) -> bool {
    matches!(
        c,
        0x09..=0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
    )
}

/// Node `path.posix.extname` (UTF-16 birimleriyle); dönen değer uzantının başlangıç indeksi.
fn extname_start(s: &[u16]) -> Option<usize> {
    let (mut start_dot, mut start_part, mut end): (isize, usize, isize) = (-1, 0, -1);
    let mut matched_slash = true;
    let mut pre_dot_state = 0i32;
    for i in (0..s.len()).rev() {
        let c = s[i];
        if c == b'/' as u16 {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i as isize + 1;
        }
        if c == b'.' as u16 {
            if start_dot == -1 {
                start_dot = i as isize;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
    }
    if start_dot == -1
        || end == -1
        || pre_dot_state == 0
        || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part as isize + 1)
    {
        return None;
    }
    // Adlarımızda '/' olmadığından `end` her zaman uzunluktur.
    Some(start_dot as usize)
}

fn extname(name: &str) -> String {
    let u: Vec<u16> = name.encode_utf16().collect();
    extname_start(&u).map(|i| String::from_utf16_lossy(&u[i..])).unwrap_or_default()
}

fn is_reserved(n: &str) -> bool {
    let b = n.as_bytes();
    let rest = if b.len() >= 3 && ["con", "prn", "aux", "nul"].iter().any(|p| b[..3].eq_ignore_ascii_case(p.as_bytes())) {
        &n[3..]
    } else if b.len() >= 4
        && (b[..3].eq_ignore_ascii_case(b"com") || b[..3].eq_ignore_ascii_case(b"lpt"))
        && b[3].is_ascii_digit()
    {
        &n[4..]
    } else {
        return false;
    };
    // (\..*)?$  — '.' satır sonlandırıcılarıyla eşleşmez
    rest.is_empty() || (rest.starts_with('.') && !rest.chars().any(|c| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')))
}

/// Karşı taraftan gelen dosya adını güvenli bir yerel ada çevirir (referanstaki `safeName`).
pub fn safe_name(name: &str) -> String {
    // basename (posix), ters bölüler de ayırıcı sayılır
    let s = name.replace('\\', "/");
    let s = s.trim_end_matches('/');
    let base = s.rsplit('/').next().unwrap_or("");
    let mut u: Vec<u16> = base
        .encode_utf16()
        .map(|c| if c <= 0x1f || "<>:\"/\\|?*".encode_utf16().any(|x| x == c) { b'_' as u16 } else { c })
        .collect();
    while matches!(u.last(), Some(&c) if c == b'.' as u16 || c == b' ' as u16) {
        u.pop();
    }
    let a = u.iter().position(|&c| !is_js_space(c)).unwrap_or(u.len());
    let b = u.iter().rposition(|&c| !is_js_space(c)).map(|i| i + 1).unwrap_or(a);
    let mut u = u[a..b.max(a)].to_vec();
    let dot = [b'.' as u16];
    if u.is_empty() || u == dot || u == [b'.' as u16, b'.' as u16] {
        u = "dosya".encode_utf16().collect();
    }
    if is_reserved(&String::from_utf16_lossy(&u)) {
        u.insert(0, b'_' as u16);
    }
    if u.len() > 200 {
        let ext: Vec<u16> = extname_start(&u).map(|i| u[i..].iter().take(20).copied().collect()).unwrap_or_default();
        let mut n: Vec<u16> = u[..200 - ext.len()].to_vec();
        n.extend(ext);
        u = n;
    }
    String::from_utf16_lossy(&u)
}

fn part_of(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// `dir/name`, varsa `name (2).ext`, `name (3).ext`… (`.part` dosyaları da dolu sayılır).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let ext = extname(name);
    let base = &name[..name.len() - ext.len()];
    let mut p = dir.join(name);
    let mut i = 2u64;
    while p.exists() || part_of(&p).exists() {
        p = dir.join(format!("{base} ({i}){ext}"));
        i += 1;
    }
    p
}

// ---------------------------------------------------------------------------------------------

fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

struct Transfer {
    id: String,
    dir: TransferDir,
    name: String,
    size: u64,
    done: u64,
    status: TransferStatus,
    /// Giden: kaynak dosya. Gelen: tamamlanınca son yol.
    path: Option<PathBuf>,
    final_path: Option<PathBuf>,
    part: Option<PathBuf>,
    file: Option<BufWriter<File>>,
    hash: Option<Sha256>,
}

impl Transfer {
    fn active(&self) -> bool {
        matches!(
            self.status,
            TransferStatus::Waiting | TransferStatus::Sending | TransferStatus::Offered | TransferStatus::Receiving
        )
    }

    fn drop_incoming(&mut self, status: TransferStatus) {
        self.status = status;
        self.file = None; // önce kapat (Windows'ta açık dosya silinemez)
        self.hash = None;
        if let Some(part) = self.part.take() {
            let _ = fs::remove_file(part);
        }
        self.final_path = None;
    }
}

type OnChange = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    send: Sender,
    download_dir: Arc<RwLock<PathBuf>>,
    list: Mutex<Vec<Transfer>>,
    on_change: RwLock<Option<OnChange>>,
    last_progress: Mutex<Instant>,
}

#[derive(Clone)]
pub struct FileShare {
    inner: Arc<Inner>,
}

impl FileShare {
    pub fn new(send: Sender, download_dir: Arc<RwLock<PathBuf>>) -> Self {
        FileShare {
            inner: Arc::new(Inner {
                send,
                download_dir,
                list: Mutex::new(Vec::new()),
                on_change: RwLock::new(None),
                last_progress: Mutex::new(Instant::now()),
            }),
        }
    }

    /// Liste değiştiğinde çağrılır (ilerleme için en fazla ~10 kez/sn). Arayüzü yeniden çizdirmek için.
    pub fn set_on_change(&self, f: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self.inner.on_change.write().unwrap_or_else(|e| e.into_inner()) = f;
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Transfer>> {
        self.inner.list.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        let f = self.inner.on_change.read().ok().and_then(|f| f.clone());
        if let Some(f) = f {
            f();
        }
    }

    fn progress(&self) {
        let mut t = self.inner.last_progress.lock().unwrap_or_else(|e| e.into_inner());
        if t.elapsed() >= Duration::from_millis(100) {
            *t = Instant::now();
            drop(t);
            self.changed();
        }
    }

    fn send(&self, m: Msg) -> bool {
        (self.inner.send)(&m)
    }

    pub fn list(&self) -> Vec<TransferView> {
        self.lock()
            .iter()
            .map(|t| TransferView {
                id: t.id.clone(),
                dir: t.dir,
                name: t.name.clone(),
                size: t.size,
                done: t.done,
                status: t.status,
                path: t.path.clone(),
            })
            .collect()
    }

    /// Bitmiş (gönderildi/alındı/reddedildi/iptal/hata) kayıtları listeden kaldırır.
    pub fn clear_finished(&self) {
        self.lock().retain(|t| t.active());
        self.changed();
    }

    // ---- gönderen ----

    /// Dosyayı karşı tarafa önerir. Bağlı değilse `NotConnected` hatası.
    pub fn offer(&self, path: &Path) -> io::Result<String> {
        let md = fs::metadata(path)?;
        if !md.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "yalnızca dosya gönderilebilir"));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "dosya adı yok"))?;
        let id = hex::encode(rand::random::<[u8; 8]>());
        let size = md.len();
        self.lock().push(Transfer {
            id: id.clone(),
            dir: TransferDir::Out,
            name: name.clone(),
            size,
            done: 0,
            status: TransferStatus::Waiting,
            path: Some(path.to_path_buf()),
            final_path: None,
            part: None,
            file: None,
            hash: None,
        });
        if !self.send(Msg::FileOffer { id: id.clone(), name, size }) {
            self.lock().retain(|t| !(t.id == id && t.dir == TransferDir::Out));
            return Err(io::Error::new(io::ErrorKind::NotConnected, "bağlı değil"));
        }
        self.changed();
        Ok(id)
    }

    fn set_out(&self, id: &str, f: impl FnOnce(&mut Transfer)) {
        if let Some(t) = self.lock().iter_mut().find(|t| t.id == id && t.dir == TransferDir::Out) {
            f(t);
        }
    }

    fn start_stream(&self, id: &str) {
        let (path, size) = {
            let mut l = self.lock();
            let Some(t) = l.iter_mut().find(|t| t.id == id && t.dir == TransferDir::Out) else { return };
            if t.status != TransferStatus::Waiting {
                return;
            }
            t.status = TransferStatus::Sending;
            (t.path.clone().unwrap_or_default(), t.size)
        };
        self.changed();
        let me = self.clone();
        let id = id.to_string();
        let r = thread::Builder::new().name("kopru-file-send".into()).spawn({
            let id = id.clone();
            move || me.stream(&id, &path, size)
        });
        if r.is_err() {
            self.set_out(&id, |t| t.status = TransferStatus::Error);
            self.send(Msg::FileCancel { id });
            self.changed();
        }
    }

    fn stream(&self, id: &str, path: &Path, size: u64) {
        let res = (|| -> Result<bool, ()> {
            let mut f = File::open(path).map_err(|_| ())?;
            let mut hash = Sha256::new();
            let mut sent = 0u64;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let still = self
                    .lock()
                    .iter()
                    .any(|t| t.id == id && t.dir == TransferDir::Out && t.status == TransferStatus::Sending);
                if !still {
                    return Ok(false); // iptal edildi
                }
                let mut n = 0;
                while n < CHUNK {
                    match f.read(&mut buf[n..]) {
                        Ok(0) => break,
                        Ok(k) => n += k,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => return Err(()),
                    }
                }
                if n == 0 {
                    break;
                }
                sent += n as u64;
                if sent > size {
                    return Err(()); // dosya gönderim sırasında değişti
                }
                hash.update(&buf[..n]);
                let data = B64STD.encode(&buf[..n]);
                if !self.send(Msg::FileChunk { id: id.to_string(), data }) {
                    return Err(());
                }
                self.set_out(id, |t| t.done = sent);
                self.progress();
            }
            if sent != size {
                return Err(());
            }
            if !self.send(Msg::FileDone { id: id.to_string(), sha256: hex::encode(hash.finalize()) }) {
                return Err(());
            }
            Ok(true)
        })();
        match res {
            Ok(true) => self.set_out(id, |t| {
                if t.status == TransferStatus::Sending {
                    t.status = TransferStatus::Sent
                }
            }),
            Ok(false) => {}
            Err(()) => {
                self.set_out(id, |t| {
                    if t.status == TransferStatus::Sending {
                        t.status = TransferStatus::Error
                    }
                });
                self.send(Msg::FileCancel { id: id.to_string() });
            }
        }
        self.changed();
    }

    // ---- alıcı ----

    /// Kullanıcı gelen dosyayı kabul etti: `.part` oluşturulur ve gönderene haber verilir.
    pub fn accept(&self, id: &str) {
        let ok = {
            let mut l = self.lock();
            let Some(t) = l.iter_mut().find(|t| t.id == id && t.dir == TransferDir::In) else { return };
            if t.status != TransferStatus::Offered {
                return;
            }
            let dir = self.inner.download_dir.read().map(|d| d.clone()).unwrap_or_default();
            let opened = fs::create_dir_all(&dir).and_then(|_| {
                let fin = unique_path(&dir, &t.name);
                let part = part_of(&fin);
                OpenOptions::new().write(true).create_new(true).open(&part).map(|f| (fin, part, f))
            });
            match opened {
                Ok((fin, part, f)) => {
                    t.final_path = Some(fin);
                    t.part = Some(part);
                    t.file = Some(BufWriter::with_capacity(CHUNK, f));
                    t.hash = Some(Sha256::new());
                    t.status = TransferStatus::Receiving;
                    true
                }
                Err(_) => {
                    t.status = TransferStatus::Error;
                    false
                }
            }
        };
        if ok {
            if !self.send(Msg::FileAccept { id: id.into() }) {
                if let Some(t) = self.lock().iter_mut().find(|t| t.id == id && t.dir == TransferDir::In) {
                    t.drop_incoming(TransferStatus::Error);
                }
            }
        } else {
            self.send(Msg::FileCancel { id: id.into() });
        }
        self.changed();
    }

    pub fn reject(&self, id: &str) {
        {
            let mut l = self.lock();
            let Some(t) = l.iter_mut().find(|t| t.id == id && t.dir == TransferDir::In) else { return };
            if t.status != TransferStatus::Offered {
                return;
            }
            t.status = TransferStatus::Rejected;
        }
        self.send(Msg::FileCancel { id: id.into() });
        self.changed();
    }

    /// Giden veya gelen aktarımı iptal eder ve karşı tarafa bildirir.
    pub fn cancel(&self, id: &str) {
        self.cancel_local(id);
        self.send(Msg::FileCancel { id: id.into() });
        self.changed();
    }

    fn cancel_local(&self, id: &str) {
        for t in self.lock().iter_mut().filter(|t| t.id == id) {
            match (t.dir, t.status) {
                (TransferDir::Out, TransferStatus::Waiting | TransferStatus::Sending) => t.status = TransferStatus::Cancelled,
                (TransferDir::In, TransferStatus::Offered | TransferStatus::Receiving) => {
                    t.drop_incoming(TransferStatus::Cancelled)
                }
                _ => {}
            }
        }
    }

    /// Bağlantı koptuğunda yarım kalanları iptal eder (karşı tarafa mesaj göndermez).
    pub fn abort_all(&self) {
        {
            let mut l = self.lock();
            for t in l.iter_mut() {
                match (t.dir, t.status) {
                    (TransferDir::Out, TransferStatus::Waiting | TransferStatus::Sending) => {
                        t.status = TransferStatus::Cancelled
                    }
                    (TransferDir::In, TransferStatus::Offered | TransferStatus::Receiving) => {
                        t.drop_incoming(TransferStatus::Cancelled)
                    }
                    _ => {}
                }
            }
        }
        self.changed();
    }

    /// `file.*` mesajlarını işler; başka türse false.
    pub fn on_message(&self, m: &Msg) -> bool {
        match m {
            Msg::FileOffer { id, name, size } => {
                if !valid_id(id) || *size > MAX_SAFE_INT {
                    return true;
                }
                {
                    let mut l = self.lock();
                    if l.iter().any(|t| t.id == *id && t.dir == TransferDir::In) {
                        return true;
                    }
                    if l.iter().filter(|t| t.dir == TransferDir::In).count() >= MAX_INCOMING {
                        // En eski bitmiş gelen kaydı at; hepsi etkinse yok say.
                        match l.iter().position(|t| t.dir == TransferDir::In && !t.active()) {
                            Some(i) => {
                                l.remove(i);
                            }
                            None => return true,
                        }
                    }
                    l.push(Transfer {
                        id: id.clone(),
                        dir: TransferDir::In,
                        name: safe_name(name),
                        size: *size,
                        done: 0,
                        status: TransferStatus::Offered,
                        path: None,
                        final_path: None,
                        part: None,
                        file: None,
                        hash: None,
                    });
                }
                self.changed();
            }
            Msg::FileAccept { id } => {
                if valid_id(id) {
                    self.start_stream(id);
                }
            }
            Msg::FileChunk { id, data } => {
                if !valid_id(id) {
                    return true;
                }
                let mut fail = false;
                {
                    let mut l = self.lock();
                    let Some(t) = l.iter_mut().find(|t| t.id == *id && t.dir == TransferDir::In) else { return true };
                    if t.status != TransferStatus::Receiving {
                        return true;
                    }
                    // Çözmeden önce boyut sınaması: 4 karakter en az 3 bayt eder (dolgu en fazla 2 bayt düşürür),
                    // böylece bildirilenden çok büyük bir parça için bellek ayrılmaz.
                    let room = t.size - t.done;
                    let min_decoded = (data.len() as u64 / 4 * 3).saturating_sub(2);
                    let buf = if min_decoded > room { None } else { B64STD.decode(data.as_bytes()).ok() };
                    match buf {
                        Some(buf) if t.done + buf.len() as u64 <= t.size => {
                            t.done += buf.len() as u64;
                            if let Some(h) = t.hash.as_mut() {
                                h.update(&buf);
                            }
                            let w = t.file.as_mut().map(|f| f.write_all(&buf));
                            if !matches!(w, Some(Ok(()))) {
                                t.drop_incoming(TransferStatus::Error);
                                fail = true;
                            }
                        }
                        _ => {
                            // bildirilenden fazla bayt veya bozuk veri
                            t.drop_incoming(TransferStatus::Cancelled);
                            fail = true;
                        }
                    }
                }
                if fail {
                    self.send(Msg::FileCancel { id: id.clone() });
                    self.changed();
                } else {
                    self.progress();
                }
            }
            Msg::FileDone { id, sha256 } => {
                if !valid_id(id) {
                    return true;
                }
                {
                    let mut l = self.lock();
                    let Some(t) = l.iter_mut().find(|t| t.id == *id && t.dir == TransferDir::In) else { return true };
                    if t.status != TransferStatus::Receiving {
                        return true;
                    }
                    let digest = t.hash.take().map(|h| hex::encode(h.finalize())).unwrap_or_default();
                    let flushed = t.file.take().map(|f| f.into_inner().map_err(|_| ()).and_then(|f| f.sync_all().map_err(|_| ())));
                    let good = t.done == t.size && digest == sha256.to_ascii_lowercase() && matches!(flushed, Some(Ok(())));
                    if !good {
                        t.drop_incoming(TransferStatus::Error);
                    } else {
                        let (part, fin) = (t.part.take().unwrap_or_default(), t.final_path.clone().unwrap_or_default());
                        // Nadir yarış: aynı adla başka dosya oluştuysa yeni ad seç.
                        let fin = if fin.exists() {
                            unique_path(fin.parent().unwrap_or(Path::new(".")), &t.name)
                        } else {
                            fin
                        };
                        match fs::rename(&part, &fin) {
                            Ok(()) => {
                                t.status = TransferStatus::Received;
                                t.path = Some(fin.clone());
                                t.final_path = Some(fin);
                            }
                            Err(_) => {
                                t.part = Some(part);
                                t.drop_incoming(TransferStatus::Error);
                            }
                        }
                    }
                }
                self.changed();
            }
            Msg::FileCancel { id } => {
                if valid_id(id) {
                    {
                        let mut l = self.lock();
                        for t in l.iter_mut().filter(|t| t.id == *id) {
                            match (t.dir, t.status) {
                                // Henüz kabul edilmemiş öneri iptal edildiyse karşı taraf reddetmiştir.
                                (TransferDir::Out, TransferStatus::Waiting) => t.status = TransferStatus::Rejected,
                                (TransferDir::Out, TransferStatus::Sending) => t.status = TransferStatus::Cancelled,
                                (TransferDir::In, TransferStatus::Offered | TransferStatus::Receiving) => {
                                    t.drop_incoming(TransferStatus::Cancelled)
                                }
                                _ => {}
                            }
                        }
                    }
                    self.changed();
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn safe_name_cases() {
        let cases: &[(&str, &str)] = &[
            ("rapor.pdf", "rapor.pdf"),
            ("../../etc/passwd", "passwd"),
            ("..\\..\\windows\\system32\\x.dll", "x.dll"),
            ("C:\\Users\\a\\b.txt", "b.txt"),
            ("a/b/", "b"),
            ("///", "dosya"),
            ("", "dosya"),
            (".", "dosya"),
            ("..", "dosya"),
            ("...", "dosya"),
            ("con", "_con"),
            ("CON.txt", "_CON.txt"),
            ("com1.tar.gz", "_com1.tar.gz"),
            ("lpt9", "_lpt9"),
            ("coma", "coma"),
            ("nul.", "_nul"),
            ("aux .", "_aux"),
            ("  boşluk  ", "boşluk"),
            ("a<b>c:d\"e|f?g*h", "a_b_c_d_e_f_g_h"),
            ("tab\there\nnew", "tab_here_new"),
            ("dosya. . .", "dosya"),
            (".gizli", ".gizli"),
            ("\u{a0}nbsp\u{a0}", "nbsp"),
            ("a\u{85}", "a\u{85}"),
            ("\u{feff}bom", "bom"),
            ("C:x.txt", "C_x.txt"),
            ("con.\u{2028}x", "con.\u{2028}x"),
            ("x\\", "x"),
            ("/abs/path/ok.txt", "ok.txt"),
        ];
        for (i, o) in cases {
            assert_eq!(safe_name(i), *o, "girdi {i:?}");
        }
        let long = format!("{}.uzantı", "x".repeat(250));
        let s = safe_name(&long);
        assert_eq!(s.encode_utf16().count(), 200);
        assert!(s.ends_with(".uzantı"));
        let s = safe_name(&format!("{}.{}", "y".repeat(230), "z".repeat(30)));
        assert_eq!(s.encode_utf16().count(), 200);
        assert!(s.ends_with(&format!(".{}", "z".repeat(19))));
        let s = safe_name(&format!("{}.png", "😀".repeat(150)));
        assert_eq!(s, format!("{}.png", "😀".repeat(98)));
    }

    #[test]
    fn extname_matches_node() {
        for (i, o) in [("..a", ".a"), (".a", ""), ("a.", "."), ("a..b", ".b"), (".a.b", ".b"), ("...", "."), ("a.b.", "."), ("..", ""), (".", ""), ("a", "")] {
            assert_eq!(extname(i), o, "{i:?}");
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kopru-{tag}-{}", hex::encode(rand::random::<[u8; 6]>())));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unique_names() {
        let d = tmpdir("uniq");
        assert_eq!(unique_path(&d, "a.txt"), d.join("a.txt"));
        fs::write(d.join("a.txt"), b"").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (2).txt"));
        fs::write(d.join("a (2).txt.part"), b"").unwrap();
        assert_eq!(unique_path(&d, "a.txt"), d.join("a (3).txt"));
        fs::write(d.join(".gizli"), b"").unwrap();
        assert_eq!(unique_path(&d, ".gizli"), d.join(".gizli (2)"));
        fs::write(d.join("arsiv.tar.gz"), b"").unwrap();
        assert_eq!(unique_path(&d, "arsiv.tar.gz"), d.join("arsiv.tar (2).gz"));
        let _ = fs::remove_dir_all(d);
    }

    /// İki FileShare'i ayrı teslim iş parçacıklarıyla birbirine bağlar (gerçek bağlantı gibi sıralı, eşzamansız).
    fn wired(dir_a: PathBuf, dir_b: PathBuf) -> (FileShare, FileShare) {
        let (ta, ra) = mpsc::sync_channel::<Msg>(8);
        let (tb, rb) = mpsc::sync_channel::<Msg>(8);
        let a = FileShare::new(Arc::new(move |m: &Msg| ta.send(m.clone()).is_ok()), Arc::new(RwLock::new(dir_a)));
        let b = FileShare::new(Arc::new(move |m: &Msg| tb.send(m.clone()).is_ok()), Arc::new(RwLock::new(dir_b)));
        let (a2, b2) = (a.clone(), b.clone());
        thread::spawn(move || {
            for m in ra {
                b2.on_message(&m);
            }
        });
        thread::spawn(move || {
            for m in rb {
                a2.on_message(&m);
            }
        });
        (a, b)
    }

    fn wait<F: Fn() -> bool>(what: &str, f: F) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(20), "zaman aşımı: {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn status(fs_: &FileShare, id: &str, dir: TransferDir) -> Option<TransferStatus> {
        fs_.list().into_iter().find(|t| t.id == id && t.dir == dir).map(|t| t.status)
    }

    fn random_file(dir: &Path, name: &str, len: usize) -> (PathBuf, Vec<u8>) {
        let data: Vec<u8> = (0..len).map(|_| rand::random::<u8>()).collect();
        let p = dir.join(name);
        fs::write(&p, &data).unwrap();
        (p, data)
    }

    #[test]
    fn end_to_end_transfer() {
        let (src, dst) = (tmpdir("src"), tmpdir("dst").join("alt klasör"));
        let (a, b) = wired(tmpdir("unused"), dst.clone());
        // Birden çok parça + son parça kısmi; ayrıca boş dosya.
        let (p, data) = random_file(&src, "büyük.bin", CHUNK * 3 + 12345);
        let id = a.offer(&p).unwrap();
        wait("öneri", || status(&b, &id, TransferDir::In) == Some(TransferStatus::Offered));
        assert!(!dst.exists(), "kabul edilmeden diske yazılmamalı");
        assert_eq!(b.list()[0].size, data.len() as u64);
        b.accept(&id);
        wait("alındı", || status(&b, &id, TransferDir::In) == Some(TransferStatus::Received));
        wait("gönderildi", || status(&a, &id, TransferDir::Out) == Some(TransferStatus::Sent));
        let got = b.list().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(got.path.as_deref(), Some(dst.join("büyük.bin").as_path()));
        assert_eq!(fs::read(dst.join("büyük.bin")).unwrap(), data);
        assert!(!dst.join("büyük.bin.part").exists());
        assert_eq!(got.done, data.len() as u64);

        // Aynı ad tekrar => "büyük (2).bin"
        let id2 = a.offer(&p).unwrap();
        wait("öneri2", || status(&b, &id2, TransferDir::In) == Some(TransferStatus::Offered));
        b.accept(&id2);
        wait("alındı2", || status(&b, &id2, TransferDir::In) == Some(TransferStatus::Received));
        assert_eq!(fs::read(dst.join("büyük (2).bin")).unwrap(), data);

        // Boş dosya
        let (p0, _) = random_file(&src, "boş.txt", 0);
        let id3 = a.offer(&p0).unwrap();
        wait("öneri3", || status(&b, &id3, TransferDir::In) == Some(TransferStatus::Offered));
        b.accept(&id3);
        wait("alındı3", || status(&b, &id3, TransferDir::In) == Some(TransferStatus::Received));
        assert_eq!(fs::read(dst.join("boş.txt")).unwrap().len(), 0);

        // Reddetme
        let id4 = a.offer(&p).unwrap();
        wait("öneri4", || status(&b, &id4, TransferDir::In) == Some(TransferStatus::Offered));
        b.reject(&id4);
        wait("reddedildi", || status(&a, &id4, TransferDir::Out) == Some(TransferStatus::Rejected));
        assert!(a.offer(&src).is_err(), "klasör gönderilemez");
    }

    /// Mesajları doğrudan besleyerek alıcıyı sınar; gönderilenleri toplar.
    fn receiver(dir: PathBuf) -> (FileShare, Arc<Mutex<Vec<Msg>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o2 = out.clone();
        let fsh = FileShare::new(
            Arc::new(move |m: &Msg| {
                o2.lock().unwrap().push(m.clone());
                true
            }),
            Arc::new(RwLock::new(dir)),
        );
        (fsh, out)
    }

    const ID: &str = "0123456789abcdef";

    fn offer_accept(r: &FileShare, name: &str, size: u64) {
        r.on_message(&Msg::FileOffer { id: ID.into(), name: name.into(), size });
        r.accept(ID);
        assert_eq!(status(r, ID, TransferDir::In), Some(TransferStatus::Receiving));
    }

    #[test]
    fn hash_mismatch_deletes_part() {
        let d = tmpdir("hash");
        let (r, out) = receiver(d.clone());
        offer_accept(&r, "x.txt", 3);
        assert!(d.join("x.txt.part").exists());
        assert!(r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(b"abc") }));
        r.on_message(&Msg::FileDone { id: ID.into(), sha256: "00".repeat(32) });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Error));
        assert!(!d.join("x.txt.part").exists());
        assert!(!d.join("x.txt").exists());
        assert_eq!(out.lock().unwrap()[0], Msg::FileAccept { id: ID.into() });
    }

    #[test]
    fn short_file_is_error() {
        let d = tmpdir("short");
        let (r, _) = receiver(d.clone());
        offer_accept(&r, "x.txt", 4);
        r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(b"abc") });
        let h = hex::encode(Sha256::digest(b"abc"));
        r.on_message(&Msg::FileDone { id: ID.into(), sha256: h });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Error));
        assert!(fs::read_dir(&d).unwrap().next().is_none());
    }

    #[test]
    fn oversize_aborts() {
        let d = tmpdir("over");
        let (r, out) = receiver(d.clone());
        offer_accept(&r, "x.txt", 3);
        r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(b"ab") });
        r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(b"cd") });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Cancelled));
        assert!(!d.join("x.txt.part").exists());
        assert_eq!(out.lock().unwrap().last(), Some(&Msg::FileCancel { id: ID.into() }));
        // Sonraki parçalar ve done yok sayılır.
        r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(b"a") });
        r.on_message(&Msg::FileDone { id: ID.into(), sha256: String::new() });
        assert!(fs::read_dir(&d).unwrap().next().is_none());
        // Büyük bir parça çözülmeden reddedilir.
        let (r, _) = receiver(d.clone());
        offer_accept(&r, "y.txt", 10);
        r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(vec![0u8; 100_000]) });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Cancelled));
        assert!(fs::read_dir(&d).unwrap().next().is_none());
    }

    #[test]
    fn untrusted_input_ignored() {
        let d = tmpdir("bad");
        let (r, out) = receiver(d.clone());
        // geçersiz kimlikler
        for id in ["", "../x", "0123456789ABCDEF", "0123456789abcdef0", "zzzzzzzzzzzzzzzz"] {
            assert!(r.on_message(&Msg::FileOffer { id: id.into(), name: "a".into(), size: 1 }));
        }
        assert!(r.list().is_empty());
        // çok büyük boyut
        r.on_message(&Msg::FileOffer { id: ID.into(), name: "a".into(), size: u64::MAX });
        assert!(r.list().is_empty());
        // kabul edilmeden gelen parça/bitti hiçbir şey yazmaz
        r.on_message(&Msg::FileOffer { id: ID.into(), name: "../../a.txt".into(), size: 1 });
        r.on_message(&Msg::FileChunk { id: ID.into(), data: "YQ==".into() });
        r.on_message(&Msg::FileDone { id: ID.into(), sha256: "x".into() });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Offered));
        assert_eq!(r.list()[0].name, "a.txt");
        // yinelenen öneri yok sayılır
        r.on_message(&Msg::FileOffer { id: ID.into(), name: "b".into(), size: 1 });
        assert_eq!(r.list().len(), 1);
        // bozuk base64
        r.accept(ID);
        r.on_message(&Msg::FileChunk { id: ID.into(), data: "!!!!".into() });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Cancelled));
        assert!(fs::read_dir(&d).unwrap().next().is_none());
        // bilinmeyen kimliğe accept => hiçbir şey
        r.on_message(&Msg::FileAccept { id: "aaaaaaaaaaaaaaaa".into() });
        assert!(!r.on_message(&Msg::ClipText { text: "x".into() }));
        let _ = out;
    }

    #[test]
    fn peer_cancel_mid_transfer_and_abort_all() {
        let d = tmpdir("cancel");
        let (r, _) = receiver(d.clone());
        offer_accept(&r, "x.txt", 10);
        r.on_message(&Msg::FileChunk { id: ID.into(), data: B64STD.encode(b"abc") });
        r.on_message(&Msg::FileCancel { id: ID.into() });
        assert_eq!(status(&r, ID, TransferDir::In), Some(TransferStatus::Cancelled));
        assert!(fs::read_dir(&d).unwrap().next().is_none());

        let (r, _) = receiver(d.clone());
        offer_accept(&r, "y.txt", 10);
        r.on_message(&Msg::FileOffer { id: "1111111111111111".into(), name: "z".into(), size: 1 });
        r.abort_all();
        assert!(r.list().iter().all(|t| t.status == TransferStatus::Cancelled));
        assert!(fs::read_dir(&d).unwrap().next().is_none());
        r.clear_finished();
        assert!(r.list().is_empty());
    }

    #[test]
    fn sender_cancel_stops_stream() {
        let (src, dst) = (tmpdir("csrc"), tmpdir("cdst"));
        let (a, b) = wired(tmpdir("unused2"), dst.clone());
        let (p, _) = random_file(&src, "uzun.bin", CHUNK * 40);
        let id = a.offer(&p).unwrap();
        wait("öneri", || status(&b, &id, TransferDir::In) == Some(TransferStatus::Offered));
        b.accept(&id);
        wait("gönderiliyor", || a.list()[0].done > 0);
        a.cancel(&id);
        wait("alıcı iptal", || status(&b, &id, TransferDir::In) == Some(TransferStatus::Cancelled));
        assert_eq!(status(&a, &id, TransferDir::Out), Some(TransferStatus::Cancelled));
        thread::sleep(Duration::from_millis(100));
        assert!(fs::read_dir(&dst).unwrap().next().is_none(), ".part silinmeli");
    }
}
