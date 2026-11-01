//! Başsız arayüz testleri: `draw` çeşitli (uç) durumlarda panik etmemeli.
//! `KOPRU_UI_SHOTS=<klasör> cargo test ui::tests::screenshots -- --nocapture` ekran görüntülerini PNG olarak yazar.

use std::path::PathBuf;

use super::*;
use crate::state::*;

struct Run {
    ctx: egui::Context,
    prefs: UiPrefs,
    time: f64,
}

impl Run {
    fn new(prefs: UiPrefs) -> Self {
        Run {
            ctx: egui::Context::default(),
            prefs,
            time: 0.0,
        }
    }

    fn frame(
        &mut self,
        state: &UiState,
        size: egui::Vec2,
        ppp: f32,
        tweak: impl FnOnce(&mut egui::RawInput),
    ) -> (egui::FullOutput, Vec<Action>) {
        self.time += 0.4;
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(self.time),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        tweak(&mut raw);
        let mut out = Vec::new();
        let prefs = &mut self.prefs;
        let full = self.ctx.run(raw, |ctx| draw(ctx, state, &mut out, prefs));
        (full, out)
    }

    fn frames(&mut self, state: &UiState, size: egui::Vec2, n: usize) -> Vec<Action> {
        let mut all = Vec::new();
        for _ in 0..n {
            all.extend(self.frame(state, size, 1.0, |_| {}).1);
        }
        all
    }
}

fn tr(
    id: &str,
    dir: TransferDir,
    name: &str,
    size: u64,
    done: u64,
    status: TransferStatus,
) -> TransferView {
    TransferView {
        id: id.into(),
        dir,
        name: name.into(),
        size,
        done,
        status,
        path: None,
    }
}

fn idle_empty() -> UiState {
    UiState {
        phase: LinkPhase::Idle,
        peer: None,
        devices: vec![],
        trusted: vec![],
        transfers: vec![],
        ..UiState::demo()
    }
}

fn pairing(sas: &str, mine: Option<bool>, theirs: Option<bool>) -> UiState {
    UiState {
        phase: LinkPhase::Pairing {
            sas: sas.into(),
            mine,
            theirs,
        },
        ..UiState::demo()
    }
}

fn long_names() -> UiState {
    let long = "Çok-uzun-bir-bilgisayar-adı-ĞÜŞİÖÇ-ğüşıöç-".repeat(8);
    let mut s = UiState::demo();
    s.my_name = long.clone();
    s.my_id = "f".repeat(300);
    s.peer = Some(PeerView {
        id: "x".into(),
        name: long.clone(),
        address: "fe80::1234:5678:9abc:def0%en0".repeat(3),
    });
    s.last_error = "Bağlantı kurulamadı: ".to_owned() + &"zaman aşımı ".repeat(40);
    for d in &mut s.devices {
        d.name = long.clone();
        d.address = "10.0.0.1".repeat(10);
    }
    s.trusted[0].name = long.clone();
    s.transfers.push(tr(
        "long",
        TransferDir::In,
        &format!("{long}.tar.gz"),
        u64::MAX,
        u64::MAX / 3,
        TransferStatus::Receiving,
    ));
    s.download_dir = PathBuf::from("/".to_owned() + &"klasör/".repeat(60));
    s
}

fn many_transfers() -> UiState {
    let statuses = [
        TransferStatus::Waiting,
        TransferStatus::Sending,
        TransferStatus::Sent,
        TransferStatus::Offered,
        TransferStatus::Receiving,
        TransferStatus::Received,
        TransferStatus::Rejected,
        TransferStatus::Cancelled,
        TransferStatus::Error,
    ];
    let mut s = UiState::demo();
    s.transfers = (0..200)
        .map(|i| {
            let st = statuses[i % statuses.len()];
            let dir = if i % 2 == 0 {
                TransferDir::In
            } else {
                TransferDir::Out
            };
            let size = [0u64, 1, 999, 1000, 10_500, 999_999_999, 5_000_000_000_000][i % 7];
            let done = [0u64, size / 2, size, size.saturating_add(77)][i % 4];
            tr(
                &format!("t{i}"),
                dir,
                &format!("dosya-{i}.bin"),
                size,
                done,
                st,
            )
        })
        .collect();
    s
}

fn weird() -> Vec<UiState> {
    let mut v = vec![
        pairing("", None, None),
        pairing("12", None, None),
        pairing("123 456", None, None),
        pairing("١٢٣٤٥٦٧٨٩٠١٢٣٤٥", None, None),
        pairing("💥🔥✓", Some(true), None),
    ];
    let mut s = idle_empty();
    s.my_name = String::new();
    s.my_id = String::new();
    s.last_error = "   ".into();
    s.devices = vec![DeviceView {
        id: String::new(),
        name: String::new(),
        address: String::new(),
        port: 0,
        trusted: true,
    }];
    s.trusted = vec![TrustedView {
        id: String::new(),
        name: "\n\t".into(),
        since: String::new(),
    }];
    s.transfers = vec![tr("", TransferDir::Out, "", 0, 0, TransferStatus::Sending)];
    s.download_dir = PathBuf::new();
    v.push(s);
    let mut c = UiState::demo();
    c.phase = LinkPhase::Connected;
    c.peer = None;
    v.push(c);
    v
}

fn all_states() -> Vec<(&'static str, UiState)> {
    let mut v = vec![
        ("demo", UiState::demo()),
        ("idle-empty", idle_empty()),
        (
            "connecting",
            UiState {
                phase: LinkPhase::Connecting,
                ..UiState::demo()
            },
        ),
        (
            "checking",
            UiState {
                phase: LinkPhase::Checking,
                peer: None,
                ..UiState::demo()
            },
        ),
        ("long-names", long_names()),
        ("many-transfers", many_transfers()),
    ];
    for mine in [None, Some(true), Some(false)] {
        for theirs in [None, Some(true), Some(false)] {
            v.push(("pairing", pairing("482913", mine, theirs)));
        }
    }
    for s in weird() {
        v.push(("weird", s));
    }
    v
}

#[test]
fn draw_never_panics() {
    let tabs = [Tab::Devices, Tab::Clipboard, Tab::Files, Tab::Settings];
    let sizes = [
        egui::vec2(920.0, 640.0),
        egui::vec2(720.0, 520.0),
        egui::vec2(160.0, 120.0),
        egui::vec2(2560.0, 1440.0),
    ];
    for theme in [ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::System] {
        for (name, st) in all_states() {
            for tab in tabs {
                for size in sizes {
                    let mut r = Run::new(UiPrefs {
                        tab,
                        theme,
                        ..Default::default()
                    });
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        r.frames(&st, size, 3);
                    }));
                    assert!(
                        res.is_ok(),
                        "panik: durum={name} sekme={tab:?} tema={theme:?} boyut={size:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn tessellation_works_for_every_tab() {
    let mut r = Run::new(UiPrefs::default());
    for tab in [Tab::Devices, Tab::Clipboard, Tab::Files, Tab::Settings] {
        r.prefs.tab = tab;
        let (full, _) = r.frame(&UiState::demo(), egui::vec2(920.0, 640.0), 2.0, |_| {});
        let prims = r.ctx.tessellate(full.shapes, full.pixels_per_point);
        assert!(!prims.is_empty());
    }
}

#[test]
fn dropped_files_become_send_paths() {
    let mut r = Run::new(UiPrefs::default());
    let st = UiState::demo();
    r.frames(&st, egui::vec2(920.0, 640.0), 1);
    let (_, out) = r.frame(&st, egui::vec2(920.0, 640.0), 1.0, |raw| {
        raw.dropped_files = vec![
            egui::DroppedFile {
                path: Some(PathBuf::from("/tmp/a.txt")),
                ..Default::default()
            },
            egui::DroppedFile {
                path: None,
                name: "web-only".into(),
                ..Default::default()
            },
        ];
    });
    assert_eq!(
        out,
        vec![Action::SendPaths(vec![PathBuf::from("/tmp/a.txt")])]
    );
    assert_eq!(r.prefs.tab, Tab::Files);
    // yolu olmayan bırakma eylem üretmez
    let (_, out) = r.frame(&st, egui::vec2(920.0, 640.0), 1.0, |raw| {
        raw.dropped_files = vec![egui::DroppedFile {
            path: None,
            ..Default::default()
        }];
    });
    assert!(out.is_empty());
}

#[test]
fn escape_cancels_pairing_but_enter_does_not_confirm() {
    let mut r = Run::new(UiPrefs::default());
    let st = pairing("123456", None, None);
    r.frames(&st, egui::vec2(920.0, 640.0), 2);
    let key = |k: egui::Key| egui::Event::Key {
        key: k,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    let (_, out) = r.frame(&st, egui::vec2(920.0, 640.0), 1.0, |raw| {
        raw.events.push(key(egui::Key::Enter))
    });
    assert!(out.is_empty(), "{out:?}");
    let (_, out) = r.frame(&st, egui::vec2(920.0, 640.0), 1.0, |raw| {
        raw.events.push(key(egui::Key::Escape))
    });
    assert_eq!(out, vec![Action::ConfirmPairing(false)]);
}

/// Bir noktaya tıklama (basma + bırakma) olayları.
fn click_at(raw: &mut egui::RawInput, pos: egui::Pos2, pressed: bool) {
    raw.events.push(egui::Event::PointerMoved(pos));
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    });
}

#[test]
fn pairing_confirm_button_emits_action() {
    let mut r = Run::new(UiPrefs::default());
    let st = pairing("123456", None, None);
    let size = egui::vec2(920.0, 640.0);
    r.frames(&st, size, 3);
    // "Aynı, eşleştir" sayfanın sağ alt düğmesi: sayfa ortalanmış, 360 pt geniş.
    // Düğmeyi bulmak için sayfanın alt kısmında sağ yarıyı tarar.
    let mut found = None;
    'outer: for dy in (0..260).step_by(6) {
        for dx in [60.0, 90.0, 120.0] {
            let pos = egui::pos2(460.0 + dx, 320.0 + dy as f32 - 40.0);
            let mut probe = Run::new(UiPrefs::default());
            probe.frames(&st, size, 3);
            probe.frame(&st, size, 1.0, |raw| click_at(raw, pos, true));
            let (_, out) = probe.frame(&st, size, 1.0, |raw| click_at(raw, pos, false));
            if out.contains(&Action::ConfirmPairing(true)) {
                found = Some(pos);
                break 'outer;
            }
            assert!(
                out.iter().all(|a| matches!(a, Action::ConfirmPairing(_))),
                "beklenmeyen eylem: {out:?}"
            );
        }
    }
    assert!(found.is_some(), "onay düğmesi bulunamadı");
    // Alttaki arayüz karartma tarafından kapatılmış olmalı: kenar çubuğuna tıklamak sekmeyi değiştirmez.
    let pos = egui::pos2(60.0, 140.0);
    r.frame(&st, size, 1.0, |raw| click_at(raw, pos, true));
    r.frame(&st, size, 1.0, |raw| click_at(raw, pos, false));
    assert_eq!(r.prefs.tab, Tab::Devices);
}

#[test]
fn size_formatting() {
    use super::view::fmt_size;
    assert_eq!(fmt_size(0), "0 bayt");
    assert_eq!(fmt_size(999), "999 bayt");
    assert_eq!(fmt_size(1000), "1,0 KB");
    assert_eq!(fmt_size(220_000), "220 KB");
    assert_eq!(fmt_size(3_400_000), "3,4 MB");
    assert_eq!(fmt_size(48_000_000), "48 MB");
    assert_eq!(fmt_size(999_999), "1,0 MB");
    assert!(fmt_size(u64::MAX).ends_with("PB"));
}

#[test]
fn theme_choice_roundtrip() {
    for t in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
        assert_eq!(ThemeChoice::parse(t.as_str()), t);
    }
    assert_eq!(ThemeChoice::parse("??"), ThemeChoice::System);
}

#[test]
fn app_icon_is_valid() {
    let i = super::app_icon();
    assert_eq!(i.rgba.len(), (i.width * i.height * 4) as usize);
}

/// Ekran görüntüleri (yalnızca `KOPRU_UI_SHOTS` ayarlıysa yazar).
#[test]
fn screenshots() {
    let Ok(dir) = std::env::var("KOPRU_UI_SHOTS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ppp = 2.0;
    let size = egui::vec2(920.0, 640.0);
    let mut idle_files = idle_empty();
    idle_files.transfers.clear();
    let scenes: Vec<(&str, Tab, UiState, bool)> = vec![
        ("devices", Tab::Devices, UiState::demo(), false),
        ("devices-idle", Tab::Devices, idle_empty(), false),
        ("clipboard", Tab::Clipboard, UiState::demo(), false),
        ("files", Tab::Files, UiState::demo(), false),
        ("files-empty", Tab::Files, idle_files, false),
        ("files-drop", Tab::Files, UiState::demo(), true),
        ("settings", Tab::Settings, UiState::demo(), false),
        (
            "pairing",
            Tab::Devices,
            pairing("482913", None, None),
            false,
        ),
        (
            "pairing-waiting",
            Tab::Devices,
            pairing("482913", Some(true), None),
            false,
        ),
        ("long-names", Tab::Devices, long_names(), false),
        ("min-size-files", Tab::Files, many_transfers(), false),
    ];
    let only = std::env::var("KOPRU_UI_ONLY").ok();
    for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
        for (name, tab, st, hover) in &scenes {
            if only.as_deref().is_some_and(|o| !name.contains(o)) {
                continue;
            }
            let size = if name.starts_with("min-size") {
                egui::vec2(720.0, 520.0)
            } else {
                size
            };
            let mut r = Run::new(UiPrefs {
                tab: *tab,
                theme,
                ..Default::default()
            });
            let mut raster = raster::Raster::new();
            let mut last = None;
            for _ in 0..4 {
                let (full, _) = r.frame(st, size, ppp, |raw| {
                    if *hover {
                        raw.hovered_files = vec![egui::HoveredFile {
                            path: Some("/tmp/x".into()),
                            mime: String::new(),
                        }];
                    }
                });
                raster.apply(&full.textures_delta);
                last = Some(full);
            }
            let full = last.unwrap();
            let prims = r.ctx.tessellate(full.shapes, full.pixels_per_point);
            let (w, h) = ((size.x * ppp) as usize, (size.y * ppp) as usize);
            let clear = theme::Palette::for_theme(r.ctx.theme()).bg;
            let rgb = raster.render(&prims, full.pixels_per_point, w, h, clear);
            let path = dir.join(format!("{name}-{}.png", theme.as_str()));
            raster::write_png(&path, w, h, &rgb).unwrap();
            println!("{}", path.display());
        }
    }
}

#[test]
fn sidebar_click_switches_tab_and_switch_toggles_clipboard() {
    let st = UiState::demo();
    let size = egui::vec2(920.0, 640.0);
    let mut r = Run::new(UiPrefs::default());
    r.frames(&st, size, 2);
    let pos = egui::pos2(60.0, 140.0); // "Paylaşım"
    r.frame(&st, size, 1.0, |raw| click_at(raw, pos, true));
    r.frame(&st, size, 1.0, |raw| click_at(raw, pos, false));
    assert_eq!(r.prefs.tab, Tab::Clipboard);
    // Paylaşım sayfasındaki anahtar: ilk kartın sağ kenarında.
    let mut hit = Vec::new();
    for x in [820.0, 835.0, 845.0, 855.0] {
        let pos = egui::pos2(x, 128.0);
        r.frame(&st, size, 1.0, |raw| click_at(raw, pos, true));
        hit.extend(r.frame(&st, size, 1.0, |raw| click_at(raw, pos, false)).1);
    }
    assert!(hit.contains(&Action::SetClipboard(false)), "{hit:?}");
}
