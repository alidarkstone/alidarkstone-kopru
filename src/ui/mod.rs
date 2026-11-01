//! Köprü arayüzü (egui). Yalnızca `crate::state::UiState` okur ve `crate::state::Action` üretir.
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
    fn macos_default() -> Self {
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
        draw(ctx, &s, &mut self.out, &mut self.prefs);
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
        .with_title("Köprü")
        .with_app_id("kopru")
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
        "Köprü",
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

/// 64×64 uygulama simgesi: mavi yuvarlak kare içinde beyaz köprü kemeri (kodla üretilir).
fn app_icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let (top, bot) = ([0x3A, 0x9B, 0xFF], [0x00, 0x6A, 0xE6]);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // yuvarlak kare maskesi (kenar yumuşatmalı)
            let r = 14.0;
            let dx = (fx - 32.0).abs() - (32.0 - 2.0 - r);
            let dy = (fy - 32.0).abs() - (32.0 - 2.0 - r);
            let d = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r;
            let mask = (0.5 - d).clamp(0.0, 1.0);
            if mask <= 0.0 {
                continue;
            }
            let t = fy / N as f32;
            let mut c = [0f32; 3];
            for i in 0..3 {
                c[i] = top[i] as f32 + (bot[i] as f32 - top[i] as f32) * t;
            }
            // beyaz kemer ve tabliye
            let arc_d = (((fx - 32.0).powi(2) + (fy - 58.0).powi(2)).sqrt() - 30.0).abs();
            let arc = fy < 44.0 && fx > 8.0 && fx < 56.0;
            let mut white = 0.0f32;
            if arc {
                white = (2.6 - arc_d).clamp(0.0, 1.0);
            }
            if (fy - 42.0).abs() < 2.2 && fx > 7.0 && fx < 57.0 {
                white = 1.0;
            }
            for px in [20.0f32, 32.0, 44.0] {
                let ytop = 58.0 - (900.0 - (px - 32.0).powi(2)).sqrt();
                if (fx - px).abs() < 1.3 && fy > ytop && fy < 42.0 {
                    white = 1.0;
                }
            }
            if ((fx - 14.0).abs() < 1.8 || (fx - 50.0).abs() < 1.8) && fy > 42.0 && fy < 52.0 {
                white = 1.0;
            }
            let i = (y * N + x) * 4;
            for k in 0..3 {
                rgba[i + k] = (c[k] + (255.0 - c[k]) * white).round() as u8;
            }
            rgba[i + 3] = (mask * 255.0) as u8;
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
    }
}

impl ThemeChoice {
    pub(crate) fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(self.preference());
    }
}
