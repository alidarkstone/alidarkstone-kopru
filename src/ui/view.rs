//! Ekranlar: kenar çubuğu, Cihazlar / Paylaşım / Dosyalar / Ayarlar ve eşleştirme sayfası.

use egui::{pos2, vec2, Align, Align2, Color32, Id, Layout, Margin, Order, Rect, Sense, Ui};

use super::theme::{self, st, Palette};
use super::widgets::{self as w, Btn, Icon};
use super::{Tab, ThemeChoice, UiPrefs};
use crate::state::{Action, LinkPhase, TransferDir, TransferStatus, TransferView, UiState};

const SIDEBAR_W: f32 = 216.0;
const CONTENT_MAX_W: f32 = 620.0;
const NAME_MAX: usize = 40;

/// Bir kareyi çizer. Ağ/özellik koduna dokunmaz; kullanıcının istekleri `out`'a eklenir.
/// Hiçbir `state` değeri için panik etmez.
pub fn draw(ctx: &egui::Context, state: &UiState, out: &mut Vec<Action>, prefs: &mut UiPrefs) {
    theme::install(ctx);
    theme::refresh_font_readiness(ctx);
    prefs.theme.apply(ctx);
    let p = Palette::for_theme(ctx.theme());

    handle_drops(ctx, state, out, prefs);

    egui::SidePanel::left("kopru-sidebar")
        .exact_width(SIDEBAR_W)
        .resizable(false)
        .frame(egui::Frame::none().fill(p.sidebar).inner_margin(Margin {
            left: 10.0,
            right: 10.0,
            top: 16.0 + prefs.titlebar_inset,
            bottom: 12.0,
        }))
        .show(ctx, |ui| sidebar(ui, &p, state, prefs));

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(p.bg))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let full = ui.available_width();
                    let cw = (full - 56.0).clamp(200.0, CONTENT_MAX_W);
                    let side = ((full - cw) / 2.0).max(0.0);
                    ui.add_space(26.0 + prefs.titlebar_inset);
                    ui.horizontal(|ui| {
                        ui.add_space(side);
                        ui.vertical(|ui| {
                            ui.set_width(cw);
                            match prefs.tab {
                                Tab::Devices => devices_page(ui, &p, state, out, prefs),
                                Tab::Clipboard => clipboard_page(ui, &p, state, out),
                                Tab::Files => files_page(ui, &p, state, out),
                                Tab::Settings => settings_page(ui, &p, state, out, prefs),
                            }
                            ui.add_space(12.0);
                        });
                    });
                });
        });

    if let LinkPhase::Pairing { sas, mine, theirs } = &state.phase {
        pairing_sheet(ctx, &p, state, sas, *mine, *theirs, out);
    }

    drop_overlay(ctx, &p, state);

    // Durum dışarıdan çekildiği için ara sıra yeniden çiz; etkin işlerde daha sık.
    let busy = matches!(
        state.phase,
        LinkPhase::Connecting | LinkPhase::Checking | LinkPhase::Pairing { .. }
    ) || state.transfers.iter().any(|t| is_active(t.status));
    ctx.request_repaint_after(std::time::Duration::from_millis(if busy {
        120
    } else {
        1000
    }));
}

// ───────────────────────────── yardımcılar ─────────────────────────────

fn is_active(s: TransferStatus) -> bool {
    matches!(s, TransferStatus::Sending | TransferStatus::Receiving)
}

fn connected(state: &UiState) -> bool {
    state.phase == LinkPhase::Connected
}

fn peer_name(state: &UiState) -> String {
    state
        .peer
        .as_ref()
        .map(|p| p.name.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Diğer bilgisayar".to_owned())
}

/// Ondalık (Apple gibi 1000 tabanlı) boyut; Türkçe ondalık virgülü.
pub fn fmt_size(b: u64) -> String {
    const U: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if b < 1000 {
        return format!("{b} bayt");
    }
    let mut v = b as f64 / 1000.0;
    let mut i = 0;
    while v >= 999.95 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if v < 10.0 {
        format!("{v:.1} {}", U[i]).replace('.', ",")
    } else {
        format!("{} {}", v.round() as u64, U[i])
    }
}

fn frac(t: &TransferView) -> f32 {
    if t.size == 0 {
        if matches!(t.status, TransferStatus::Sent | TransferStatus::Received) {
            1.0
        } else {
            0.0
        }
    } else {
        (t.done.min(t.size) as f64 / t.size as f64) as f32
    }
}

fn device_icon(name: &str) -> Icon {
    let n = name.to_lowercase();
    if [
        "macbook",
        "laptop",
        "dizüstü",
        "notebook",
        "air",
        "thinkpad",
        "surface",
    ]
    .iter()
    .any(|k| n.contains(k))
    {
        Icon::Laptop
    } else {
        Icon::Desktop
    }
}

fn page_header(ui: &mut Ui, p: &Palette, title: &str, sub: &str) {
    ui.label(w::text(title, theme::semibold(24.0), p.text));
    ui.add_space(2.0);
    w::para(ui, sub, theme::regular(13.0), p.text2);
    ui.add_space(20.0);
}

fn banner(ui: &mut Ui, p: &Palette, msg: &str) {
    egui::Frame::none()
        .fill(p.red_soft)
        .rounding(10.0)
        .inner_margin(Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                w::paint_icon(ui.painter(), r, Icon::Info, p.red);
                w::para(ui, msg, theme::regular(12.5), p.text);
            });
        });
    ui.add_space(16.0);
}

/// Bırakılan dosyaları gönder; kullanıcıyı Dosyalar'a götür.
fn handle_drops(ctx: &egui::Context, _state: &UiState, out: &mut Vec<Action>, prefs: &mut UiPrefs) {
    let paths: Vec<std::path::PathBuf> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .filter_map(|f| f.path.clone())
            .collect()
    });
    if !paths.is_empty() {
        out.push(Action::SendPaths(paths));
        prefs.tab = Tab::Files;
    }
}

fn drop_overlay(ctx: &egui::Context, p: &Palette, state: &UiState) {
    let n = ctx.input(|i| i.raw.hovered_files.len());
    if n == 0 {
        return;
    }
    let screen = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("kopru-drop")));
    painter.rect_filled(screen, 0.0, p.bg.gamma_multiply(0.95));
    let zone = screen.shrink(18.0);
    painter.rect_filled(zone, 16.0, p.accent_soft);
    w::dashed_round_rect(&painter, zone, 16.0, st(2.0, p.accent));
    let c = zone.center();
    w::paint_icon(
        &painter,
        Rect::from_center_size(c - vec2(0.0, 38.0), vec2(44.0, 44.0)),
        Icon::Tray,
        p.accent,
    );
    let (title, sub) = if connected(state) {
        (
            if n == 1 {
                "Göndermek için bırakın".to_owned()
            } else {
                format!("{n} öğeyi göndermek için bırakın")
            },
            format!("{} alıcıya gidecek", peer_name(state)),
        )
    } else {
        (
            "Göndermek için bırakın".to_owned(),
            "Bağlantı kurulunca gönderilir".to_owned(),
        )
    };
    painter.text(
        c + vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        title,
        theme::semibold(17.0),
        p.text,
    );
    painter.text(
        c + vec2(0.0, 32.0),
        Align2::CENTER_CENTER,
        sub,
        theme::regular(13.0),
        p.text2,
    );
}

// ───────────────────────────── kenar çubuğu ─────────────────────────────

fn status_of(state: &UiState, p: &Palette) -> (Color32, String) {
    match &state.phase {
        LinkPhase::Idle => (p.text3, "Bağlı değil".into()),
        LinkPhase::Connecting => (p.orange, "Bağlanıyor…".into()),
        LinkPhase::Checking => (p.orange, "Doğrulanıyor…".into()),
        LinkPhase::Pairing { .. } => (p.accent, "Eşleştirme onayı".into()),
        LinkPhase::Connected => (p.green, peer_name(state)),
    }
}

fn sidebar(ui: &mut Ui, p: &Palette, state: &UiState, prefs: &mut UiPrefs) {
    // Uygulama kimliği + bağlantı durumu
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        w::tile(ui, p.accent, Icon::Bridge, 34.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(w::text("Köprü", theme::semibold(15.0), p.text));
            ui.horizontal(|ui| {
                let (color, label) = status_of(state, p);
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                w::status_dot(ui.painter(), r.center(), color, connected(state));
                ui.spacing_mut().item_spacing.x = 4.0;
                w::line(ui, label, theme::regular(11.5), p.text2);
            });
        });
    });
    ui.add_space(18.0);

    let offered = state
        .transfers
        .iter()
        .filter(|t| t.status == TransferStatus::Offered && t.dir == TransferDir::In)
        .count();
    let active = state
        .transfers
        .iter()
        .filter(|t| is_active(t.status))
        .count();
    let items: [(Tab, &str, Icon, Color32); 4] = [
        (Tab::Devices, "Cihazlar", Icon::Laptop, p.accent),
        (Tab::Clipboard, "Paylaşım", Icon::Clipboard, p.orange),
        (
            Tab::Files,
            "Dosyalar",
            Icon::Folder,
            Color32::from_rgb(0x5E, 0x5C, 0xE6),
        ),
        (
            Tab::Settings,
            "Ayarlar",
            Icon::Gear,
            Color32::from_rgb(0x8E, 0x8E, 0x93),
        ),
    ];
    for (tab, label, icon, color) in items {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        let sel = prefs.tab == tab;
        if sel {
            ui.painter().rect_filled(rect, 7.0, p.selected);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 7.0, p.selected.gamma_multiply(0.45));
        }
        let tr = Rect::from_center_size(
            pos2(rect.left() + 8.0 + 11.0, rect.center().y),
            vec2(22.0, 22.0),
        );
        w::paint_tile(ui.painter(), tr, color, icon);
        ui.painter().text(
            pos2(tr.right() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            if sel {
                theme::semibold(13.5)
            } else {
                theme::regular(13.5)
            },
            p.text,
        );
        let rc = pos2(rect.right() - 8.0, rect.center().y);
        if tab == Tab::Files && offered > 0 {
            w::badge(ui.painter(), rc, offered, p.red, Color32::WHITE);
        } else if tab == Tab::Files && active > 0 {
            w::badge(ui.painter(), rc, active, p.fill_active, p.text);
        }
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            prefs.tab = tab;
        }
        ui.add_space(2.0);
    }

    // Alt: bu bilgisayar (alt kenara sabit)
    let mr = ui.max_rect();
    let foot = Rect::from_min_max(
        pos2(mr.left() + 6.0, mr.bottom() - 50.0),
        pos2(mr.right() - 4.0, mr.bottom()),
    );
    if foot.top() > ui.cursor().top() + 8.0 {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(foot), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            w::line(ui, "Bu bilgisayar", theme::regular(11.0), p.text3);
            let name = if state.my_name.trim().is_empty() {
                "Adsız"
            } else {
                state.my_name.trim()
            };
            w::line(ui, name, theme::semibold(12.5), p.text);
            w::line(ui, &state.my_id, theme::regular(11.0), p.text2).on_hover_text("Cihaz kimliği");
        });
    }
}

// ───────────────────────────── Cihazlar ─────────────────────────────

fn devices_page(
    ui: &mut Ui,
    p: &Palette,
    state: &UiState,
    out: &mut Vec<Action>,
    prefs: &mut UiPrefs,
) {
    page_header(
        ui,
        p,
        "Cihazlar",
        "Aynı ağdaki bir bilgisayarla eşleşin. Pano ve dosyalar uçtan uca şifrelenir.",
    );
    if !state.last_error.trim().is_empty() {
        banner(ui, p, state.last_error.trim());
    }

    w::section(ui, p, Some("Bağlantı"), None, |s| {
        let p = s.p;
        match &state.phase {
            LinkPhase::Connected => {
                let peer = state.peer.clone();
                let name = peer_name(state);
                let addr = peer.as_ref().map(|x| x.address.clone()).unwrap_or_default();
                s.row(
                    |ui| {
                        w::tile(ui, p.green, device_icon(&name), 30.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            w::line(ui, &name, theme::semibold(13.5), p.text);
                            let sub = if addr.is_empty() {
                                "Bağlı · uçtan uca şifreli".to_owned()
                            } else {
                                format!("Bağlı · {addr} · uçtan uca şifreli")
                            };
                            w::line(ui, sub, theme::regular(11.5), p.text2);
                        });
                    },
                    |ui| {
                        if Btn::secondary("Bağlantıyı kes").show(ui, &p).clicked() {
                            out.push(Action::Disconnect);
                        }
                    },
                );
            }
            LinkPhase::Connecting | LinkPhase::Checking | LinkPhase::Pairing { .. } => {
                let label = match &state.phase {
                    LinkPhase::Connecting => "Bağlanıyor…",
                    LinkPhase::Checking => "Kimlik doğrulanıyor…",
                    _ => "Eşleştirme kodu onayı bekleniyor",
                };
                let sub = state.peer.as_ref().map(|x| {
                    if x.address.is_empty() {
                        x.name.clone()
                    } else {
                        format!("{} · {}", x.name, x.address)
                    }
                });
                s.row(
                    |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                        ui.painter().rect_filled(r, 7.5, p.fill);
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(Rect::from_center_size(r.center(), vec2(16.0, 16.0))),
                        );
                        w::spinner(&mut child, p.text2, 16.0);
                        w::title_sub(ui, &p, label, sub.as_deref());
                    },
                    |ui| {
                        if Btn::secondary("Vazgeç").show(ui, &p).clicked() {
                            out.push(Action::Disconnect);
                        }
                    },
                );
            }
            LinkPhase::Idle => {
                s.empty(
                    Icon::Laptop,
                    "Bağlı değil",
                    "Aşağıdaki listeden bir cihaz seçin ya da adresini girerek bağlanın.",
                );
            }
        }
    });

    let busy = matches!(
        state.phase,
        LinkPhase::Connecting | LinkPhase::Checking | LinkPhase::Pairing { .. }
    );
    let footer = if state.devices.is_empty() {
        Some("Diğer bilgisayarda Köprü'nün açık olduğundan ve iki cihazın aynı Wi‑Fi ya da kablolu ağda bulunduğundan emin olun.")
    } else {
        None
    };
    w::section(ui, p, Some("Yakındaki cihazlar"), footer, |s| {
        let p = s.p;
        if state.devices.is_empty() {
            s.custom(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    w::spinner(ui, p.text2, 14.0);
                    ui.add_space(4.0);
                    ui.label(w::text("Cihaz aranıyor…", theme::regular(13.0), p.text2));
                });
            });
            return;
        }
        for d in &state.devices {
            let is_peer = connected(state) && state.peer.as_ref().is_some_and(|x| x.id == d.id);
            let name = if d.name.trim().is_empty() {
                "Adsız cihaz"
            } else {
                d.name.trim()
            };
            let mut sub = d.address.clone();
            if d.trusted {
                sub.push_str(if sub.is_empty() {
                    "Güvenilir"
                } else {
                    " · Güvenilir"
                });
            }
            s.row_inset(
                54.0,
                |ui| {
                    w::tile(
                        ui,
                        if d.trusted {
                            p.accent
                        } else {
                            Color32::from_rgb(0x8E, 0x8E, 0x93)
                        },
                        device_icon(name),
                        30.0,
                    );
                    w::title_sub(ui, &p, name, Some(&sub));
                },
                |ui| {
                    if is_peer {
                        ui.label(w::text("Bağlı", theme::regular(12.5), p.text2));
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        w::paint_icon(ui.painter(), r, Icon::Check, p.green);
                    } else if Btn::secondary("Bağlan")
                        .enabled(!busy)
                        .show(ui, &p)
                        .clicked()
                    {
                        out.push(Action::Connect(d.id.clone()));
                    }
                },
            );
        }
    });

    w::section(
        ui,
        p,
        Some("Adresle bağlan"),
        Some("Cihaz listede görünmüyorsa diğer bilgisayarın IP adresini yazın (ör. 192.168.1.20 ya da 192.168.1.20:47910)."),
        |s| {
            let p = s.p;
            let submit = std::cell::Cell::new(false);
            let ok = !prefs.address.trim().is_empty() && !busy;
            s.row(
                |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut prefs.address)
                            .hint_text("IP adresi veya ad:port")
                            .desired_width(ui.available_width())
                            .margin(vec2(8.0, 5.0))
                            .char_limit(255),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit.set(true);
                    }
                },
                |ui| {
                    if Btn::primary("Bağlan").enabled(ok).show(ui, &p).clicked() {
                        submit.set(true);
                    }
                },
            );
            if submit.get() && !busy {
                let a = prefs.address.trim().to_owned();
                if !a.is_empty() {
                    out.push(Action::ConnectAddress(a));
                }
            }
        },
    );
}

// ───────────────────────────── Paylaşım (pano) ─────────────────────────────

fn clipboard_page(ui: &mut Ui, p: &Palette, state: &UiState, out: &mut Vec<Action>) {
    page_header(
        ui,
        p,
        "Paylaşım",
        "Bir bilgisayarda kopyaladığınız metin, diğerinde yapıştırılmaya hazır olur.",
    );

    w::section(ui, p, None, None, |s| {
        let p = s.p;
        s.row_inset(
            54.0,
            |ui| {
                w::tile(ui, p.orange, Icon::Clipboard, 30.0);
                w::title_sub(
                    ui,
                    &p,
                    "Panoyu paylaş",
                    Some("Kopyalanan metin otomatik olarak gönderilir"),
                );
            },
            |ui| {
                let mut on = state.clipboard_enabled;
                if w::switch(ui, &p, &mut on).changed() {
                    out.push(Action::SetClipboard(on));
                }
            },
        );
        let (color, msg) = match (state.clipboard_enabled, connected(state)) {
            (true, true) => (
                p.green,
                format!("Etkin — {} ile eşitleniyor", peer_name(state)),
            ),
            (true, false) => (p.orange, "Bir cihaza bağlanınca başlar".to_owned()),
            (false, _) => (p.text3, "Kapalı — pano hiçbir yere gönderilmez".to_owned()),
        };
        s.row_inset(
            54.0,
            |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(30.0, 20.0), Sense::hover());
                w::status_dot(
                    ui.painter(),
                    r.center(),
                    color,
                    state.clipboard_enabled && connected(state),
                );
                w::line(ui, msg, theme::regular(13.0), p.text);
            },
            |_| {},
        );
    });

    w::section(ui, p, Some("Gizlilik"), None, |s| {
        let p = s.p;
        let rows: [(Icon, &str, &str); 2] = [
            (
                Icon::Lock,
                "Uçtan uca şifreli",
                "Yalnızca eşleştirdiğiniz bilgisayar okuyabilir",
            ),
            (
                Icon::Shield,
                "Yalnızca yerel ağ",
                "İnternete ya da bir sunucuya hiçbir şey gönderilmez",
            ),
        ];
        for (icon, t, sub) in rows {
            s.row_inset(
                54.0,
                |ui| {
                    w::tile(ui, p.green, icon, 30.0);
                    w::title_sub(ui, &p, t, Some(sub));
                },
                |_| {},
            );
        }
    });
}

// ───────────────────────────── Dosyalar ─────────────────────────────

fn files_page(ui: &mut Ui, p: &Palette, state: &UiState, out: &mut Vec<Action>) {
    page_header(
        ui,
        p,
        "Dosyalar",
        "Dosyaları pencereye sürükleyip bırakın ya da seçin.",
    );

    // Bırakma alanı
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::click());
    let hot = hovering || resp.hovered();
    ui.painter().rect_filled(
        rect,
        theme::R_CARD,
        if hot { p.accent_soft } else { p.card },
    );
    w::dashed_round_rect(
        ui.painter(),
        rect.shrink(0.5),
        theme::R_CARD,
        st(1.2, if hot { p.accent } else { p.text3 }),
    );
    let c = rect.center();
    w::paint_icon(
        ui.painter(),
        Rect::from_center_size(c - vec2(0.0, 32.0), vec2(30.0, 30.0)),
        Icon::Tray,
        if hot { p.accent } else { p.text2 },
    );
    ui.painter().text(
        c + vec2(0.0, 4.0),
        Align2::CENTER_CENTER,
        "Dosyaları buraya bırakın",
        theme::semibold(14.0),
        p.text,
    );
    let hint = if connected(state) {
        format!(
            "veya seçmek için tıklayın — {} alıcıya gider",
            peer_name(state)
        )
    } else {
        "Göndermek için önce bir cihaza bağlanın".to_owned()
    };
    let hint_galley = ui
        .painter()
        .layout(hint, theme::regular(12.5), p.text2, rect.width() - 32.0);
    ui.painter().galley(
        pos2(c.x - hint_galley.size().x / 2.0, c.y + 18.0),
        hint_galley,
        p.text2,
    );
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        out.push(Action::PickFiles);
    }
    ui.add_space(22.0);

    w::section(ui, p, Some("Aktarımlar"), None, |s| {
        if state.transfers.is_empty() {
            s.empty(
                Icon::Doc,
                "Henüz aktarım yok",
                "Gönderdiğiniz ve aldığınız dosyalar burada görünür.",
            );
            return;
        }
        for t in &state.transfers {
            transfer_row(s, t, out);
        }
    });

    w::section(
        ui,
        p,
        Some("Kaydetme yeri"),
        Some("Gelen dosyalar bu klasöre kaydedilir."),
        |s| {
            let p = s.p;
            let path = state.download_dir.display().to_string();
            s.row_inset(
                54.0,
                |ui| {
                    w::tile(ui, Color32::from_rgb(0x5E, 0x5C, 0xE6), Icon::Folder, 30.0);
                    w::line(ui, path.clone(), theme::regular(12.5), p.text)
                        .on_hover_text(path.as_str());
                },
                |ui| {
                    if Btn::secondary("Değiştir…").show(ui, &p).clicked() {
                        out.push(Action::ChooseDownloadDir);
                    }
                    if Btn::secondary("Aç").show(ui, &p).clicked() {
                        out.push(Action::OpenDownloads);
                    }
                },
            );
        },
    );
}

fn transfer_status_text(t: &TransferView) -> String {
    let pct = (frac(t) * 100.0).floor() as u32;
    match t.status {
        TransferStatus::Waiting => {
            format!("{} · Karşı tarafın kabulü bekleniyor", fmt_size(t.size))
        }
        TransferStatus::Sending => format!(
            "Gönderiliyor · {} / {} · %{pct}",
            fmt_size(t.done.min(t.size)),
            fmt_size(t.size)
        ),
        TransferStatus::Receiving => format!(
            "Alınıyor · {} / {} · %{pct}",
            fmt_size(t.done.min(t.size)),
            fmt_size(t.size)
        ),
        TransferStatus::Sent => format!("{} · Gönderildi", fmt_size(t.size)),
        TransferStatus::Received => format!("{} · Alındı", fmt_size(t.size)),
        TransferStatus::Offered => format!("{} · Almak ister misiniz?", fmt_size(t.size)),
        TransferStatus::Rejected => "Reddedildi".to_owned(),
        TransferStatus::Cancelled => "İptal edildi".to_owned(),
        TransferStatus::Error => "Aktarılamadı".to_owned(),
    }
}

fn transfer_row(s: &mut w::Section, t: &TransferView, out: &mut Vec<Action>) {
    let p = s.p;
    let name = if t.name.trim().is_empty() {
        "Adsız dosya"
    } else {
        t.name.trim()
    };
    let incoming = t.dir == TransferDir::In;
    let tile_color = match t.status {
        TransferStatus::Error | TransferStatus::Rejected | TransferStatus::Cancelled => {
            Color32::from_rgb(0x8E, 0x8E, 0x93)
        }
        _ if incoming => p.green,
        _ => p.accent,
    };
    let sub_color = match t.status {
        TransferStatus::Error => p.red,
        TransferStatus::Rejected | TransferStatus::Cancelled => p.text3,
        _ => p.text2,
    };
    let show_bar = is_active(t.status);
    s.row_inset(
        54.0,
        |ui| {
            w::tile(
                ui,
                tile_color,
                if incoming {
                    Icon::ArrowDown
                } else {
                    Icon::ArrowUp
                },
                30.0,
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                w::line(ui, name, theme::regular(13.5), p.text).on_hover_text(name);
                w::line(ui, transfer_status_text(t), theme::regular(11.5), sub_color);
                if show_bar {
                    ui.add_space(3.0);
                    w::progress(ui, &p, frac(t), if incoming { p.green } else { p.accent });
                }
            });
        },
        |ui| match t.status {
            TransferStatus::Offered if incoming => {
                if Btn::primary("Kabul et").show(ui, &p).clicked() {
                    out.push(Action::FileAccept(t.id.clone()));
                }
                if Btn::secondary("Reddet").show(ui, &p).clicked() {
                    out.push(Action::FileReject(t.id.clone()));
                }
            }
            TransferStatus::Offered
            | TransferStatus::Waiting
            | TransferStatus::Sending
            | TransferStatus::Receiving => {
                if w::icon_button(ui, &p, Icon::Xmark, "İptal et").clicked() {
                    out.push(Action::FileCancel(t.id.clone()));
                }
            }
            TransferStatus::Received => {
                let label = if cfg!(target_os = "macos") {
                    "Finder'da göster"
                } else {
                    "Klasörde göster"
                };
                if Btn::plain(label).show(ui, &p).clicked() {
                    out.push(Action::FileShow(t.id.clone()));
                }
            }
            TransferStatus::Sent => {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                w::paint_icon(ui.painter(), r, Icon::Check, p.green);
            }
            TransferStatus::Rejected | TransferStatus::Cancelled | TransferStatus::Error => {}
        },
    );
}

// ───────────────────────────── Ayarlar ─────────────────────────────

fn settings_page(
    ui: &mut Ui,
    p: &Palette,
    state: &UiState,
    out: &mut Vec<Action>,
    prefs: &mut UiPrefs,
) {
    page_header(
        ui,
        p,
        "Ayarlar",
        "Bu bilgisayarın adı, görünüm ve güvenilen cihazlar.",
    );

    w::section(
        ui,
        p,
        Some("Bu bilgisayar"),
        Some("Ad, diğer bilgisayarın cihaz listesinde görünür."),
        |s| {
            let p = s.p;
            s.row(
                |ui| {
                    w::line(ui, "Ad", theme::regular(13.5), p.text);
                },
                |ui| {
                    let buf = prefs.name_edit.get_or_insert_with(|| state.my_name.clone());
                    let r = ui.add(
                        egui::TextEdit::singleline(buf)
                            .desired_width(220.0)
                            .margin(vec2(8.0, 5.0))
                            .char_limit(NAME_MAX)
                            .horizontal_align(Align::RIGHT),
                    );
                    if r.lost_focus() {
                        let v = buf.trim().to_owned();
                        if !v.is_empty() && v != state.my_name {
                            out.push(Action::SetName(v));
                        }
                        prefs.name_edit = None;
                    } else if !r.has_focus() {
                        // odakta değilken her zaman güncel adı göster
                        prefs.name_edit = None;
                    }
                },
            );
            s.row(
                |ui| {
                    w::line(ui, "Cihaz kimliği", theme::regular(13.5), p.text);
                },
                |ui| {
                    ui.add(
                        egui::Label::new(w::text(
                            &state.my_id,
                            egui::FontId::monospace(12.0),
                            p.text2,
                        ))
                        .truncate(),
                    );
                },
            );
        },
    );

    w::section(ui, p, Some("Görünüm"), None, |s| {
        let p = s.p;
        s.row(
            |ui| {
                w::line(ui, "Tema", theme::regular(13.5), p.text);
            },
            |ui| {
                let mut i = match prefs.theme {
                    ThemeChoice::System => 0,
                    ThemeChoice::Light => 1,
                    ThemeChoice::Dark => 2,
                };
                if w::segmented(ui, &p, &mut i, &["Sistem", "Açık", "Koyu"]) {
                    prefs.theme = match i {
                        1 => ThemeChoice::Light,
                        2 => ThemeChoice::Dark,
                        _ => ThemeChoice::System,
                    };
                }
            },
        );
    });

    w::section(ui, p, Some("Pano"), None, |s| {
        let p = s.p;
        s.row(
            |ui| {
                w::title_sub(
                    ui,
                    &p,
                    "Panoyu paylaş",
                    Some("Kopyalanan metni bağlı bilgisayara gönder"),
                );
            },
            |ui| {
                let mut on = state.clipboard_enabled;
                if w::switch(ui, &p, &mut on).changed() {
                    out.push(Action::SetClipboard(on));
                }
            },
        );
    });

    w::section(
        ui,
        p,
        Some("Güvenilen cihazlar"),
        Some("Unutulan cihaz bir sonraki bağlantıda yeniden eşleştirme kodu ister."),
        |s| {
            let p = s.p;
            if state.trusted.is_empty() {
                s.empty(
                    Icon::Shield,
                    "Güvenilen cihaz yok",
                    "Bir cihazla eşleştiğinizde burada listelenir.",
                );
                return;
            }
            for t in &state.trusted {
                let name = if t.name.trim().is_empty() {
                    "Adsız cihaz"
                } else {
                    t.name.trim()
                };
                let sub = if t.since.is_empty() {
                    String::new()
                } else {
                    format!("Eşleşme: {}", t.since)
                };
                s.row_inset(
                    54.0,
                    |ui| {
                        w::tile(ui, p.accent, device_icon(name), 30.0);
                        w::title_sub(ui, &p, name, if sub.is_empty() { None } else { Some(&sub) });
                    },
                    |ui| {
                        if Btn::destructive("Unut").show(ui, &p).clicked() {
                            out.push(Action::Forget(t.id.clone()));
                        }
                    },
                );
            }
        },
    );

    w::section(ui, p, Some("Hakkında"), None, |s| {
        let p = s.p;
        s.row_inset(
            54.0,
            |ui| {
                w::tile(ui, p.accent, Icon::Bridge, 30.0);
                w::title_sub(
                    ui,
                    &p,
                    &format!("Köprü {}", env!("CARGO_PKG_VERSION")),
                    Some("Uçtan uca şifreli · yalnızca yerel ağ · telemetri yok"),
                );
            },
            |_| {},
        );
    });
}

// ───────────────────────────── eşleştirme sayfası ─────────────────────────────

fn pairing_sheet(
    ctx: &egui::Context,
    p: &Palette,
    state: &UiState,
    sas: &str,
    mine: Option<bool>,
    theirs: Option<bool>,
    out: &mut Vec<Action>,
) {
    let screen = ctx.screen_rect();
    // Karartma: alttaki arayüze tıklanmasını engeller.
    egui::Area::new(Id::new("kopru-scrim"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, p.scrim);
        });

    let digits: Vec<char> = sas
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(12)
        .collect();
    let peer = peer_name(state);
    egui::Area::new(Id::new("kopru-pairing"))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -10.0))
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(p.card)
                .rounding(theme::R_SHEET)
                .stroke(st(1.0, p.hairline))
                .shadow(egui::epaint::Shadow { offset: vec2(0.0, 18.0), blur: 48.0, spread: 0.0, color: p.shadow })
                .inner_margin(Margin { left: 28.0, right: 28.0, top: 26.0, bottom: 22.0 })
                .show(ui, |ui| {
                    let width = 360.0_f32.min(screen.width() - 96.0).max(220.0);
                    ui.set_width(width);
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 24.0, p.accent_soft);
                        w::paint_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(22.0, 22.0)), Icon::Lock, p.accent);
                        ui.add_space(12.0);
                        ui.label(w::text("Eşleştirme kodunu karşılaştırın", theme::semibold(17.0), p.text));
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(w::text(
                                format!("{peer} ekranında da aynı kod görünüyorsa eşleştirin. Kodlar farklıysa iptal edin."),
                                theme::regular(12.5),
                                p.text2,
                            ))
                            .wrap()
                            .halign(Align::Center),
                        );
                        ui.add_space(18.0);
                        code_cells(ui, p, &digits);
                        ui.add_space(14.0);

                        let (icon, color, msg): (Option<Icon>, Color32, String) = match (mine, theirs) {
                            (_, Some(false)) => (Some(Icon::Xmark), p.red, format!("{peer} eşleştirmeyi reddetti")),
                            (Some(true), Some(true)) => (Some(Icon::Check), p.green, "İki taraf da onayladı".into()),
                            (Some(true), _) => (None, p.text2, format!("{peer} onayı bekleniyor…")),
                            (Some(false), _) => (Some(Icon::Xmark), p.text2, "İptal edildi".into()),
                            (None, Some(true)) => (Some(Icon::Check), p.green, format!("{peer} kodu onayladı")),
                            (None, None) => (None, p.text3, String::new()),
                        };
                        if !msg.is_empty() {
                        ui.allocate_ui_with_layout(vec2(width, 18.0), Layout::left_to_right(Align::Center), |ui| {
                            let gw = ui.painter().layout_no_wrap(msg.clone(), theme::regular(12.5), color).size().x + 20.0;
                            ui.add_space(((width - gw) / 2.0).max(0.0));
                            ui.spacing_mut().item_spacing.x = 6.0;
                            match icon {
                                Some(i) => {
                                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                    w::paint_icon(ui.painter(), r, i, color);
                                }
                                None => {
                                    w::spinner(ui, p.text2, 14.0);
                                }
                            }
                            ui.label(w::text(msg, theme::regular(12.5), color));
                        });
                        ui.add_space(4.0);
                        }
                        ui.add_space(16.0);

                        let decided = mine.is_some();
                        let bw = ((width - 10.0) / 2.0).max(60.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            if Btn::secondary("Farklı, iptal").large().min_width(bw).enabled(mine != Some(false)).show(ui, p).clicked() {
                                out.push(Action::ConfirmPairing(false));
                            }
                            if Btn::primary("Aynı, eşleştir").large().min_width(bw).enabled(!decided).show(ui, p).clicked() {
                                out.push(Action::ConfirmPairing(true));
                            }
                        });
                    });
                });
        });

    // Esc ile iptal (Enter bilerek atanmadı: kod okunmadan onaylanmasın).
    if mine.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        out.push(Action::ConfirmPairing(false));
    }
}

/// 6 haneli kod: her hane kendi kutusunda, 3+3 gruplu; sabit genişlikli rakamlar.
fn code_cells(ui: &mut Ui, p: &Palette, digits: &[char]) {
    let n = digits.len().max(1);
    let gap = 6.0;
    let group_gap = if n == 6 { 14.0 } else { 0.0 };
    let avail = ui.available_width();
    let cell_w = ((avail - gap * (n as f32 - 1.0) - group_gap) / n as f32).clamp(18.0, 44.0);
    let cell_h = (cell_w * 1.25).max(28.0);
    let total = cell_w * n as f32 + gap * (n as f32 - 1.0) + group_gap;
    let (rect, _) = ui.allocate_exact_size(vec2(total.min(avail), cell_h), Sense::hover());
    let font = theme::semibold((cell_w * 0.72).clamp(14.0, 30.0));
    let mut x = rect.left();
    for i in 0..n {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(cell_w, cell_h));
        ui.painter().rect_filled(r, 9.0, p.fill);
        let ch = digits
            .get(i)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "–".into());
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, ch, font.clone(), p.text);
        x += cell_w + gap;
        if n == 6 && i == 2 {
            x += group_gap;
        }
    }
}
