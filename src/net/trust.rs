//! Güvenilen (eşleşmiş) cihazlar. Dosya biçimi Electron sürümünün `trusted.json`'u ile aynı:
//! `[{"id":"<16 hex>","raw":"<base64url>","name":"...","since":"<ISO-8601>"}]`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::state::TrustedView;

pub trait TrustStore: Send + Sync {
    /// Kimlik VE açık anahtar birlikte eşleşmeli.
    fn is_trusted(&self, id: &str, raw_b64url: &str) -> bool;
    fn add(&self, id: &str, raw_b64url: &str, name: &str);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedEntry {
    pub id: String,
    pub raw: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub since: String,
}

fn upsert(list: &mut Vec<TrustedEntry>, id: &str, raw: &str, name: &str) {
    list.retain(|t| t.id != id);
    list.push(TrustedEntry { id: id.into(), raw: raw.into(), name: name.into(), since: now_iso() });
}

fn to_views(list: &[TrustedEntry]) -> Vec<TrustedView> {
    list.iter()
        .map(|t| TrustedView { id: t.id.clone(), name: t.name.clone(), since: t.since.chars().take(10).collect() })
        .collect()
}

/// JSON dosyasında saklanan güvenilenler listesi. Her değişiklik atomik olarak yazılır.
pub struct FileTrustStore {
    path: PathBuf,
    list: Mutex<Vec<TrustedEntry>>,
}

impl FileTrustStore {
    /// Dosya yoksa veya okunamıyorsa boş liste ile başlar (referanstaki davranış).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let list = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
            .map(|v| v.into_iter().filter_map(|x| serde_json::from_value::<TrustedEntry>(x).ok()).collect())
            .unwrap_or_default();
        FileTrustStore { path, list: Mutex::new(list) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<TrustedEntry>> {
        self.list.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, list: &[TrustedEntry]) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let data = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &self.path)
    }

    pub fn forget(&self, id: &str) {
        let mut l = self.lock();
        l.retain(|t| t.id != id);
        let _ = self.save(&l);
    }

    /// Bu kimlik listede var mı (açık anahtara bakmadan; arayüzdeki "güvenilir" rozeti için).
    pub fn contains(&self, id: &str) -> bool {
        self.lock().iter().any(|t| t.id == id)
    }

    pub fn entries(&self) -> Vec<TrustedEntry> {
        self.lock().clone()
    }

    /// Arayüz için; `since` yalnızca tarih (YYYY-AA-GG).
    pub fn list(&self) -> Vec<TrustedView> {
        to_views(&self.lock())
    }
}

impl TrustStore for FileTrustStore {
    fn is_trusted(&self, id: &str, raw: &str) -> bool {
        self.lock().iter().any(|t| t.id == id && t.raw == raw)
    }
    fn add(&self, id: &str, raw: &str, name: &str) {
        let mut l = self.lock();
        upsert(&mut l, id, raw, name);
        let _ = self.save(&l);
    }
}

/// Bellekte tutulan liste (testler ve geçici kullanım için).
#[derive(Default)]
pub struct MemoryTrustStore {
    list: Mutex<Vec<TrustedEntry>>,
}

impl MemoryTrustStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn forget(&self, id: &str) {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).retain(|t| t.id != id);
    }
    pub fn list(&self) -> Vec<TrustedView> {
        to_views(&self.list.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl TrustStore for MemoryTrustStore {
    fn is_trusted(&self, id: &str, raw: &str) -> bool {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|t| t.id == id && t.raw == raw)
    }
    fn add(&self, id: &str, raw: &str, name: &str) {
        upsert(&mut self.list.lock().unwrap_or_else(|e| e.into_inner()), id, raw, name);
    }
}

/// Şimdiki zaman, `new Date().toISOString()` biçiminde (UTC, milisaniyeli).
pub fn now_iso() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    iso_from_unix_ms(d.as_millis() as i64)
}

fn iso_from_unix_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Howard Hinnant'ın civil_from_days algoritması.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, m, d, sod / 3600, sod % 3600 / 60, sod % 60, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso_from_unix_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_unix_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_from_unix_ms(1_793_534_706_123), "2026-11-01T12:05:06.123Z");
    }

    #[test]
    fn file_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("kopru-trust-{}", hex::encode(rand::random::<[u8; 6]>())));
        let p = dir.join("trusted.json");
        let s = FileTrustStore::load(&p);
        assert!(!s.is_trusted("aaaaaaaaaaaaaaaa", "KEY"));
        s.add("aaaaaaaaaaaaaaaa", "KEY", "Mac");
        s.add("bbbbbbbbbbbbbbbb", "KEY2", "PC");
        s.add("aaaaaaaaaaaaaaaa", "KEY", "Mac yeni");
        assert!(s.is_trusted("aaaaaaaaaaaaaaaa", "KEY"));
        assert!(!s.is_trusted("aaaaaaaaaaaaaaaa", "OTHER"), "aynı kimlik, farklı anahtar güvenilmez");
        let s2 = FileTrustStore::load(&p);
        assert!(s2.is_trusted("aaaaaaaaaaaaaaaa", "KEY"));
        let l = s2.list();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].name, "Mac yeni");
        assert_eq!(l[1].since.len(), 10);
        s2.forget("aaaaaaaaaaaaaaaa");
        assert!(!FileTrustStore::load(&p).contains("aaaaaaaaaaaaaaaa"));
        // Electron biçimi ve bozuk girdiler
        fs::write(&p, r#"[{"id":"cccccccccccccccc","raw":"R","name":"E","since":"2025-01-01T00:00:00.000Z"},{"x":1},5]"#).unwrap();
        let s3 = FileTrustStore::load(&p);
        assert!(s3.is_trusted("cccccccccccccccc", "R"));
        assert_eq!(s3.list().len(), 1);
        fs::write(&p, "bozuk").unwrap();
        assert!(FileTrustStore::load(&p).list().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
