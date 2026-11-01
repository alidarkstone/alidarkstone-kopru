//! Ağ çekirdeği: çerçeveleme, anahtarlar, şifreli kanal, bağlantı yöneticisi, keşif, güvenilenler.
//! Tel biçimi Electron referansıyla (reference/electron/src/core) birebir aynıdır.

pub mod channel;
pub mod discovery;
pub mod framing;
pub mod keys;
pub mod link;
pub mod trust;

#[allow(unused_imports)]
pub use discovery::{Discovery, DiscoveryConfig, Found, DISCOVERY_PORT};
#[allow(unused_imports)]
pub use keys::Identity;
#[allow(unused_imports)]
pub use link::{Link, LinkEvent, MsgHandler, DEFAULT_PORT};
#[allow(unused_imports)]
pub use trust::{FileTrustStore, MemoryTrustStore, TrustStore, TrustedEntry};

/// Electron referans uygulamasıyla uçtan uca birlikte çalışma testi (Node gerekir; yoksa atlanır).
#[cfg(test)]
#[path = "../../tests/interop/interop_test.rs"]
mod interop_test;
