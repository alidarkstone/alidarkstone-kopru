//! Şifreli kanal üzerinden giden uygulama mesajları (JSON, "t" alanı türü belirtir).
//! Electron referans sürümüyle (reference/electron) birebir aynı tel biçimi.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Msg {
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "trust")]
    Trust { known: bool },
    #[serde(rename = "pair.ok")]
    PairOk,
    #[serde(rename = "pair.no")]
    PairNo,

    #[serde(rename = "clip.text")]
    ClipText { text: String },

    #[serde(rename = "file.offer")]
    FileOffer { id: String, name: String, size: u64 },
    #[serde(rename = "file.accept")]
    FileAccept { id: String },
    /// data: standart base64 (dolgulu)
    #[serde(rename = "file.chunk")]
    FileChunk { id: String, data: String },
    #[serde(rename = "file.done")]
    FileDone { id: String, sha256: String },
    #[serde(rename = "file.cancel")]
    FileCancel { id: String },

    /// edge: hedefin GİRİŞ kenarı (kaynağın kenarının tersi); pos 0..1
    #[serde(rename = "input.enter")]
    InputEnter { edge: Edge, pos: f64 },
    #[serde(rename = "input.ev")]
    InputEv(InputEvent),
    #[serde(rename = "input.leave")]
    InputLeave { pos: f64 },
    #[serde(rename = "input.release")]
    InputRelease,

    /// Bilinmeyen türler sessizce yok sayılır (ileri uyumluluk).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn opposite(self) -> Edge {
        match self {
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

/// Platformdan bağımsız giriş olayı. Tuşlar USB HID kullanım kodu (sayfa 0x07) olarak taşınır,
/// böylece Windows ve macOS arasında çevrilebilir. `vk`/`sc` alanları Windows→Windows için
/// isteğe bağlı ek bilgidir (Electron sürümüyle uyum).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InputEvent {
    Move { dx: i32, dy: i32 },
    Button { b: MouseButton, down: bool },
    /// d: 120 = bir çentik; h: yatay
    Wheel { d: i32, #[serde(default)] h: bool },
    Key {
        #[serde(default)]
        hid: u16,
        #[serde(default)]
        vk: u16,
        #[serde(default)]
        sc: u16,
        #[serde(default)]
        ext: bool,
        down: bool,
    },
}
