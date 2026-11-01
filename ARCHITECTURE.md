# Köprü (Rust) — mimari ve ekip sözleşmesi

Hedef: **tek dosya, kurulumsuz** uygulama. Windows: tek `kopru.exe` (x86_64-pc-windows-gnu, `#![windows_subsystem = "windows"]`).
macOS: tek `Köprü.app` (GitHub Actions'ta derlenir). Harici yardımcı program YOK; her şey aynı ikili dosyada.
Referans tasarım: `reference/electron/` (Electron sürümü). Protokol **tel düzeyinde birebir aynı** olmalı
(`reference/electron/src/core/channel.js`, `link.js`, `discovery.js`, `features/*.js`). Davranışı oradan okuyun ama Rust'ta deyimsel yazın.

## Modüller ve sahipleri

| Dosya(lar) | Sahibi | İçerik |
|---|---|---|
| `src/msg.rs`, `src/state.rs` | koordinatör | Ortak türler. Değiştirmeniz gerekirse raporunuzda belirtin, sessizce değiştirmeyin. |
| `src/net/{mod,framing,keys,channel,link,discovery,trust}.rs` | **çekirdek** | Şifreli kanal, eşleştirme, keşif, güvenilen cihazlar |
| `src/features/{mod,files,clipboard}.rs` | **çekirdek** | Dosya aktarımı, pano |
| `src/input/{mod,windows,macos,keymap}.rs` | **giriş** | Fare/klavye paylaşımı durum makinesi + platform arka uçları |
| `src/ui/{mod,theme,widgets,view}.rs` | **arayüz** | egui arayüzü, Apple tarzı tema (açık/koyu) |
| `src/main.rs`, `src/app.rs`, `src/config.rs` | koordinatör | Birleştirme |

Başkasının dosyasına yazmayın. Kendi modülünüz `cargo build` ve `cargo test` ile tek başına derlenmeli
(diğer modüller yokken derlemek için geçici `mod` satırlarını kendi test ortamınızda tutabilirsiniz ama `src/main.rs`'yi koordinatör yönetir).

## Arayüzler (koordinatörün beklediği API)

### net
```rust
pub struct Identity { pub id: String /*16 hex*/, pub raw: [u8;32], /* signing key */ }
impl Identity { pub fn generate() -> Self; pub fn from_pkcs8_pem(&str) -> Result<Self>; pub fn to_pkcs8_pem(&self) -> String; }

pub trait TrustStore: Send + Sync { fn is_trusted(&self, id: &str, raw_b64url: &str) -> bool; fn add(&self, id: &str, raw_b64url: &str, name: &str); }

pub enum LinkEvent { State, Message(crate::msg::Msg), Drain }
pub struct Link;  // Clone + Send + Sync (Arc içi)
impl Link {
  pub fn new(identity: Arc<Identity>, name: Arc<RwLock<String>>, trust: Arc<dyn TrustStore>, events: std::sync::mpsc::Sender<LinkEvent>) -> Self;
  pub fn listen(&self, port: u16) -> io::Result<u16>;     // 0 => rastgele; 47910 varsayılan
  pub fn connect(&self, addr: SocketAddr);
  pub fn disconnect(&self);
  pub fn confirm_pairing(&self, ok: bool);
  pub fn send(&self, m: &Msg) -> bool;                      // bağlı değilse false
  pub fn phase(&self) -> crate::state::LinkPhase;
  pub fn peer(&self) -> Option<crate::state::PeerView>;
  pub fn last_error(&self) -> String;
  pub fn close(&self);
}
pub struct Discovery; impl Discovery { pub fn start(id, name: Arc<RwLock<String>>, tcp_port: u16) -> io::Result<Self>; pub fn list(&self) -> Vec<(id,name,addr,port)>; }
```
### features
```rust
pub type Sender = Arc<dyn Fn(&Msg) -> bool + Send + Sync>;
pub struct FileShare; impl FileShare { new(send: Sender, download_dir: Arc<RwLock<PathBuf>>); offer(&Path)->io::Result<String>; accept/reject/cancel(&str); on_message(&Msg)->bool; abort_all(); list()->Vec<TransferView>; }
pub struct ClipboardShare; impl ClipboardShare { start(send: Sender, enabled: Arc<AtomicBool>) -> Self; on_message(&Msg)->bool; }
```
### input
```rust
pub struct InputShare; impl InputShare {
  pub fn new(send: Sender) -> Self;   // platform arka ucunu seçer; desteklenmiyorsa available=false
  pub fn configure(&self, enabled: bool, edge: Edge);
  pub fn on_link(&self, connected: bool);
  pub fn on_message(&self, m: &Msg) -> bool;
  pub fn view(&self) -> crate::state::InputView;
}
```
### ui
```rust
pub fn run(state: Arc<dyn Fn() -> UiState + Send + Sync>, act: Arc<dyn Fn(Action) + Send + Sync>, repaint_hook: impl FnOnce(egui::Context)) -> eframe::Result<()>;
pub fn run_demo() -> eframe::Result<()>; // UiState::demo() ile
```

## Güvenlik kuralları (herkes)
- Karşı taraftan gelen her alan güvenilmezdir: boyut, dosya adı, sayılar sınırlanır ve doğrulanır.
- `unsafe` yalnızca platform API çağrılarında, kısa ve yorumlu.
- Ağ yalnızca yerel eş cihazla; internete hiçbir bağlantı yok. Telemetri yok.
- Yönetici izni istenmez; sistem ayarı, güvenlik duvarı, kullanıcı hesabı değiştirilmez.
- Panik yerine hata döndürün; ağ girdisi asla panik ettirmemeli.

## Derleme
```
cargo build --release --target x86_64-pc-windows-gnu   # kopru.exe
cargo test                                              # Linux'ta birim testleri
```
