//! Apple tarzı tema: renk belirteçleri (açık/koyu), yazı tipleri ve egui stili.
//!
//! Yazı tipi: Inter 4.1 (SIL Open Font License 1.1, `assets/fonts/OFL.txt`).
//! `assets/fonts/*.ttf` dosyaları resmi `inter-ui` paketindeki WOFF2 dosyalarından
//! Latin + Latin Genişletilmiş-A alt kümesine indirilip TTF'ye çevrildi; rakamlar
//! sabit genişlikli (`tnum`) olacak şekilde cmap'e işlendi, böylece ilerleme ve
//! boyut sayıları titremez. Eksik glifler egui'nin varsayılan yazı tiplerine düşer.

use egui::{
    epaint::Shadow, style::Selection, Color32, FontData, FontDefinitions, FontFamily, FontId,
    Rounding, Stroke, Style, TextStyle, Theme, Vec2, Visuals,
};

pub const FONT_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");

/// Kalın yazı ailesinin adı.
pub const SEMIBOLD: &str = "inter-semibold";

/// Köşe yarıçapları.
/// `Stroke::new` için tür çıkarımı sorunsuz kısaltma.
pub fn st(width: f32, color: Color32) -> Stroke {
    Stroke::new(width, color)
}

pub const R_CARD: f32 = 12.0;
pub const R_CONTROL: f32 = 7.0;
pub const R_SHEET: f32 = 14.0;

/// Arayüzün kullandığı anlamsal renkler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// İçerik alanı zemini.
    pub bg: Color32,
    /// Kenar çubuğu zemini (biraz farklı, "vibrancy" hissi).
    pub sidebar: Color32,
    /// Gruplanmış liste kartı.
    pub card: Color32,
    /// Karttaki satırın üzerine gelince.
    pub card_hover: Color32,
    /// İnce ayırıcı çizgi.
    pub separator: Color32,
    /// Kart / kontrol kenarı.
    pub hairline: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub green: Color32,
    pub orange: Color32,
    pub red: Color32,
    pub red_soft: Color32,
    /// Kontrol dolgusu (ikincil düğme, segment rayı, kapalı anahtar).
    pub fill: Color32,
    pub fill_hover: Color32,
    pub fill_active: Color32,
    /// Seçili segment / anahtar topuzu.
    pub raised: Color32,
    /// Kenar çubuğunda seçili öğe.
    pub selected: Color32,
    /// Metin kutusu zemini.
    pub input: Color32,
    /// Modal sayfanın arkasındaki karartma.
    pub scrim: Color32,
    pub shadow: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Saydam renk (alfa 0-255). egui gamma uzayında karıştırdığı için CSS'teki `rgba()` gibi
/// davranması adına gamma uzayında önçarpılır (`from_rgba_unmultiplied` doğrusal uzayda çarpar
/// ve düşük alfalarda beklenenden çok daha parlak sonuç verir).
fn a(rgb: u32, alpha: u8) -> Color32 {
    let c = hex(rgb);
    let m = |x: u8| ((x as u16 * alpha as u16 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(c.r()), m(c.g()), m(c.b()), alpha)
}

impl Palette {
    pub fn dark() -> Self {
        Palette {
            dark: true,
            bg: hex(0x1E1E20),
            sidebar: hex(0x28282B),
            card: hex(0x2A2A2D),
            card_hover: hex(0x323236),
            separator: a(0xFFFFFF, 18),
            hairline: a(0xFFFFFF, 22),
            text: hex(0xF2F2F7),
            text2: hex(0x9C9CA3),
            text3: hex(0x6B6B72),
            accent: hex(0x0A84FF),
            accent_hover: hex(0x3396FF),
            accent_soft: a(0x0A84FF, 46),
            on_accent: Color32::WHITE,
            green: hex(0x30D158),
            orange: hex(0xFF9F0A),
            red: hex(0xFF453A),
            red_soft: a(0xFF453A, 40),
            fill: hex(0x3A3A3E),
            fill_hover: hex(0x444449),
            fill_active: hex(0x4E4E54),
            raised: hex(0x636369),
            selected: a(0xFFFFFF, 22),
            input: hex(0x1F1F22),
            scrim: a(0x000000, 120),
            shadow: a(0x000000, 110),
        }
    }

    pub fn light() -> Self {
        Palette {
            dark: false,
            bg: hex(0xF5F5F7),
            sidebar: hex(0xEAEAEE),
            card: hex(0xFFFFFF),
            card_hover: hex(0xF7F7F9),
            separator: hex(0xE6E6EB),
            hairline: hex(0xDEDEE3),
            text: hex(0x1D1D1F),
            text2: hex(0x6E6E73),
            text3: hex(0xA1A1A6),
            accent: hex(0x007AFF),
            accent_hover: hex(0x1A88FF),
            accent_soft: a(0x007AFF, 30),
            on_accent: Color32::WHITE,
            green: hex(0x34C759),
            orange: hex(0xFF9500),
            red: hex(0xFF3B30),
            red_soft: a(0xFF3B30, 26),
            fill: hex(0xE9E9EC),
            fill_hover: hex(0xE0E0E4),
            fill_active: hex(0xD5D5DA),
            raised: hex(0xFFFFFF),
            selected: a(0x000000, 18),
            input: hex(0xFFFFFF),
            scrim: a(0x000000, 60),
            shadow: a(0x000000, 40),
        }
    }

    pub fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::light(),
        }
    }
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

thread_local! {
    /// Bu karede kalın aile yüklü mü? `set_fonts` bir sonraki karede etkili olur; ilk karede
    /// tanımsız aileye başvurmak epaint'i panikletir, bu yüzden o zaman normal yazıya düşülür.
    static SEMIBOLD_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Her karenin başında, kare içinde çağrılır (kare dışında `ctx.fonts` kullanılamaz).
pub fn refresh_font_readiness(ctx: &egui::Context) {
    let ready = ctx.fonts(|f| f.families().contains(&FontFamily::Name(SEMIBOLD.into())));
    SEMIBOLD_READY.with(|c| c.set(ready));
}

pub fn semibold(size: f32) -> FontId {
    if SEMIBOLD_READY.with(|c| c.get()) {
        FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
    } else {
        FontId::new(size, FontFamily::Proportional)
    }
}

/// Yazı tiplerini ve iki temanın stilini bağlama bir kez yükler (tekrar çağrılması zararsızdır).
pub fn install(ctx: &egui::Context) {
    let key = egui::Id::new("darkdrop-theme-installed");
    if ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, true));

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), FontData::from_static(FONT_REGULAR));
    fonts
        .font_data
        .insert("inter-sb".into(), FontData::from_static(FONT_SEMIBOLD));
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(p) = fonts.families.get_mut(&FontFamily::Proportional) {
        p.insert(0, "inter".into());
    }
    let mut sb = vec!["inter-sb".to_string()];
    sb.extend(fallbacks);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), sb);
    ctx.set_fonts(fonts);

    ctx.set_style_of(Theme::Dark, style(&Palette::dark()));
    ctx.set_style_of(Theme::Light, style(&Palette::light()));
}

fn style(p: &Palette) -> Style {
    let mut s = Style::default();
    s.text_styles = [
        (TextStyle::Small, regular(11.5)),
        (TextStyle::Body, regular(13.5)),
        (TextStyle::Button, regular(13.5)),
        (TextStyle::Heading, regular(20.0)),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();
    s.spacing.item_spacing = Vec2::new(8.0, 6.0);
    s.spacing.button_padding = Vec2::new(12.0, 5.0);
    s.spacing.interact_size = Vec2::new(28.0, 26.0);
    s.spacing.scroll = egui::style::ScrollStyle::floating();
    s.spacing.scroll.floating_width = 6.0;
    s.animation_time = 0.16;
    s.visuals = visuals(p);
    s
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.override_text_color = None;
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = st(1.0, p.hairline);
    v.window_rounding = Rounding::same(R_SHEET);
    v.window_shadow = Shadow {
        offset: Vec2::new(0.0, 10.0),
        blur: 30.0,
        spread: 0.0,
        color: p.shadow,
    };
    v.popup_shadow = Shadow {
        offset: Vec2::new(0.0, 4.0),
        blur: 14.0,
        spread: 0.0,
        color: p.shadow,
    };
    v.menu_rounding = Rounding::same(R_CONTROL);
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.card_hover;
    v.code_bg_color = p.fill;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.orange;
    v.error_fg_color = p.red;
    v.selection = Selection {
        bg_fill: p.accent_soft,
        stroke: st(1.0, p.accent),
    };
    v.text_cursor.stroke = st(1.5, p.accent);
    v.striped = false;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.handle_shape = egui::style::HandleShape::Circle;

    let r = Rounding::same(R_CONTROL);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = st(1.0, p.separator);
    w.noninteractive.fg_stroke = st(1.0, p.text);
    w.noninteractive.rounding = r;

    w.inactive.bg_fill = p.fill;
    w.inactive.weak_bg_fill = p.fill;
    w.inactive.bg_stroke = st(1.0, p.hairline);
    w.inactive.fg_stroke = st(1.0, p.text);
    w.inactive.rounding = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.fill_hover;
    w.hovered.weak_bg_fill = p.fill_hover;
    w.hovered.bg_stroke = st(1.0, p.hairline);
    w.hovered.fg_stroke = st(1.0, p.text);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.fill_active;
    w.active.weak_bg_fill = p.fill_active;
    w.active.bg_stroke = st(1.0, p.accent);
    w.active.fg_stroke = st(1.0, p.text);
    w.active.rounding = r;
    w.active.expansion = 0.0;

    w.open = w.active;
    v
}
