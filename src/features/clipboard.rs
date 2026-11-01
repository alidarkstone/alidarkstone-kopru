//! Pano paylaşımı (yalnızca metin). Referans: `reference/electron/src/features/clipboard.js`.
//! Varsayılan olarak KAPALI; kullanıcı açar. Pano ~600 ms'de bir okunur, değiştiyse eşe gönderilir.
//! Eşten gelen metin panoya yazılır ve hatırlanır; böylece geri yankılanmaz.
//!
//! Tüm pano erişimi tek bir iş parçacığında yapılır: gelen metnin yazılması ile okuma sırası
//! karışmaz (yankı bastırma yarışsız), Linux'ta pano sahipliği o iş parçacığında yaşar.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::Sender;
use crate::msg::Msg;

pub const POLL: Duration = Duration::from_millis(600);
/// 1 MB metin üst sınırı (UTF-8 bayt; referanstaki UTF-16 sınırından hiçbir zaman gevşek değil).
pub const MAX_TEXT: usize = 1024 * 1024;

/// Pano erişimi (testlerde sahte pano kullanılır).
pub trait ClipboardBackend: Send {
    fn read_text(&mut self) -> Option<String>;
    fn write_text(&mut self, text: &str);
}

/// Sistem panosu (arboard). Açılamazsa sonraki denemede yeniden dener.
#[derive(Default)]
pub struct SystemClipboard {
    cb: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn get(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.cb.is_none() {
            self.cb = arboard::Clipboard::new().ok();
        }
        self.cb.as_mut()
    }
}

impl ClipboardBackend for SystemClipboard {
    fn read_text(&mut self) -> Option<String> {
        self.get()?.get_text().ok()
    }
    fn write_text(&mut self, text: &str) {
        if let Some(cb) = self.get() {
            if cb.set_text(text.to_owned()).is_err() {
                self.cb = None;
            }
        }
    }
}

pub struct ClipboardShare {
    enabled: Arc<AtomicBool>,
    /// Yazılmayı bekleyen en son gelen metin (yalnızca sonuncusu tutulur: sel birikmez).
    pending: Arc<Mutex<Option<String>>>,
    wake: mpsc::SyncSender<()>,
    stop: Arc<AtomicBool>,
}

impl ClipboardShare {
    /// Sistem panosuyla başlatır. `enabled` false iken hiçbir şey gönderilmez veya yazılmaz.
    pub fn start(send: Sender, enabled: Arc<AtomicBool>) -> Self {
        Self::start_with(Box::new(SystemClipboard::default()), send, enabled, POLL)
    }

    pub fn start_with(mut backend: Box<dyn ClipboardBackend>, send: Sender, enabled: Arc<AtomicBool>, poll: Duration) -> Self {
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        let pending = Arc::new(Mutex::new(None::<String>));
        let pend = pending.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let (en, st) = (enabled.clone(), stop.clone());
        let _ = thread::Builder::new().name("darkdrop-clipboard".into()).spawn(move || {
            // Başlangıçtaki içerik gönderilmez.
            let mut last = backend.read_text().unwrap_or_default();
            loop {
                match rx.recv_timeout(poll) {
                    Ok(()) => {
                        if let Some(text) = pend.lock().unwrap_or_else(|e| e.into_inner()).take() {
                            last = text;
                            backend.write_text(&last);
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if st.load(Ordering::SeqCst) {
                    return;
                }
                let now = backend.read_text().unwrap_or_default();
                if now == last {
                    continue;
                }
                last = now;
                if !en.load(Ordering::SeqCst) || last.is_empty() || last.len() > MAX_TEXT {
                    continue;
                }
                send(&Msg::ClipText { text: last.clone() });
            }
        });
        ClipboardShare { enabled, pending, wake: tx, stop }
    }

    /// `clip.text` işler (kapalıyken false döner, referanstaki gibi).
    pub fn on_message(&self, m: &Msg) -> bool {
        let Msg::ClipText { text } = m else { return false };
        if !self.enabled.load(Ordering::SeqCst) {
            return false;
        }
        if text.len() > MAX_TEXT {
            return true;
        }
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.clone());
        let _ = self.wake.try_send(()); // dolu: zaten bir uyandırma bekliyor, en son metni o alır
        true
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for ClipboardShare {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<String>>, Arc<Mutex<usize>>);
    impl ClipboardBackend for Fake {
        fn read_text(&mut self) -> Option<String> {
            Some(self.0.lock().unwrap().clone())
        }
        fn write_text(&mut self, t: &str) {
            *self.0.lock().unwrap() = t.into();
            *self.1.lock().unwrap() += 1;
        }
    }

    fn setup(initial: &str, on: bool) -> (ClipboardShare, Fake, Arc<Mutex<Vec<String>>>, Arc<AtomicBool>) {
        let fake = Fake::default();
        *fake.0.lock().unwrap() = initial.into();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let s2 = sent.clone();
        let en = Arc::new(AtomicBool::new(on));
        let c = ClipboardShare::start_with(
            Box::new(fake.clone()),
            Arc::new(move |m: &Msg| {
                if let Msg::ClipText { text } = m {
                    s2.lock().unwrap().push(text.clone());
                }
                true
            }),
            en.clone(),
            Duration::from_millis(20),
        );
        (c, fake, sent, en)
    }

    fn settle() {
        thread::sleep(Duration::from_millis(120));
    }

    #[test]
    fn sends_changes_not_initial() {
        let (_c, fake, sent, _) = setup("ilk", true);
        settle();
        assert!(sent.lock().unwrap().is_empty());
        *fake.0.lock().unwrap() = "yeni ğ".into();
        settle();
        assert_eq!(*sent.lock().unwrap(), vec!["yeni ğ".to_string()]);
        settle();
        assert_eq!(sent.lock().unwrap().len(), 1, "değişmeyen içerik tekrar gönderilmez");
        *fake.0.lock().unwrap() = String::new();
        settle();
        assert_eq!(sent.lock().unwrap().len(), 1, "boş metin gönderilmez");
    }

    #[test]
    fn off_by_default_semantics() {
        let (c, fake, sent, en) = setup("", false);
        *fake.0.lock().unwrap() = "gizli".into();
        settle();
        assert!(sent.lock().unwrap().is_empty());
        assert!(!c.on_message(&Msg::ClipText { text: "dışarıdan".into() }));
        settle();
        assert_eq!(*fake.0.lock().unwrap(), "gizli");
        en.store(true, Ordering::SeqCst);
        settle();
        assert!(sent.lock().unwrap().is_empty(), "açılınca eski içerik gönderilmez");
    }

    #[test]
    fn echo_suppressed() {
        let (c, fake, sent, _) = setup("", true);
        let t = Instant::now();
        for i in 0..20 {
            assert!(c.on_message(&Msg::ClipText { text: format!("uzak {i}") }));
            thread::sleep(Duration::from_millis(7));
        }
        settle();
        assert_eq!(*fake.0.lock().unwrap(), "uzak 19");
        assert!(sent.lock().unwrap().is_empty(), "gelen metin geri gönderilmemeli: {:?}", sent.lock().unwrap());
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    /// Pano yazımı yavaşken gelen sel birikmez: yalnızca en son metin yazılır (bellek sınırlı).
    #[test]
    fn incoming_flood_coalesces() {
        struct Slow(Arc<Mutex<Vec<String>>>);
        impl ClipboardBackend for Slow {
            fn read_text(&mut self) -> Option<String> {
                self.0.lock().unwrap().last().cloned()
            }
            fn write_text(&mut self, t: &str) {
                thread::sleep(Duration::from_millis(50));
                self.0.lock().unwrap().push(t.into());
            }
        }
        let writes = Arc::new(Mutex::new(Vec::new()));
        let c = ClipboardShare::start_with(
            Box::new(Slow(writes.clone())),
            Arc::new(|_: &Msg| true),
            Arc::new(AtomicBool::new(true)),
            Duration::from_millis(20),
        );
        for i in 0..200 {
            assert!(c.on_message(&Msg::ClipText { text: format!("uzak {i}") }));
        }
        thread::sleep(Duration::from_millis(400));
        let w = writes.lock().unwrap();
        assert!(w.len() <= 4, "{} yazım: sel kuyrukta birikiyor", w.len());
        assert_eq!(w.last().map(String::as_str), Some("uzak 199"));
    }

    #[test]
    fn size_cap() {
        let (c, fake, sent, _) = setup("", true);
        let big = "a".repeat(MAX_TEXT + 1);
        assert!(c.on_message(&Msg::ClipText { text: big.clone() }));
        settle();
        assert_eq!(*fake.1.lock().unwrap(), 0);
        *fake.0.lock().unwrap() = big;
        settle();
        assert!(sent.lock().unwrap().is_empty());
        assert!(!c.on_message(&Msg::Ping));
    }
}
