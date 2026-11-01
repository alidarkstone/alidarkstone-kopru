//! Özel çizilmiş, macOS hissi veren küçük bileşenler: anahtar, segment kontrolü,
//! düğmeler, gruplu liste bölümleri, ilerleme çubuğu, durum noktası, dönen gösterge
//! ve vektör simgeler.

use egui::{
    pos2, vec2, Align, Align2, Color32, CursorIcon, FontId, Layout, Margin, Painter, Pos2, Rect,
    Response, RichText, Rounding, Sense, Shape, Stroke, Ui,
};

use super::theme::{self, st, Palette, R_CARD, R_CONTROL};

/// 1 fiziksel piksellik çizgi kalınlığı.
pub fn hairline(ui: &Ui) -> f32 {
    1.0 / ui.ctx().pixels_per_point().max(0.5)
}

// ───────────────────────────── metin ─────────────────────────────

pub fn text(s: impl Into<String>, font: FontId, color: Color32) -> RichText {
    RichText::new(s).font(font).color(color)
}

/// Taşarsa "…" ile kısaltılan tek satır etiket.
pub fn line(ui: &mut Ui, s: impl Into<String>, font: FontId, color: Color32) -> Response {
    ui.add(
        egui::Label::new(text(s, font, color))
            .truncate()
            .selectable(false),
    )
}

/// Satır kaydırmalı paragraf.
pub fn para(ui: &mut Ui, s: impl Into<String>, font: FontId, color: Color32) -> Response {
    ui.add(
        egui::Label::new(text(s, font, color))
            .wrap()
            .selectable(false),
    )
}

// ───────────────────────────── düğmeler ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Vurgu renginde dolu düğme (varsayılan eylem).
    Primary,
    /// Gri dolgulu düğme.
    Secondary,
    /// Gri dolgu, kırmızı yazı (geri alınamaz eylem).
    Destructive,
    /// Zeminsiz, vurgu renginde yazı.
    Plain,
}

pub struct Btn {
    label: String,
    kind: Kind,
    enabled: bool,
    min_width: f32,
    height: f32,
}

impl Btn {
    pub fn new(label: impl Into<String>, kind: Kind) -> Self {
        Btn {
            label: label.into(),
            kind,
            enabled: true,
            min_width: 0.0,
            height: 28.0,
        }
    }
    pub fn primary(label: impl Into<String>) -> Self {
        Self::new(label, Kind::Primary)
    }
    pub fn secondary(label: impl Into<String>) -> Self {
        Self::new(label, Kind::Secondary)
    }
    pub fn destructive(label: impl Into<String>) -> Self {
        Self::new(label, Kind::Destructive)
    }
    pub fn plain(label: impl Into<String>) -> Self {
        Self::new(label, Kind::Plain)
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
    pub fn large(mut self) -> Self {
        self.height = 34.0;
        self
    }

    pub fn show(self, ui: &mut Ui, p: &Palette) -> Response {
        let font = if self.kind == Kind::Primary || self.height > 30.0 {
            theme::semibold(13.5)
        } else {
            theme::regular(13.5)
        };
        let galley = ui
            .painter()
            .layout_no_wrap(self.label.clone(), font, Color32::PLACEHOLDER);
        let pad = if self.kind == Kind::Plain { 6.0 } else { 14.0 };
        let w = (galley.size().x + pad * 2.0).max(self.min_width);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(w, self.height), sense);
        if ui.is_rect_visible(rect) {
            let hovered = self.enabled && resp.hovered();
            let pressed = self.enabled && resp.is_pointer_button_down_on();
            let (fill, fg) = match self.kind {
                Kind::Primary => (
                    if pressed {
                        p.accent.gamma_multiply(0.85)
                    } else if hovered {
                        p.accent_hover
                    } else {
                        p.accent
                    },
                    p.on_accent,
                ),
                Kind::Secondary | Kind::Destructive => (
                    if pressed {
                        p.fill_active
                    } else if hovered {
                        p.fill_hover
                    } else {
                        p.fill
                    },
                    if self.kind == Kind::Destructive {
                        p.red
                    } else {
                        p.text
                    },
                ),
                Kind::Plain => (
                    if pressed {
                        p.fill
                    } else if hovered {
                        p.fill.gamma_multiply(0.6)
                    } else {
                        Color32::TRANSPARENT
                    },
                    p.accent,
                ),
            };
            let (fill, fg) = if self.enabled {
                (fill, fg)
            } else if self.kind == Kind::Plain {
                (Color32::TRANSPARENT, p.text3)
            } else {
                // macOS: devre dışı düğme renksiz, yazısı soluk
                (p.fill.gamma_multiply(0.7), p.text3)
            };
            let rounding = Rounding::same(if self.height > 30.0 { 9.0 } else { R_CONTROL });
            ui.painter().rect_filled(rect, rounding, fill);
            if self.kind == Kind::Primary && self.enabled {
                // ince üst parlama: düğmeye hafif kabartma hissi
                let top = Rect::from_min_size(rect.min, vec2(rect.width(), 1.0))
                    .shrink2(vec2(rounding.nw, 0.0));
                ui.painter()
                    .rect_filled(top, 0.0, Color32::from_white_alpha(28));
            }
            let pos = rect.center() - galley.size() / 2.0;
            ui.painter().galley(pos, galley, fg);
        }
        if self.enabled {
            resp.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            resp
        }
    }
}

/// Simgeli yuvarlak küçük düğme (ör. aktarımı iptal et).
pub fn icon_button(ui: &mut Ui, p: &Palette, icon: Icon, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        p.fill_active
    } else if resp.hovered() {
        p.fill_hover
    } else {
        p.fill
    };
    ui.painter().circle_filled(rect.center(), 11.0, fill);
    paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(12.0, 12.0)),
        icon,
        p.text2,
    );
    resp.on_hover_text(tooltip)
        .on_hover_cursor(CursorIcon::PointingHand)
}

// ───────────────────────────── anahtar (iOS switch) ─────────────────────────────

/// iOS tarzı anahtar. Değer değişirse `true` döner (yanıtın `changed()` değeri de ayarlanır).
pub fn switch(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let size = vec2(40.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.18);
        let track = lerp_color(
            if resp.hovered() { p.fill_hover } else { p.fill },
            p.accent,
            t,
        );
        let r = rect.height() / 2.0;
        ui.painter().rect_filled(rect, r, track);
        if !p.dark && t < 0.5 {
            ui.painter()
                .rect_stroke(rect, r, st(hairline(ui), p.hairline));
        }
        let knob_r = r - 2.0;
        let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
        let c = pos2(x, rect.center().y);
        // yumuşak gölge
        ui.painter().circle_filled(
            c + vec2(0.0, 1.0),
            knob_r + 0.5,
            Color32::from_black_alpha(if p.dark { 60 } else { 34 }),
        );
        ui.painter().circle_filled(c, knob_r, Color32::WHITE);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

// ───────────────────────────── segment kontrolü ─────────────────────────────

/// macOS segment kontrolü. Seçim değişirse `true` döner.
pub fn segmented(ui: &mut Ui, p: &Palette, selected: &mut usize, labels: &[&str]) -> bool {
    if labels.is_empty() {
        return false;
    }
    let font = theme::regular(12.5);
    let seg_w = labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap((*l).to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
                + 26.0
        })
        .fold(64.0_f32, f32::max);
    let h = 26.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(seg_w * labels.len() as f32 + 4.0, h), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, R_CONTROL, p.fill);
    let sel = (*selected).min(labels.len() - 1);
    let anim = ui.ctx().animate_value_with_time(
        ui.id().with(("seg", rect.min.x as i32, rect.min.y as i32)),
        sel as f32,
        0.16,
    );
    let thumb = Rect::from_min_size(
        pos2(rect.left() + 2.0 + anim * seg_w, rect.top() + 2.0),
        vec2(seg_w, h - 4.0),
    );
    painter.rect_filled(
        thumb.translate(vec2(0.0, 0.5)),
        R_CONTROL - 1.5,
        Color32::from_black_alpha(if p.dark { 50 } else { 22 }),
    );
    painter.rect_filled(thumb, R_CONTROL - 1.5, p.raised);
    let mut changed = false;
    for (i, l) in labels.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + 2.0 + i as f32 * seg_w, rect.top() + 2.0),
            vec2(seg_w, h - 4.0),
        );
        let resp = ui.interact(
            r,
            ui.id().with(("seg-item", i, rect.min.x as i32)),
            Sense::click(),
        );
        if resp.clicked() && *selected != i {
            *selected = i;
            changed = true;
        }
        // seçili olmayan bölmeler arasında ince ayraç
        if i > 0 && i != sel && i != sel + 1 {
            painter.line_segment(
                [
                    pos2(r.left(), r.top() + 5.0),
                    pos2(r.left(), r.bottom() - 5.0),
                ],
                st(1.0, p.hairline),
            );
        }
        let f = if i == sel {
            theme::semibold(12.5)
        } else {
            font.clone()
        };
        painter.text(r.center(), Align2::CENTER_CENTER, *l, f, p.text);
        resp.on_hover_cursor(CursorIcon::PointingHand);
    }
    changed
}

// ───────────────────────────── gruplu liste (inset list) ─────────────────────────────

/// System Settings tarzı gruplu bölüm: üst başlık, yuvarlak köşeli kart, satırlar arası ince ayırıcı, alt not.
pub fn section(
    ui: &mut Ui,
    p: &Palette,
    header: Option<&str>,
    footer: Option<&str>,
    body: impl FnOnce(&mut Section),
) {
    if let Some(h) = header {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            line(ui, h, theme::semibold(12.5), p.text2);
        });
        ui.add_space(4.0);
    }
    let w = ui.available_width();
    egui::Frame::none()
        .fill(p.card)
        .rounding(R_CARD)
        .stroke(st(
            hairline(ui),
            if p.dark { p.hairline } else { p.separator },
        ))
        .show(ui, |ui| {
            ui.set_width(w);
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut s = Section { ui, p: *p, rows: 0 };
            body(&mut s);
        });
    if let Some(f) = footer {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.set_max_width(w - 12.0);
            para(ui, f, theme::regular(11.5), p.text2);
        });
    }
    ui.add_space(22.0);
}

pub struct Section<'a> {
    pub ui: &'a mut Ui,
    pub p: Palette,
    rows: usize,
}

impl Section<'_> {
    /// Satır: soldaki içerik kalan genişliği kullanır (uzun metin kısaltılır), sağdaki içerik sağa yaslanır.
    pub fn row(&mut self, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
        self.row_inset(14.0, left, right)
    }

    /// Ayırıcının soldan girintisi verilebilen satır (ör. simgeli satırlarda metne hizalı ayırıcı).
    pub fn row_inset(
        &mut self,
        sep_inset: f32,
        left: impl FnOnce(&mut Ui),
        right: impl FnOnce(&mut Ui),
    ) {
        let p = self.p;
        if self.rows > 0 {
            let y = self.ui.cursor().top();
            let x0 = self.ui.max_rect().left() + sep_inset;
            let x1 = self.ui.max_rect().right();
            let hw = hairline(self.ui);
            self.ui
                .painter()
                .line_segment([pos2(x0, y), pos2(x1, y)], st(hw, p.separator));
        }
        self.rows += 1;
        egui::Frame::none()
            .inner_margin(Margin::symmetric(14.0, 9.0))
            .show(self.ui, |ui| {
                let w = ui.available_width();
                ui.allocate_ui_with_layout(
                    vec2(w, 28.0),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        right(ui);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            left(ui);
                        });
                    },
                );
            });
    }

    /// Ortalanmış boş durum: simge, başlık, nazik açıklama.
    pub fn empty(&mut self, icon: Icon, title: &str, body: &str) {
        let p = self.p;
        self.rows += 1;
        egui::Frame::none()
            .inner_margin(Margin::symmetric(24.0, 22.0))
            .show(self.ui, |ui| {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                    paint_icon(ui.painter(), r, icon, p.text3);
                    ui.add_space(8.0);
                    ui.label(text(title, theme::semibold(13.5), p.text));
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(text(body, theme::regular(12.5), p.text2))
                            .wrap()
                            .halign(Align::Center)
                            .selectable(false),
                    );
                });
            });
    }

    /// Serbest içerikli satır (ör. bırakma alanı).
    pub fn custom(&mut self, add: impl FnOnce(&mut Ui)) {
        if self.rows > 0 {
            let y = self.ui.cursor().top();
            let r = self.ui.max_rect();
            let hw = hairline(self.ui);
            self.ui.painter().line_segment(
                [pos2(r.left() + 14.0, y), pos2(r.right(), y)],
                st(hw, self.p.separator),
            );
        }
        self.rows += 1;
        egui::Frame::none()
            .inner_margin(Margin::symmetric(14.0, 10.0))
            .show(self.ui, add);
    }
}

/// Başlık + alt satırdan oluşan iki satırlı metin bloğu.
pub fn title_sub(ui: &mut Ui, p: &Palette, title: &str, sub: Option<&str>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        line(ui, title, theme::regular(13.5), p.text);
        if let Some(s) = sub {
            line(ui, s, theme::regular(11.5), p.text2);
        }
    });
}

/// Renkli, yuvarlak köşeli simge kutucuğu (System Settings'teki gibi).
pub fn tile(ui: &mut Ui, color: Color32, icon: Icon, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_tile(ui.painter(), rect, color, icon);
    resp
}

pub fn paint_tile(painter: &Painter, rect: Rect, color: Color32, icon: Icon) {
    let r = rect.width() * 0.25;
    painter.rect_filled(rect, r, color);
    let inner = rect.shrink(rect.width() * 0.2);
    paint_icon(painter, inner, icon, Color32::WHITE);
}

// ───────────────────────────── durum göstergeleri ─────────────────────────────

/// Durum noktası; `halo` ile çevresinde yumuşak hale.
pub fn status_dot(painter: &Painter, center: Pos2, color: Color32, halo: bool) {
    if halo {
        painter.circle_filled(center, 6.5, color.gamma_multiply(0.22));
    }
    painter.circle_filled(center, 4.0, color);
}

/// macOS tarzı, çubuklu dönen gösterge.
pub fn spinner(ui: &mut Ui, color: Color32, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(66));
        let t = ui.input(|i| i.time);
        let n = 8;
        let step = ((t * 10.0) as i64).rem_euclid(n) as usize;
        let c = rect.center();
        let (r0, r1) = (size * 0.22, size * 0.46);
        for k in 0..n as usize {
            let ang = k as f32 / n as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let d = vec2(ang.cos(), ang.sin());
            let age = (step + n as usize - k) % n as usize;
            let alpha = 1.0 - age as f32 / n as f32 * 0.8;
            ui.painter().line_segment(
                [c + d * r0, c + d * r1],
                st(size * 0.11, color.gamma_multiply(alpha)),
            );
        }
    }
    resp
}

/// İnce (4 pt) ilerleme çubuğu.
pub fn progress(ui: &mut Ui, p: &Palette, frac: f32, color: Color32) -> Response {
    let w = ui.available_width().max(10.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, p.fill);
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if frac > 0.0 {
        let fw = (rect.width() * frac).max(4.0);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(fw, rect.height())),
            2.0,
            color,
        );
    }
    resp
}

/// Küçük yuvarlak sayı rozeti.
pub fn badge(painter: &Painter, right_center: Pos2, n: usize, fill: Color32, fg: Color32) {
    let s = if n > 99 {
        "99+".to_owned()
    } else {
        n.to_string()
    };
    let g = painter.layout_no_wrap(s, theme::semibold(11.0), fg);
    let w = (g.size().x + 10.0).max(18.0);
    let r = Rect::from_min_size(
        pos2(right_center.x - w, right_center.y - 9.0),
        vec2(w, 18.0),
    );
    painter.rect_filled(r, 9.0, fill);
    painter.galley(r.center() - g.size() / 2.0, g, fg);
}

/// Kesikli, yuvarlak köşeli çerçeve (bırakma alanı).
pub fn dashed_round_rect(painter: &Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let mut pts = Vec::new();
    let corners = [
        (pos2(rect.right() - radius, rect.top() + radius), -90.0_f32),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
        (pos2(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    for (c, start) in corners {
        for i in 0..=8 {
            let a = (start + i as f32 * 90.0 / 8.0).to_radians();
            pts.push(c + vec2(a.cos(), a.sin()) * radius);
        }
    }
    if let Some(first) = pts.first().copied() {
        pts.push(first);
    }
    painter.extend(Shape::dashed_line(&pts, stroke, 6.0, 4.0));
}

// ───────────────────────────── simgeler ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Bridge,
    Laptop,
    Desktop,
    Clipboard,
    Folder,
    Doc,
    Gear,
    ArrowDown,
    ArrowUp,
    Lock,
    Check,
    Xmark,
    Radar,
    Tray,
    Shield,
    Info,
    Swatch,
}

/// Simgeyi `rect` içine, SF Symbols'e benzer ince çizgilerle çizer.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let o = rect.center() - vec2(s, s) / 2.0;
    let pt = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
    let w = (s * 0.09).clamp(1.2, 2.4);
    let sk = st(w, color);
    let rr = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pt(x0, y0), pt(x1, y1));
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| -> Vec<Pos2> {
        (0..=16)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / 16.0).to_radians();
                pt(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    match icon {
        Icon::Bridge => {
            painter.add(Shape::line(arc(0.5, 0.86, 0.42, 200.0, 340.0), sk));
            painter.line_segment([pt(0.04, 0.62), pt(0.96, 0.62)], st(w * 1.2, color));
            for x in [0.26, 0.5, 0.74] {
                let top = 0.86 - (0.42f32.powi(2) - (x - 0.5f32).powi(2)).max(0.0).sqrt() * 0.94;
                painter.line_segment([pt(x, top + 0.04), pt(x, 0.62)], st(w * 0.8, color));
            }
            painter.line_segment([pt(0.16, 0.62), pt(0.16, 0.84)], sk);
            painter.line_segment([pt(0.84, 0.62), pt(0.84, 0.84)], sk);
        }
        Icon::Laptop => {
            painter.rect_stroke(rr(0.18, 0.2, 0.82, 0.66), s * 0.05, sk);
            painter.line_segment([pt(0.04, 0.8), pt(0.96, 0.8)], st(w * 1.3, color));
        }
        Icon::Desktop => {
            painter.rect_stroke(rr(0.08, 0.14, 0.92, 0.68), s * 0.06, sk);
            painter.line_segment([pt(0.5, 0.68), pt(0.5, 0.84)], sk);
            painter.line_segment([pt(0.32, 0.86), pt(0.68, 0.86)], sk);
        }
        Icon::Clipboard => {
            painter.rect_stroke(rr(0.2, 0.16, 0.8, 0.9), s * 0.08, sk);
            painter.rect_filled(rr(0.36, 0.08, 0.64, 0.24), s * 0.05, color);
            painter.line_segment([pt(0.34, 0.46), pt(0.66, 0.46)], sk);
            painter.line_segment([pt(0.34, 0.64), pt(0.56, 0.64)], sk);
        }
        Icon::Folder => {
            let pts = vec![
                pt(0.08, 0.24),
                pt(0.38, 0.24),
                pt(0.46, 0.34),
                pt(0.92, 0.34),
                pt(0.92, 0.82),
                pt(0.08, 0.82),
                pt(0.08, 0.24),
            ];
            painter.add(Shape::line(pts, sk));
            painter.line_segment([pt(0.08, 0.44), pt(0.92, 0.44)], st(w * 0.8, color));
        }
        Icon::Doc => {
            let pts = vec![
                pt(0.2, 0.06),
                pt(0.58, 0.06),
                pt(0.8, 0.28),
                pt(0.8, 0.94),
                pt(0.2, 0.94),
                pt(0.2, 0.06),
            ];
            painter.add(Shape::line(pts, sk));
            painter.add(Shape::line(
                vec![pt(0.58, 0.06), pt(0.58, 0.28), pt(0.8, 0.28)],
                sk,
            ));
        }
        Icon::Gear => {
            let c = pt(0.5, 0.5);
            for k in 0..8 {
                let a = (k as f32 * 45.0).to_radians();
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * s * 0.28, c + d * s * 0.44], st(w * 1.9, color));
            }
            painter.circle_stroke(c, s * 0.3, st(w * 1.2, color));
            painter.circle_stroke(c, s * 0.11, sk);
        }
        Icon::ArrowDown | Icon::ArrowUp => {
            let (a, b, tip) = if icon == Icon::ArrowDown {
                (0.12, 0.84, 1.0)
            } else {
                (0.88, 0.16, -1.0)
            };
            painter.line_segment([pt(0.5, a), pt(0.5, b)], sk);
            painter.add(Shape::line(
                vec![
                    pt(0.24, b - 0.26 * tip),
                    pt(0.5, b),
                    pt(0.76, b - 0.26 * tip),
                ],
                sk,
            ));
        }
        Icon::Lock => {
            painter.rect_filled(rr(0.2, 0.44, 0.8, 0.9), s * 0.1, color);
            painter.add(Shape::line(
                arc(0.5, 0.44, 0.2, 180.0, 360.0),
                st(w * 1.3, color),
            ));
            painter.line_segment([pt(0.3, 0.44), pt(0.3, 0.46)], st(w * 1.3, color));
            painter.line_segment([pt(0.7, 0.44), pt(0.7, 0.46)], st(w * 1.3, color));
        }
        Icon::Check => {
            painter.add(Shape::line(
                vec![pt(0.18, 0.52), pt(0.42, 0.76), pt(0.84, 0.26)],
                st(w * 1.3, color),
            ));
        }
        Icon::Xmark => {
            painter.line_segment([pt(0.24, 0.24), pt(0.76, 0.76)], st(w * 1.2, color));
            painter.line_segment([pt(0.76, 0.24), pt(0.24, 0.76)], st(w * 1.2, color));
        }
        Icon::Radar => {
            painter.circle_filled(pt(0.5, 0.78), s * 0.07, color);
            painter.add(Shape::line(arc(0.5, 0.78, 0.24, 225.0, 315.0), sk));
            painter.add(Shape::line(arc(0.5, 0.78, 0.44, 225.0, 315.0), sk));
        }
        Icon::Tray => {
            painter.add(Shape::line(
                vec![
                    pt(0.08, 0.58),
                    pt(0.08, 0.88),
                    pt(0.92, 0.88),
                    pt(0.92, 0.58),
                ],
                sk,
            ));
            painter.add(Shape::line(
                vec![
                    pt(0.08, 0.6),
                    pt(0.32, 0.6),
                    pt(0.38, 0.7),
                    pt(0.62, 0.7),
                    pt(0.68, 0.6),
                    pt(0.92, 0.6),
                ],
                sk,
            ));
            painter.line_segment([pt(0.5, 0.08), pt(0.5, 0.5)], sk);
            painter.add(Shape::line(
                vec![pt(0.32, 0.26), pt(0.5, 0.08), pt(0.68, 0.26)],
                sk,
            ));
        }
        Icon::Shield => {
            let mut pts = vec![pt(0.5, 0.06), pt(0.86, 0.2), pt(0.86, 0.48)];
            pts.extend(
                arc(0.5, 0.48, 0.36, 0.0, 90.0)
                    .into_iter()
                    .map(|q| pos2(q.x, q.y + (q.y - pt(0.5, 0.48).y) * 0.25)),
            );
            pts.extend(
                arc(0.5, 0.48, 0.36, 90.0, 180.0)
                    .into_iter()
                    .map(|q| pos2(q.x, q.y + (q.y - pt(0.5, 0.48).y) * 0.25)),
            );
            pts.extend([pt(0.14, 0.2), pt(0.5, 0.06)]);
            painter.add(Shape::line(pts, sk));
            painter.add(Shape::line(
                vec![pt(0.34, 0.5), pt(0.46, 0.62), pt(0.68, 0.38)],
                sk,
            ));
        }
        Icon::Info => {
            painter.circle_stroke(pt(0.5, 0.5), s * 0.42, sk);
            painter.circle_filled(pt(0.5, 0.3), w * 0.75, color);
            painter.line_segment([pt(0.5, 0.44), pt(0.5, 0.72)], st(w * 1.1, color));
        }
        Icon::Swatch => {
            painter.circle_stroke(pt(0.5, 0.5), s * 0.4, sk);
            let pts = arc(0.5, 0.5, 0.4, -90.0, 90.0);
            let mut fan = vec![pt(0.5, 0.5)];
            fan.extend(pts);
            painter.add(Shape::convex_polygon(fan, color, Stroke::NONE));
        }
    }
}
