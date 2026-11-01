//! Ayarlar: kullanıcı veri klasöründe (Windows: %APPDATA%\DarkDrop, macOS: ~/Library/Application Support/DarkDrop)
//! küçük bir JSON dosyası. Uygulama başka hiçbir yere yazmaz (gelen dosyalar hariç).

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    pub clipboard: bool,
    pub download_dir: Option<PathBuf>,
    pub last_peer: String,
    pub theme: String,
}

pub fn data_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("DarkDrop")
}

pub fn default_download_dir() -> PathBuf {
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir).join("DarkDrop")
}

impl Settings {
    pub fn load(dir: &Path) -> Settings {
        fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("settings.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(tmp, dir.join("settings.json"))
    }

    pub fn download_dir(&self) -> PathBuf {
        self.download_dir.clone().unwrap_or_else(default_download_dir)
    }

    pub fn device_name(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().chars().take(40).collect();
        }
        hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "Bilgisayar".into())
    }
}
