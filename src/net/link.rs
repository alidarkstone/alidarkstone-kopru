//! Bağlantı yöneticisi: tek bir eş cihazla oturum.
//! Referans: `reference/electron/src/core/link.js`.
//!
//! - Gelen bağlantıları dinler (TCP), istenirse bir cihaza bağlanır. Aynı anda yalnızca bir oturum.
//! - El sıkışmadan sonra iki taraf `trust {known}` gönderir. İkisi de birbirini tanıyorsa doğrudan
//!   bağlanır; biri bile tanımıyorsa ikisi de 6 haneli kod onayına geçer. İKİ taraf da onaylarsa
//!   karşı taraf güvenilenlere eklenir.
//! - Eşleşme tamamlanmadan hiçbir özellik mesajı işlenmez.
//!
//! İş parçacıkları (oturum başına): okuyucu (el sıkışmayı da yapar), yazıcı (şifreler ve yazar),
//! zamanlayıcı (ping / ölü bağlantı / eşleştirme zaman aşımı). Async çalışma zamanı yok.

use std::collections::VecDeque;
use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::channel::{handshake, PeerInfo, Role, Sealer};
use super::framing::{read_frame, write_frame};
use super::keys::Identity;
use super::trust::TrustStore;
use crate::msg::Msg;
use crate::state::{LinkPhase, PeerView};

pub const DEFAULT_PORT: u16 = 47910;
pub const PING_EVERY: Duration = Duration::from_secs(5);
pub const DEAD_AFTER: Duration = Duration::from_secs(20);
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(90);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const WRITE_TIMEOUT: Duration = Duration::from_secs(20);
/// Gelen iki oturum başlangıcı arasındaki en kısa süre. Ortadaki birinin sunucuya arka arkaya
/// görünmeden bağlanıp 6 haneli kodu tutturana kadar denemesini yavaşlatır (90 sn'de en çok ~90 deneme).
pub const INCOMING_GAP: Duration = Duration::from_secs(1);
/// Yazma kuyruğunda bu kadar bayt birikince `send` (okuyucu iş parçacığı dışından çağrıldıysa) bekler.
const HIGH_WATER: usize = 4 * 1024 * 1024;
const LOW_WATER: usize = 1024 * 1024;

pub enum LinkEvent {
    /// Aşama / eş / hata değişti: `phase()`, `peer()`, `last_error()` yeniden okunmalı.
    State,
    /// Bağlıyken gelen özellik mesajı (ping/trust/pair.* ve bilinmeyen türler buraya gelmez).
    /// `set_handler` ile kurulan işleyici mesajı tükettiyse bu olay gönderilmez.
    Message(Msg),
    /// Dolan yazma kuyruğu boşaldı.
    Drain,
}

/// Okuyucu iş parçacığında, sırayla çağrılır. `true` => mesaj tüketildi.
pub type MsgHandler = Arc<dyn Fn(&Msg) -> bool + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    Connecting,
    Checking,
    Pairing,
    Connected,
}

struct Pair {
    sas: String,
    mine: Option<bool>,
    theirs: Option<bool>,
    deadline: Instant,
}

struct St {
    state: State,
    session: Option<Arc<Session>>,
    peer: Option<PeerInfo>,
    pair: Option<Pair>,
    known: bool,
    /// `Checking` aşamasının bitmesi gereken an (karşı taraf `trust` göndermezse).
    check_deadline: Option<Instant>,
    last_error: String,
    port: Option<u16>,
}

#[derive(Clone, Copy)]
struct Timings {
    ping_every: Duration,
    dead_after: Duration,
    pair_timeout: Duration,
}

struct Inner {
    identity: Arc<Identity>,
    name: Arc<RwLock<String>>,
    trust: Arc<dyn TrustStore>,
    events: Mutex<Sender<LinkEvent>>,
    handler: RwLock<Option<MsgHandler>>,
    st: Mutex<St>,
    closed: AtomicBool,
    next_id: AtomicU64,
    timings: Mutex<Timings>,
}

struct OutQ {
    q: VecDeque<Vec<u8>>,
    bytes: usize,
    open: bool,
    closing: bool,
    writer_started: bool,
    blocked: bool,
}

struct Session {
    id: u64,
    stream: Mutex<Option<TcpStream>>,
    closed: AtomicBool,
    out: Mutex<OutQ>,
    cv: Condvar,
    last_seen: Mutex<Instant>,
    reader: OnceLock<ThreadId>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn encode(m: &Msg) -> Option<Vec<u8>> {
    serde_json::to_vec(m).ok()
}

impl Session {
    fn new(id: u64) -> Arc<Self> {
        Arc::new(Session {
            id,
            stream: Mutex::new(None),
            closed: AtomicBool::new(false),
            out: Mutex::new(OutQ { q: VecDeque::new(), bytes: 0, open: true, closing: false, writer_started: false, blocked: false }),
            cv: Condvar::new(),
            last_seen: Mutex::new(Instant::now()),
            reader: OnceLock::new(),
        })
    }

    /// Kapatılmışsa akışı hemen kapatır ve false döner.
    fn set_stream(&self, s: TcpStream) -> bool {
        let mut g = lock(&self.stream);
        if self.closed.load(Ordering::SeqCst) {
            let _ = s.shutdown(Shutdown::Both);
            return false;
        }
        *g = Some(s);
        true
    }

    fn shutdown(&self) {
        if let Some(s) = lock(&self.stream).as_ref() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }

    /// Düz metin JSON'u yazma kuyruğuna koyar. `may_block` ise ve kuyruk doluysa yer açılana kadar
    /// bekler (okuyucu iş parçacığı asla beklemez: iki tarafın da okumayı bırakıp kilitlenmesini önler).
    fn enqueue(&self, plain: Vec<u8>, may_block: bool) -> bool {
        let is_reader = self.reader.get() == Some(&thread::current().id());
        let mut g = lock(&self.out);
        if may_block && !is_reader {
            while g.open && g.bytes > HIGH_WATER {
                g.blocked = true;
                g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        }
        if !g.open {
            return false;
        }
        g.bytes += plain.len();
        g.q.push_back(plain);
        self.cv.notify_all();
        true
    }

    /// Oturumu kapatır. Bekleyen veriler atılır; `last` verilirse yazıcı onu gönderip sonra kapatır.
    fn close(&self, last: Option<Vec<u8>>) {
        self.closed.store(true, Ordering::SeqCst);
        let flush = {
            let mut g = lock(&self.out);
            g.q.clear();
            g.bytes = 0;
            g.open = false;
            g.closing = true;
            let flush = g.writer_started && last.is_some();
            if let Some(m) = last.filter(|_| flush) {
                g.q.push_back(m);
            }
            self.cv.notify_all();
            flush
        };
        if !flush {
            self.shutdown();
        }
    }
}

/// Tek eşle oturum yöneticisi. Ucuz kopyalanır (içi `Arc`).
#[derive(Clone)]
pub struct Link {
    inner: Arc<Inner>,
}

impl Link {
    pub fn new(
        identity: Arc<Identity>,
        name: Arc<RwLock<String>>,
        trust: Arc<dyn TrustStore>,
        events: Sender<LinkEvent>,
    ) -> Self {
        Link {
            inner: Arc::new(Inner {
                identity,
                name,
                trust,
                events: Mutex::new(events),
                handler: RwLock::new(None),
                st: Mutex::new(St {
                    state: State::Idle,
                    session: None,
                    peer: None,
                    pair: None,
                    known: false,
                    check_deadline: None,
                    last_error: String::new(),
                    port: None,
                }),
                closed: AtomicBool::new(false),
                next_id: AtomicU64::new(1),
                timings: Mutex::new(Timings { ping_every: PING_EVERY, dead_after: DEAD_AFTER, pair_timeout: PAIR_TIMEOUT }),
            }),
        }
    }

    /// Bağlıyken gelen özellik mesajlarını okuyucu iş parçacığında işleyecek fonksiyon.
    /// Böylece dosya parçaları diske yazılana kadar soketten yeni veri okunmaz (alıcı tarafta geri basınç).
    pub fn set_handler(&self, h: Option<MsgHandler>) {
        *self.inner.handler.write().unwrap_or_else(|e| e.into_inner()) = h;
    }

    /// 0.0.0.0:`port` üzerinde dinler (0 => rastgele). Gerçek portu döner.
    pub fn listen(&self, port: u16) -> io::Result<u16> {
        let l = TcpListener::bind(("0.0.0.0", port))?;
        let port = l.local_addr()?.port();
        l.set_nonblocking(true)?;
        lock(&self.inner.st).port = Some(port);
        let inner = self.inner.clone();
        thread::Builder::new().name("darkdrop-accept".into()).spawn(move || {
            let mut last_start: Option<Instant> = None;
            while !inner.closed.load(Ordering::SeqCst) {
                if let Some(t) = last_start {
                    // Bekleyen bağlantılar çekirdeğin kuyruğunda bekler.
                    let left = INCOMING_GAP.saturating_sub(t.elapsed());
                    if !left.is_zero() {
                        thread::sleep(left.min(Duration::from_millis(50)));
                        continue;
                    }
                }
                match l.accept() {
                    Ok((s, _)) => {
                        if inner.incoming(s) {
                            last_start = Some(Instant::now());
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(50)),
                    Err(_) => thread::sleep(Duration::from_millis(100)),
                }
            }
        })?;
        Ok(port)
    }

    /// Dinlenen port (listen çağrılmadıysa None).
    pub fn port(&self) -> Option<u16> {
        lock(&self.inner.st).port
    }

    /// Varsa mevcut oturumu kapatıp yeni bağlantı başlatır (arka planda).
    pub fn connect(&self, addr: SocketAddr) {
        let inner = &self.inner;
        if inner.closed.load(Ordering::SeqCst) {
            return;
        }
        let sess = Session::new(inner.next_id.fetch_add(1, Ordering::SeqCst));
        {
            let mut st = lock(&inner.st);
            inner.detach_locked(&mut st, None, None);
            st.session = Some(sess.clone());
            st.state = State::Connecting;
            st.last_error.clear();
        }
        inner.emit(LinkEvent::State);
        let inner2 = inner.clone();
        let r = thread::Builder::new()
            .name("darkdrop-session".into())
            .spawn(move || inner2.run_session(sess, None, Some(addr), Role::Client));
        if r.is_err() {
            inner.close_session_any("iş parçacığı başlatılamadı");
        }
    }

    pub fn disconnect(&self) {
        let emit = {
            let mut st = lock(&self.inner.st);
            self.inner.detach_locked(&mut st, None, None)
        };
        if emit {
            self.inner.emit(LinkEvent::State);
        }
    }

    /// Kullanıcı ekrandaki kodu onayladı (true) veya reddetti (false).
    pub fn confirm_pairing(&self, ok: bool) {
        let inner = &self.inner;
        let mut st = lock(&inner.st);
        if st.state != State::Pairing {
            return;
        }
        let Some(pair) = st.pair.as_mut() else { return };
        if pair.mine.is_some() {
            return;
        }
        pair.mine = Some(ok);
        let reply = encode(if ok { &Msg::PairOk } else { &Msg::PairNo });
        if !ok {
            inner.detach_locked(&mut st, Some("eşleştirme reddedildi".into()), reply);
            drop(st);
            inner.emit(LinkEvent::State);
            return;
        }
        if let (Some(s), Some(r)) = (st.session.clone(), reply) {
            s.enqueue(r, false);
        }
        inner.check_paired(&mut st);
        drop(st);
        inner.emit(LinkEvent::State);
    }

    /// Bağlıysa şifreli mesaj gönderir. Kuyruk doluysa yer açılana kadar bekler (geri basınç).
    /// Bağlı değilse veya mesaj çok büyükse false.
    pub fn send(&self, m: &Msg) -> bool {
        let sess = {
            let st = lock(&self.inner.st);
            if st.state != State::Connected {
                return false;
            }
            match st.session.clone() {
                Some(s) => s,
                None => return false,
            }
        };
        match encode(m) {
            Some(b) if b.len() + 16 <= super::framing::MAX_FRAME => sess.enqueue(b, true),
            _ => false,
        }
    }

    pub fn phase(&self) -> LinkPhase {
        let st = lock(&self.inner.st);
        match st.state {
            State::Idle => LinkPhase::Idle,
            State::Connecting => LinkPhase::Connecting,
            State::Checking => LinkPhase::Checking,
            State::Connected => LinkPhase::Connected,
            State::Pairing => match &st.pair {
                Some(p) => LinkPhase::Pairing { sas: p.sas.clone(), mine: p.mine, theirs: p.theirs },
                None => LinkPhase::Checking,
            },
        }
    }

    pub fn is_connected(&self) -> bool {
        lock(&self.inner.st).state == State::Connected
    }

    pub fn peer(&self) -> Option<PeerView> {
        let st = lock(&self.inner.st);
        st.peer.as_ref().map(|p| PeerView { id: p.peer_id.clone(), name: p.peer_name.clone(), address: p.address.clone() })
    }

    pub fn last_error(&self) -> String {
        lock(&self.inner.st).last_error.clone()
    }

    /// Oturumu kapatır ve dinlemeyi bırakır. Sonrasında Link kullanılamaz.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.disconnect();
    }

    #[cfg(test)]
    pub(crate) fn set_timings(&self, ping_every: Duration, dead_after: Duration, pair_timeout: Duration) {
        *lock(&self.inner.timings) = Timings { ping_every, dead_after, pair_timeout };
    }
}

impl Inner {
    fn emit(&self, e: LinkEvent) {
        let _ = lock(&self.events).send(e);
    }

    fn is_current(st: &St, sess: &Session) -> bool {
        st.session.as_ref().map(|s| s.id) == Some(sess.id)
    }

    /// Mevcut oturumu ayırır ve kapatır. Olay gönderilmesi gerekiyorsa true.
    fn detach_locked(&self, st: &mut St, reason: Option<String>, last: Option<Vec<u8>>) -> bool {
        let Some(s) = st.session.take() else { return false };
        s.close(last);
        st.peer = None;
        st.pair = None;
        st.known = false;
        st.check_deadline = None;
        st.last_error = reason.unwrap_or_default();
        st.state = State::Idle;
        true
    }

    /// Yalnızca `sess` hâlâ mevcut oturumsa kapatır.
    fn close_session(&self, sess: &Session, reason: Option<String>) {
        let emit = {
            let mut st = lock(&self.st);
            if Self::is_current(&st, sess) {
                self.detach_locked(&mut st, reason, None)
            } else {
                false
            }
        };
        sess.close(None);
        if emit {
            self.emit(LinkEvent::State);
        }
    }

    fn close_session_any(&self, reason: &str) {
        let emit = {
            let mut st = lock(&self.st);
            self.detach_locked(&mut st, Some(reason.into()), None)
        };
        if emit {
            self.emit(LinkEvent::State);
        }
    }

    /// Oturum başlatıldıysa true.
    fn incoming(self: &Arc<Self>, s: TcpStream) -> bool {
        let _ = s.set_nonblocking(false);
        let sess = {
            let mut st = lock(&self.st);
            if self.closed.load(Ordering::SeqCst) || st.session.is_some() {
                // zaten bir oturum var
                let _ = s.shutdown(Shutdown::Both);
                return false;
            }
            let sess = Session::new(self.next_id.fetch_add(1, Ordering::SeqCst));
            st.session = Some(sess.clone());
            sess
        };
        let inner = self.clone();
        let sess2 = sess.clone();
        let r = thread::Builder::new()
            .name("darkdrop-session".into())
            .spawn(move || inner.run_session(sess2, Some(s), None, Role::Server));
        if r.is_err() {
            self.close_session(&sess, Some("iş parçacığı başlatılamadı".into()));
        }
        true
    }

    fn run_session(self: Arc<Self>, sess: Arc<Session>, stream: Option<TcpStream>, addr: Option<SocketAddr>, role: Role) {
        let stream = match (stream, addr) {
            (Some(s), _) => s,
            (None, Some(a)) => match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
                Ok(s) => s,
                Err(e) => return self.close_session(&sess, Some(format!("bağlanılamadı: {e}"))),
            },
            _ => return,
        };
        let _ = stream.set_nodelay(true);
        match stream.try_clone() {
            Ok(c) => {
                if !sess.set_stream(c) {
                    return;
                }
            }
            Err(e) => return self.close_session(&sess, Some(e.to_string())),
        }
        let name = self.name.read().map(|n| n.clone()).unwrap_or_default();
        let (info, sealer, mut opener) = match handshake(&stream, &self.identity, &name, role) {
            Ok(x) => x,
            Err(e) => return self.close_session(&sess, Some(e)),
        };
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let _ = sess.reader.set(thread::current().id());

        // Yazıcı
        let wstream = match stream.try_clone() {
            Ok(w) => w,
            Err(e) => return self.close_session(&sess, Some(e.to_string())),
        };
        lock(&sess.out).writer_started = true;
        {
            let inner = self.clone();
            let s2 = sess.clone();
            if thread::Builder::new()
                .name("darkdrop-writer".into())
                .spawn(move || inner.writer(s2, wstream, sealer))
                .is_err()
            {
                lock(&sess.out).writer_started = false;
                return self.close_session(&sess, Some("iş parçacığı başlatılamadı".into()));
            }
        }

        // Hazır: güven bilgisini paylaş.
        {
            let mut st = lock(&self.st);
            if !Self::is_current(&st, &sess) {
                drop(st);
                sess.close(None);
                return;
            }
            st.known = self.trust.is_trusted(&info.peer_id, &info.peer_raw);
            st.peer = Some(info);
            *lock(&sess.last_seen) = Instant::now();
            if let Some(b) = encode(&Msg::Trust { known: st.known }) {
                sess.enqueue(b, false);
            }
            st.state = State::Checking;
            st.check_deadline = Some(Instant::now() + lock(&self.timings).pair_timeout);
        }
        self.emit(LinkEvent::State);

        // Zamanlayıcı
        {
            let inner = self.clone();
            let s2 = sess.clone();
            if thread::Builder::new().name("darkdrop-timer".into()).spawn(move || inner.ticker(s2)).is_err() {
                return self.close_session(&sess, Some("iş parçacığı başlatılamadı".into()));
            }
        }

        // Okuyucu
        let mut rd = &stream;
        loop {
            let frame = match read_frame(&mut rd) {
                Ok(f) => f,
                Err(e) => {
                    let reason = if sess.closed.load(Ordering::SeqCst) || e.kind() == io::ErrorKind::UnexpectedEof {
                        None
                    } else {
                        Some(e.to_string())
                    };
                    return self.close_session(&sess, reason);
                }
            };
            let plain = match opener.open(&frame) {
                Ok(p) => p,
                Err(e) => return self.close_session(&sess, Some(e)),
            };
            let msg = match serde_json::from_slice::<Value>(&plain) {
                Ok(v) if v.get("t").map(Value::is_string).unwrap_or(false) => {
                    // Bilinen türde ama bozuk alanlı mesajlar yok sayılır (referans da onları özelliklere bırakır).
                    serde_json::from_value::<Msg>(v).unwrap_or(Msg::Unknown)
                }
                _ => return self.close_session(&sess, Some("geçersiz mesaj".into())),
            };
            self.on_message(&sess, msg);
        }
    }

    fn writer(self: Arc<Self>, sess: Arc<Session>, stream: TcpStream, mut sealer: Sealer) {
        let mut w = &stream;
        loop {
            let (item, drained) = {
                let mut g = lock(&sess.out);
                loop {
                    if let Some(item) = g.q.pop_front() {
                        g.bytes = g.bytes.saturating_sub(item.len());
                        let drained = g.blocked && g.bytes <= LOW_WATER;
                        if drained {
                            g.blocked = false;
                        }
                        sess.cv.notify_all();
                        break (Some(item), drained);
                    }
                    if g.closing {
                        break (None, false);
                    }
                    g = sess.cv.wait(g).unwrap_or_else(|e| e.into_inner());
                }
            };
            if drained {
                self.emit(LinkEvent::Drain);
            }
            let Some(item) = item else { break };
            let res = sealer.seal(&item).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)).and_then(|ct| write_frame(&mut w, &ct));
            if let Err(e) = res {
                self.close_session(&sess, Some(e.to_string()));
                break;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
    }

    fn ticker(self: Arc<Self>, sess: Arc<Session>) {
        let mut last_ping = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(100));
            if sess.closed.load(Ordering::SeqCst) {
                return;
            }
            let t = *lock(&self.timings);
            let now = Instant::now();
            let mut st = lock(&self.st);
            if !Self::is_current(&st, &sess) {
                return;
            }
            let expired = match st.state {
                State::Pairing => st.pair.as_ref().map(|p| now >= p.deadline).unwrap_or(false),
                State::Checking => st.check_deadline.map(|d| now >= d).unwrap_or(false),
                _ => false,
            };
            if expired {
                self.detach_locked(&mut st, Some("eşleştirme zaman aşımı".into()), None);
                drop(st);
                return self.emit(LinkEvent::State);
            }
            if now.duration_since(last_ping) >= t.ping_every {
                last_ping = now;
                let seen = *lock(&sess.last_seen);
                if now.duration_since(seen) > t.dead_after {
                    self.detach_locked(&mut st, Some("bağlantı koptu".into()), None);
                    drop(st);
                    return self.emit(LinkEvent::State);
                }
                if let Some(b) = encode(&Msg::Ping) {
                    sess.enqueue(b, false);
                }
            }
        }
    }

    fn decide(&self, st: &mut St, they_know: bool) {
        if st.known && they_know {
            st.state = State::Connected;
            return;
        }
        let sas = st.peer.as_ref().map(|p| p.sas.clone()).unwrap_or_default();
        let deadline = Instant::now() + lock(&self.timings).pair_timeout;
        st.pair = Some(Pair { sas, mine: None, theirs: None, deadline });
        st.state = State::Pairing;
    }

    fn check_paired(&self, st: &mut St) {
        let done = matches!(&st.pair, Some(p) if p.mine == Some(true) && p.theirs == Some(true));
        if done {
            if let Some(p) = &st.peer {
                self.trust.add(&p.peer_id, &p.peer_raw, &p.peer_name);
            }
            st.pair = None;
            st.state = State::Connected;
        }
    }

    fn on_message(&self, sess: &Arc<Session>, msg: Msg) {
        *lock(&sess.last_seen) = Instant::now();
        if msg == Msg::Ping {
            return;
        }
        {
            let mut st = lock(&self.st);
            if !Self::is_current(&st, sess) {
                return;
            }
            match st.state {
                State::Checking => {
                    match msg {
                        Msg::Trust { known } => self.decide(&mut st, known),
                        _ => {
                            self.detach_locked(&mut st, Some("beklenmeyen mesaj".into()), None);
                        }
                    }
                    drop(st);
                    return self.emit(LinkEvent::State);
                }
                State::Pairing => {
                    match msg {
                        Msg::PairOk => {
                            if let Some(p) = st.pair.as_mut() {
                                p.theirs = Some(true);
                            }
                            self.check_paired(&mut st);
                        }
                        Msg::PairNo => {
                            self.detach_locked(&mut st, Some("karşı taraf eşleştirmeyi reddetti".into()), None);
                        }
                        // eşleşme bitmeden başka mesaj kabul edilmez
                        _ => return,
                    }
                    drop(st);
                    return self.emit(LinkEvent::State);
                }
                State::Connected => {}
                _ => return,
            }
        }
        if matches!(msg, Msg::Trust { .. } | Msg::PairOk | Msg::PairNo | Msg::Unknown) {
            return;
        }
        let handler = self.handler.read().ok().and_then(|h| h.clone());
        if let Some(h) = handler {
            if h(&msg) {
                return;
            }
        }
        self.emit(LinkEvent::Message(msg));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::channel::{handshake as raw_handshake, tests::tcp_pair};
    use super::super::trust::MemoryTrustStore;
    use super::*;
    use std::io::{Read, Write};
    use std::sync::mpsc::{channel, Receiver};

    pub(crate) struct Node {
        pub link: Link,
        pub rx: Receiver<LinkEvent>,
        pub trust: Arc<MemoryTrustStore>,
        pub id: Arc<Identity>,
    }

    pub(crate) fn node(name: &str) -> Node {
        let (tx, rx) = channel();
        let trust = Arc::new(MemoryTrustStore::new());
        let id = Arc::new(Identity::generate());
        let link = Link::new(id.clone(), Arc::new(RwLock::new(name.into())), trust.clone(), tx);
        Node { link, rx, trust, id }
    }

    pub(crate) fn wait<F: Fn() -> bool>(what: &str, f: F) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(10), "zaman aşımı: {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn local(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn is_pairing(l: &Link) -> bool {
        matches!(l.phase(), LinkPhase::Pairing { .. })
    }

    fn sas(l: &Link) -> String {
        match l.phase() {
            LinkPhase::Pairing { sas, .. } => sas,
            p => panic!("eşleştirmede değil: {p:?}"),
        }
    }

    fn next_msg(rx: &Receiver<LinkEvent>) -> Msg {
        let t = Instant::now();
        loop {
            let left = Duration::from_secs(10).saturating_sub(t.elapsed());
            match rx.recv_timeout(left).expect("mesaj gelmedi") {
                LinkEvent::Message(m) => return m,
                _ => continue,
            }
        }
    }

    /// A dinler, B bağlanır, iki taraf da onaylar.
    pub(crate) fn paired() -> (Node, Node, u16) {
        let a = node("A");
        let b = node("B");
        let port = a.link.listen(0).unwrap();
        b.link.connect(local(port));
        wait("eşleştirme", || is_pairing(&a.link) && is_pairing(&b.link));
        assert_eq!(sas(&a.link), sas(&b.link));
        a.link.confirm_pairing(true);
        b.link.confirm_pairing(true);
        wait("bağlandı", || a.link.is_connected() && b.link.is_connected());
        (a, b, port)
    }

    #[test]
    fn pairing_both_confirm_then_trusted_reconnect() {
        let a = node("Masaüstü");
        let b = node("Dizüstü");
        let port = a.link.listen(0).unwrap();
        assert!(!b.link.send(&Msg::Ping), "bağlı değilken send false");
        b.link.connect(local(port));
        wait("eşleştirme", || is_pairing(&a.link) && is_pairing(&b.link));
        assert_eq!(sas(&a.link), sas(&b.link));
        assert!(!a.link.send(&Msg::ClipText { text: "x".into() }), "eşleşmeden mesaj yok");
        assert_eq!(a.link.peer().unwrap().name, "Dizüstü");
        assert_eq!(b.link.peer().unwrap().id, a.id.id);
        assert_eq!(a.link.peer().unwrap().address, "127.0.0.1");

        a.link.confirm_pairing(true);
        wait("karşı onay görünür", || matches!(b.link.phase(), LinkPhase::Pairing { theirs: Some(true), mine: None, .. }));
        assert!(matches!(a.link.phase(), LinkPhase::Pairing { mine: Some(true), theirs: None, .. }));
        assert!(a.trust.list().is_empty(), "tek taraflı onayla güvenilmez");
        b.link.confirm_pairing(true);
        wait("bağlandı", || a.link.is_connected() && b.link.is_connected());
        assert!(a.trust.is_trusted(&b.id.id, &b.id.raw_b64()));
        assert!(b.trust.is_trusted(&a.id.id, &a.id.raw_b64()));

        assert!(b.link.send(&Msg::ClipText { text: "merhaba ğüşİ".into() }));
        assert_eq!(next_msg(&a.rx), Msg::ClipText { text: "merhaba ğüşİ".into() });
        assert!(a.link.send(&Msg::FileCancel { id: "0123456789abcdef".into() }));
        assert_eq!(next_msg(&b.rx), Msg::FileCancel { id: "0123456789abcdef".into() });

        b.link.disconnect();
        wait("ayrıldı", || a.link.phase() == LinkPhase::Idle && b.link.phase() == LinkPhase::Idle);
        assert_eq!(b.link.last_error(), "");
        // Tanıdık cihazlar: kod sorulmadan bağlanır.
        b.link.connect(local(port));
        wait("doğrudan bağlandı", || a.link.is_connected() && b.link.is_connected());
        a.link.close();
        b.link.close();
    }

    #[test]
    fn pairing_reject() {
        let a = node("A");
        let b = node("B");
        let port = a.link.listen(0).unwrap();
        b.link.connect(local(port));
        wait("eşleştirme", || is_pairing(&a.link) && is_pairing(&b.link));
        b.link.confirm_pairing(false);
        wait("kapandı", || a.link.phase() == LinkPhase::Idle && b.link.phase() == LinkPhase::Idle);
        assert_eq!(b.link.last_error(), "eşleştirme reddedildi");
        assert_eq!(a.link.last_error(), "karşı taraf eşleştirmeyi reddetti");
        assert!(a.trust.list().is_empty() && b.trust.list().is_empty());
        a.link.close();
    }

    #[test]
    fn one_side_forgot_requires_pairing_again() {
        let (a, b, port) = paired();
        b.link.disconnect();
        wait("ayrıldı", || a.link.phase() == LinkPhase::Idle);
        a.trust.forget(&b.id.id);
        b.link.connect(local(port));
        wait("yeniden eşleştirme", || is_pairing(&a.link) && is_pairing(&b.link));
        a.link.confirm_pairing(true);
        b.link.confirm_pairing(true);
        wait("bağlandı", || a.link.is_connected() && b.link.is_connected());
        assert!(a.trust.is_trusted(&b.id.id, &b.id.raw_b64()));
        a.link.close();
    }

    #[test]
    fn pairing_timeout() {
        let a = node("A");
        let b = node("B");
        a.link.set_timings(PING_EVERY, DEAD_AFTER, Duration::from_millis(300));
        let port = a.link.listen(0).unwrap();
        b.link.connect(local(port));
        wait("zaman aşımı", || a.link.last_error() == "eşleştirme zaman aşımı");
        wait("b kapandı", || b.link.phase() == LinkPhase::Idle);
        a.link.close();
    }

    #[test]
    fn only_one_session() {
        let (a, b, port) = paired();
        let c = node("C");
        c.link.connect(local(port));
        wait("c reddedildi", || c.link.phase() == LinkPhase::Idle);
        assert!(!c.link.last_error().is_empty());
        assert!(a.link.is_connected() && b.link.is_connected());
        assert_eq!(a.link.peer().unwrap().id, b.id.id);
        a.link.close();
        b.link.close();
    }

    #[test]
    fn connect_refused_reports_error() {
        let b = node("B");
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        b.link.connect(local(port));
        wait("hata", || b.link.phase() == LinkPhase::Idle && b.link.last_error().starts_with("bağlanılamadı"));
    }

    /// Bağlantıyı Link sunucusuna ham el sıkışmayla yapan sahte istemci.
    fn raw_client(port: u16) -> (TcpStream, Sealer, Link, Node) {
        let a = node("A");
        a.link.set_timings(Duration::from_millis(100), Duration::from_millis(500), PAIR_TIMEOUT);
        let p = a.link.listen(port).unwrap();
        let s = TcpStream::connect(local(p)).unwrap();
        let me = Identity::generate();
        let (_, sealer, _) = raw_handshake(&s, &me, "sahte", Role::Client).unwrap();
        wait("kontrol", || a.link.phase() == LinkPhase::Checking);
        (s, sealer, a.link.clone(), a)
    }

    #[test]
    fn tampered_frame_closes() {
        let (mut s, _sealer, link, _a) = raw_client(0);
        write_frame(&mut s, &[7u8; 40]).unwrap();
        wait("kapandı", || link.phase() == LinkPhase::Idle);
        assert_eq!(link.last_error(), "şifreli paket doğrulanamadı");
        // Bağlantı gerçekten kapatıldı.
        let mut buf = Vec::new();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let _ = s.read_to_end(&mut buf);
        link.close();
    }

    #[test]
    fn unexpected_message_while_checking_closes() {
        let (mut s, mut sealer, link, _a) = raw_client(0);
        let ct = sealer.seal(br#"{"t":"clip.text","text":"erken"}"#).unwrap();
        write_frame(&mut s, &ct).unwrap();
        wait("kapandı", || link.phase() == LinkPhase::Idle);
        assert_eq!(link.last_error(), "beklenmeyen mesaj");
        link.close();
    }

    #[test]
    fn non_object_message_closes() {
        let (mut s, mut sealer, link, _a) = raw_client(0);
        let ct = sealer.seal(br#"{"x":1}"#).unwrap();
        write_frame(&mut s, &ct).unwrap();
        wait("kapandı", || link.phase() == LinkPhase::Idle);
        assert_eq!(link.last_error(), "geçersiz mesaj");
        link.close();
    }

    #[test]
    fn silent_peer_is_dead() {
        let (s, _sealer, link, _a) = raw_client(0);
        wait("koptu", || link.last_error() == "bağlantı koptu");
        drop(s);
        link.close();
    }

    /// El sıkışmayı bitirip `trust` göndermeyen, yalnızca ping atan eş tek oturum yerini sonsuza dek tutamaz.
    #[test]
    fn checking_peer_cannot_hold_slot() {
        let a = node("A");
        a.link.set_timings(Duration::from_millis(100), Duration::from_millis(500), Duration::from_millis(800));
        let link = a.link.clone();
        let mut s = TcpStream::connect(local(link.listen(0).unwrap())).unwrap();
        let (_, mut sealer, _) = raw_handshake(&s, &Identity::generate(), "sahte", Role::Client).unwrap();
        wait("kontrol", || link.phase() == LinkPhase::Checking);
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) && link.phase() == LinkPhase::Checking {
            let ct = sealer.seal(br#"{"t":"ping"}"#).unwrap();
            if write_frame(&mut s, &ct).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(link.phase(), LinkPhase::Idle);
        assert_eq!(link.last_error(), "eşleştirme zaman aşımı");
        link.close();
    }

    /// Gelen el sıkışmalar seyreltilir: ortadaki biri sunucuya arka arkaya bağlanıp kodu
    /// tutana kadar (karşı tarafa görünmeden) deneme yapamaz.
    #[test]
    fn incoming_handshakes_are_paced() {
        let a = node("A");
        let port = a.link.listen(0).unwrap();
        let t = Instant::now();
        for _ in 0..3 {
            let mut s = TcpStream::connect(local(port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            read_frame(&mut s).unwrap(); // sunucu HELLO'su
            drop(s);
        }
        assert!(t.elapsed() >= INCOMING_GAP * 2, "{:?}", t.elapsed());
        a.link.close();
    }

    #[test]
    fn garbage_handshake_then_recovers() {
        let a = node("A");
        let port = a.link.listen(0).unwrap();
        {
            let mut s = TcpStream::connect(local(port)).unwrap();
            s.write_all(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]).unwrap();
            let mut buf = Vec::new();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let _ = s.read_to_end(&mut buf);
        }
        wait("hata kaydedildi", || a.link.last_error().contains("çok büyük"));
        let mut s = TcpStream::connect(local(port)).unwrap();
        write_frame(&mut s, b"{\"t\":\"hello\"").unwrap();
        wait("geçersiz", || a.link.last_error() == "bu bir Köprü cihazı değil");
        // Hâlâ yeni bağlantı kabul eder.
        let b = node("B");
        b.link.connect(local(port));
        wait("eşleştirme", || is_pairing(&a.link) && is_pairing(&b.link));
        a.link.close();
    }

    #[test]
    fn plaintext_absent_on_the_wire() {
        let a = node("A");
        let b = node("B");
        let port = a.link.listen(0).unwrap();
        // Ortadaki dinleyici: B <-> proxy <-> A, akan tüm baytları kaydeder.
        let pl = TcpListener::bind("127.0.0.1:0").unwrap();
        let pport = pl.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let seen2 = seen.clone();
        thread::spawn(move || {
            let (c, _) = pl.accept().unwrap();
            let up = TcpStream::connect(local(port)).unwrap();
            let pump = |mut from: TcpStream, mut to: TcpStream, rec: Arc<Mutex<Vec<u8>>>| {
                thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = from.read(&mut buf) {
                        if n == 0 || to.write_all(&buf[..n]).is_err() {
                            break;
                        }
                        rec.lock().unwrap().extend_from_slice(&buf[..n]);
                    }
                    let _ = to.shutdown(Shutdown::Both);
                })
            };
            pump(c.try_clone().unwrap(), up.try_clone().unwrap(), seen2.clone());
            pump(up, c, seen2);
        });
        b.link.connect(local(pport));
        wait("eşleştirme", || is_pairing(&a.link) && is_pairing(&b.link));
        a.link.confirm_pairing(true);
        b.link.confirm_pairing(true);
        wait("bağlandı", || a.link.is_connected() && b.link.is_connected());
        b.link.send(&Msg::ClipText { text: "gizli-veri-123".into() });
        assert_eq!(next_msg(&a.rx), Msg::ClipText { text: "gizli-veri-123".into() });
        a.link.send(&Msg::ClipText { text: "gizli-yanit-456".into() });
        assert_eq!(next_msg(&b.rx), Msg::ClipText { text: "gizli-yanit-456".into() });
        let s = seen.lock().unwrap().clone();
        let has = |n: &[u8]| s.windows(n.len()).any(|w| w == n);
        assert!(has(b"\"app\":\"kopru\""), "HELLO açık metin görünmeli (kayıt çalışıyor)");
        assert!(!has(b"gizli-veri-123"));
        assert!(!has(b"gizli-yanit-456"));
        assert!(!has(b"clip.text"));
        assert!(!has(b"pair.ok"));
        a.link.close();
        b.link.close();
    }

    #[test]
    fn handler_consumes_in_order() {
        let (a, b, _) = paired();
        let got = Arc::new(Mutex::new(Vec::new()));
        let g2 = got.clone();
        a.link.set_handler(Some(Arc::new(move |m: &Msg| {
            if let Msg::ClipText { text } = m {
                g2.lock().unwrap().push(text.clone());
                return true;
            }
            false
        })));
        for i in 0..200 {
            assert!(b.link.send(&Msg::ClipText { text: format!("m{i}") }));
        }
        b.link.send(&Msg::FileCancel { id: "x".into() });
        assert_eq!(next_msg(&a.rx), Msg::FileCancel { id: "x".into() });
        let g = got.lock().unwrap();
        assert_eq!(g.len(), 200);
        assert!(g.iter().enumerate().all(|(i, t)| *t == format!("m{i}")));
        a.link.close();
        b.link.close();
    }

    #[test]
    fn oversize_send_refused() {
        let (a, b, _) = paired();
        assert!(!b.link.send(&Msg::ClipText { text: "x".repeat(super::super::framing::MAX_FRAME) }));
        assert!(b.link.is_connected());
        let _ = tcp_pair;
        a.link.close();
        b.link.close();
    }
}
