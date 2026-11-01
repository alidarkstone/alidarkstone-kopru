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

    /// Bilinmeyen türler sessizce yok sayılır (ileri uyumluluk).
    #[serde(other)]
    Unknown,
}
