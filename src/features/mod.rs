//! Özellikler: dosya aktarımı ve pano paylaşımı. Ağa yalnızca `Sender` üzerinden mesaj gönderirler.

use std::sync::Arc;

use crate::msg::Msg;

pub mod clipboard;
pub mod files;

#[allow(unused_imports)]
pub use clipboard::{ClipboardBackend, ClipboardShare, SystemClipboard};
#[allow(unused_imports)]
pub use files::{safe_name, FileShare};

/// Bağlıysa mesajı gönderir; değilse false. Geri basınç için gerekirse bekleyebilir
/// (`net::Link::send` böyledir), bu yüzden arayüz iş parçacığından büyük veri göndermeyin.
pub type Sender = Arc<dyn Fn(&Msg) -> bool + Send + Sync>;
