//! DarkDrop arayüzü (egui). Yalnızca `crate::state::UiState` okur ve `crate::state::Action` üretir.
//!
//! - [`run`]: gerçek uygulama penceresi.
//! - [`run_demo`]: `UiState::demo()` ile örnek pencere.
//! - [`draw`]: saf çizim fonksiyonu; pencere olmadan (testlerde) de çalışır.

pub mod theme;
pub mod view;
pub mod widgets;

#[cfg(test)]
mod raster;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::state::{Action, UiState};

pub use view::draw;

/// Kenar çubuğundaki bölümler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Devices,
    Clipboard,
    Files,
    Settings,
}

/// Görünüm tercihi. Varsayılan: işletim sistemini izle (okunamazsa koyu).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    /// Ayar dosyasında saklamak için.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        }
    }
    fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// Yalnızca arayüze ait, ağ durumundan bağımsız tercihler ve geçici girişler.
#[derive(Clone, Debug, Default)]
pub struct UiPrefs {
    pub tab: Tab,
    pub theme: ThemeChoice,
    /// macOS'ta saydam başlık çubuğu altında kalan üst boşluk (pt). `run` ayarlar.
    pub titlebar_inset: f32,
    /// "Adresle bağlan" kutusunun içeriği.
    pub address: String,
    /// Cihaz adı düzenlenirken tampon (None: durumdaki ad gösterilir).
    pub name_edit: Option<String>,
}

impl UiPrefs {
    pub fn macos_default() -> Self {
        UiPrefs {
            titlebar_inset: if cfg!(target_os = "macos") { 26.0 } else { 0.0 },
            ..Default::default()
        }
    }
}

type StateFn = Arc<dyn Fn() -> UiState + Send + Sync>;
type ActFn = Arc<dyn Fn(Action) + Send + Sync>;

struct App {
    state: StateFn,
    act: ActFn,
    prefs: UiPrefs,
    out: Vec<Action>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let s = (self.state)();
        self.out.clear();
        let theme_before = self.prefs.theme;
        draw(ctx, &s, &mut self.out, &mut self.prefs);
        if self.prefs.theme != theme_before {
            self.out.push(Action::SetTheme(self.prefs.theme.as_str().to_string()));
        }
        for a in self.out.drain(..) {
            (self.act)(a);
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

fn native_options() -> eframe::NativeOptions {
    let mut vp = egui::ViewportBuilder::default()
        .with_title("DarkDrop")
        .with_app_id("darkdrop")
        .with_inner_size([920.0, 640.0])
        .with_min_inner_size([720.0, 520.0])
        .with_icon(app_icon());
    if cfg!(target_os = "macos") {
        // Birleşik başlık çubuğu: trafik ışıkları kenar çubuğunun üstünde durur.
        vp = vp
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    eframe::NativeOptions {
        viewport: vp,
        centered: true,
        ..Default::default()
    }
}

/// Uygulama penceresini açar; pencere kapanınca döner.
///
/// `on_ctx`, egui bağlamı oluşur oluşmaz bir kez çağrılır: arka plan iş parçacıkları
/// durum değişince `ctx.request_repaint()` çağırabilsin diye.
pub fn run(
    state: Arc<dyn Fn() -> UiState + Send + Sync>,
    act: Arc<dyn Fn(Action) + Send + Sync>,
    on_ctx: Box<dyn FnOnce(egui::Context) + Send>,
) -> eframe::Result<()> {
    run_with(state, act, on_ctx, UiPrefs::macos_default())
}

/// [`run`] ile aynı; başlangıç tercihleri (ör. kayıtlı tema) verilebilir.
pub fn run_with(
    state: StateFn,
    act: ActFn,
    on_ctx: Box<dyn FnOnce(egui::Context) + Send>,
    prefs: UiPrefs,
) -> eframe::Result<()> {
    eframe::run_native(
        "DarkDrop",
        native_options(),
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            on_ctx(cc.egui_ctx.clone());
            Ok(Box::new(App {
                state,
                act,
                prefs,
                out: Vec::new(),
            }))
        }),
    )
}

/// Örnek veriyle pencere (arayüz geliştirme için). Eylemler yalnızca stderr'e yazılır.
pub fn run_demo() -> eframe::Result<()> {
    run(
        Arc::new(UiState::demo),
        Arc::new(|a| eprintln!("[demo] {a:?}")),
        Box::new(|_| {}),
    )
}

/// Uygulama simgesi: DarkDrop logosu (assets/logo/darkdrop-256.rgba, 256×256 RGBA).
pub(crate) const LOGO_RGBA: &[u8] = include_bytes!("../../assets/logo/darkdrop-256.rgba");
pub(crate) const LOGO_SIZE: usize = 256;

fn app_icon() -> egui::IconData {
    egui::IconData { rgba: LOGO_RGBA.to_vec(), width: LOGO_SIZE as u32, height: LOGO_SIZE as u32 }
}

impl ThemeChoice {
    pub(crate) fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(self.preference());
    }
}
