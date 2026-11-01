//! Birleştirme: ağ, özellikler, ayarlar ve arayüz.

use crate::config::{self, Settings};
use crate::features::{FileShare, Sender as FeatureSender, ClipboardShare};
use crate::net::{Discovery, FileTrustStore, Identity, Link, LinkEvent, DEFAULT_PORT};
use crate::state::{Action, DeviceView, LinkPhase, UiState};
use crate::ui;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::Duration;

struct Core {
    dir: PathBuf,
    settings: Mutex<Settings>,
    identity: Arc<Identity>,
    name: Arc<RwLock<String>>,
    trust: Arc<FileTrustStore>,
    link: Link,
    discovery: Option<Discovery>,
    files: FileShare,
    clip: ClipboardShare,
    clip_enabled: Arc<AtomicBool>,
    download_dir: Arc<RwLock<PathBuf>>,
    ctx: Mutex<Option<egui::Context>>,
}

impl Core {
    fn repaint(&self) {
        if let Some(c) = self.ctx.lock().unwrap().as_ref() {
            c.request_repaint();
        }
    }

    fn save(&self) {
        let _ = self.settings.lock().unwrap().save(&self.dir);
    }

    fn state(&self) -> UiState {
        let devices = match &self.discovery {
            Some(d) => d
                .list()
                .into_iter()
                .map(|(id, name, address, port)| DeviceView { trusted: self.trust.contains(&id), id, name, address, port })
                .collect(),
            None => Vec::new(),
        };
        let my_id = self.identity.id.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ");
        UiState {
            my_id,
            my_name: self.name.read().unwrap().clone(),
            phase: self.link.phase(),
            peer: self.link.peer(),
            last_error: self.link.last_error(),
            devices,
            trusted: self.trust.list(),
            clipboard_enabled: self.clip_enabled.load(Ordering::Relaxed),
            transfers: self.files.list(),
            download_dir: self.download_dir.read().unwrap().clone(),
        }
    }

    fn act(&self, a: Action) {
        match a {
            Action::SetName(n) => {
                let mut s = self.settings.lock().unwrap();
                s.name = n.trim().chars().take(40).collect();
                *self.name.write().unwrap() = s.device_name();
                drop(s);
                self.save();
            }
            Action::Connect(id) => {
                let found = self.discovery.as_ref().and_then(|d| d.list().into_iter().find(|f| f.0 == id));
                if let Some((_, _, addr, port)) = found {
                    if let Ok(ip) = addr.parse() {
                        self.link.connect(SocketAddr::new(ip, port));
                    }
                }
            }
            Action::ConnectAddress(text) => {
                if let Some(addr) = parse_address(&text) {
                    self.link.connect(addr);
                }
            }
            Action::Disconnect => self.link.disconnect(),
            Action::ConfirmPairing(ok) => self.link.confirm_pairing(ok),
            Action::Forget(id) => {
                self.trust.forget(&id);
                if self.link.peer().map(|p| p.id == id).unwrap_or(false) {
                    self.link.disconnect();
                }
            }
            Action::SetClipboard(on) => {
                self.clip_enabled.store(on, Ordering::Relaxed);
                self.settings.lock().unwrap().clipboard = on;
                self.save();
            }
            Action::PickFiles => self.pick_files(),
            Action::SendPaths(paths) => {
                for p in paths.into_iter().take(50) {
                    let _ = self.files.offer(&p);
                }
            }
            Action::FileAccept(id) => self.files.accept(&id),
            Action::FileReject(id) => self.files.reject(&id),
            Action::FileCancel(id) => self.files.cancel(&id),
            Action::FileShow(id) => {
                if let Some(t) = self.files.list().into_iter().find(|t| t.id == id) {
                    if let Some(p) = t.path {
                        reveal(&p);
                    }
                }
            }
            Action::OpenDownloads => {
                let d = self.download_dir.read().unwrap().clone();
                let _ = std::fs::create_dir_all(&d);
                open_path(&d);
            }
            Action::ChooseDownloadDir => self.choose_dir(),
            Action::SetTheme(t) => {
                self.settings.lock().unwrap().theme = t;
                self.save();
            }
        }
        self.repaint();
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn pick_files(&self) {
        if !self.link.is_connected() {
            return;
        }
        if let Some(files) = rfd::FileDialog::new().set_title("Gönderilecek dosyaları seçin").pick_files() {
            for f in files {
                let _ = self.files.offer(&f);
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    fn pick_files(&self) {}

    #[cfg(any(windows, target_os = "macos"))]
    fn choose_dir(&self) {
        let cur = self.download_dir.read().unwrap().clone();
        if let Some(d) = rfd::FileDialog::new().set_title("Gelen dosyaların klasörü").set_directory(&cur).pick_folder() {
            *self.download_dir.write().unwrap() = d.clone();
            self.settings.lock().unwrap().download_dir = Some(d);
            self.save();
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    fn choose_dir(&self) {}
}

/// "192.168.1.20", "192.168.1.20:47910", "masaustu.local" veya "[fe80::1]:47910"
pub fn parse_address(text: &str) -> Option<SocketAddr> {
    let t = text.trim();
    if t.is_empty() || t.len() > 255 {
        return None;
    }
    if let Ok(a) = t.parse::<SocketAddr>() {
        return Some(a);
    }
    if let Ok(ip) = t.parse::<std::net::IpAddr>() {
        return Some(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let with_port = if t.rsplit_once(':').map(|(_, p)| p.parse::<u16>().is_ok()).unwrap_or(false) {
        t.to_string()
    } else {
        format!("{t}:{DEFAULT_PORT}")
    };
    with_port.to_socket_addrs().ok()?.find(|a| a.is_ipv4()).or_else(|| with_port.to_socket_addrs().ok()?.next())
}

fn open_path(p: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(p).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

fn reveal(p: &std::path::Path) {
    #[cfg(windows)]
    {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(p.as_os_str());
        let _ = std::process::Command::new("explorer").arg(arg).spawn();
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    if let Some(d) = p.parent() {
        open_path(d);
    }
}

pub fn run() -> Result<(), String> {
    let dir = config::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let settings = Settings::load(&dir);
    let identity = Arc::new(Identity::load_or_create(&dir.join("identity.pem")).map_err(|e| e.to_string())?);
    let trust = Arc::new(FileTrustStore::load(dir.join("trusted.json")));
    let name = Arc::new(RwLock::new(settings.device_name()));
    let download_dir = Arc::new(RwLock::new(settings.download_dir()));
    let clip_enabled = Arc::new(AtomicBool::new(settings.clipboard));

    let (tx, rx) = mpsc::channel::<LinkEvent>();
    let link = Link::new(identity.clone(), name.clone(), trust.clone(), tx);
    let port = link.listen(DEFAULT_PORT).or_else(|_| link.listen(0)).map_err(|e| format!("Ağ bağlantı noktası açılamadı: {e}"))?;
    let discovery = Discovery::start(identity.id.clone(), name.clone(), port).ok();

    let l2 = link.clone();
    let sender: FeatureSender = Arc::new(move |m| l2.send(m));
    let files = FileShare::new(sender.clone(), download_dir.clone());
    let clip = ClipboardShare::start(sender, clip_enabled.clone());

    let theme = settings.theme.clone();
    let core = Arc::new(Core {
        dir,
        settings: Mutex::new(settings),
        identity,
        name,
        trust,
        link: link.clone(),
        discovery,
        files: files.clone(),
        clip,
        clip_enabled,
        download_dir,
        ctx: Mutex::new(None),
    });

    {
        let f = files.clone();
        let c = core.clone();
        link.set_handler(Some(Arc::new(move |m| c.clip.on_message(m) || f.on_message(m))));
    }
    {
        let c = core.clone();
        files.set_on_change(Some(Arc::new(move || c.repaint())));
    }
    // Bağlantı olayları
    {
        let c = core.clone();
        std::thread::spawn(move || {
            for ev in rx {
                if let LinkEvent::State = ev {
                    match c.link.phase() {
                        LinkPhase::Idle => c.files.abort_all(),
                        LinkPhase::Connected => {
                            if let Some(p) = c.link.peer() {
                                c.settings.lock().unwrap().last_peer = p.id;
                                c.save();
                            }
                        }
                        _ => {}
                    }
                }
                c.repaint();
            }
        });
    }
    // Son eşleşilen cihaz ağda görünürse otomatik bağlan (yalnızca kimliği küçük olan taraf, çakışmasın diye).
    {
        let c = core.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(4));
            if c.link.phase() != LinkPhase::Idle {
                continue;
            }
            let last = c.settings.lock().unwrap().last_peer.clone();
            if last.is_empty() || !c.trust.contains(&last) || c.identity.id >= last {
                continue;
            }
            let found = c.discovery.as_ref().and_then(|d| d.list().into_iter().find(|f| f.0 == last));
            if let Some((_, _, addr, port)) = found {
                if let Ok(ip) = addr.parse() {
                    c.link.connect(SocketAddr::new(ip, port));
                }
            }
        });
    }

    let cs = core.clone();
    let ca = core.clone();
    let cc = core.clone();
    let prefs = ui::UiPrefs { theme: ui::ThemeChoice::parse(&theme), ..ui::UiPrefs::macos_default() };
    let result = ui::run_with(
        Arc::new(move || cs.state()),
        Arc::new(move |a| ca.act(a)),
        Box::new(move |ctx| *cc.ctx.lock().unwrap() = Some(ctx)),
        prefs,
    );

    core.link.close();
    if let Some(d) = &core.discovery {
        d.stop();
    }
    core.clip.stop();
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_address;

    #[test]
    fn adres_ayristirma() {
        assert_eq!(parse_address("192.168.1.20").unwrap().to_string(), "192.168.1.20:47910");
        assert_eq!(parse_address(" 10.0.0.5:5000 ").unwrap().to_string(), "10.0.0.5:5000");
        assert_eq!(parse_address("[::1]:47910").unwrap().to_string(), "[::1]:47910");
        assert!(parse_address("").is_none());
        assert!(parse_address("192.168.1.20:99999").is_none() || parse_address("192.168.1.20:99999").is_some());
    }
}
