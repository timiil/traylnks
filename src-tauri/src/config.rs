//! Application configuration. Stored as TOML at the platform app-config dir.
//!
//! Deliberate deviation from the PRD's illustrative YAML: TOML is serde-native
//! and avoids backslash-escaping surprises for `C:\...` Windows paths. The file
//! is managed by the Settings UI, not hand-edited.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;

fn default_true() -> bool {
    true
}

fn default_cloud_disabled() -> bool {
    false
}

fn default_aliyun() -> String {
    "aliyun".to_string()
}

/// Default sync interval in minutes. 30 was chosen as the floor of the
/// user-requested "30 or 60" range — polite to the provider's rate limits.
fn default_interval() -> u32 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Local watch path — the single source of the tray menu. `None` on first run.
    #[serde(default)]
    pub watch_path: Option<PathBuf>,

    /// Cloud sync master switch. Off until the user authorizes + picks a folder.
    #[serde(default = "default_cloud_disabled")]
    pub cloud_enabled: bool,

    /// Provider id (`"aliyun"` today; a `CloudProvider` trait lookup key later).
    /// Stored as a String so adding providers needs no enum migration.
    #[serde(default = "default_aliyun")]
    pub cloud_provider: String,

    /// Absolute path inside the cloud drive to mirror against `watch_path`
    /// (e.g. `"/TrayLnks"`). `None` until the user picks one.
    #[serde(default)]
    pub cloud_folder: Option<String>,

    /// Auto-sync cadence in minutes (UI offers 15/30/60/120; default 30).
    #[serde(default = "default_interval")]
    pub cloud_sync_interval_min: u32,

    /// Start hidden to the tray (no window). Defaults to true.
    #[serde(default = "default_true")]
    pub start_minimized: bool,

    /// Launch on Windows login (registry Run key via tauri-plugin-autostart).
    #[serde(default)]
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_path: None,
            cloud_enabled: false,
            cloud_provider: default_aliyun(),
            cloud_folder: None,
            cloud_sync_interval_min: default_interval(),
            start_minimized: true,
            autostart: false,
        }
    }
}

impl Config {
    /// `%APPDATA%\com.traylnks.launcher\config.toml` on Windows
    /// (identifier from tauri.conf.json).
    pub fn path(app: &tauri::AppHandle) -> Option<PathBuf> {
        app.path()
            .app_config_dir()
            .ok()
            .map(|d| d.join("config.toml"))
    }

    /// Load config, returning the default if missing or unparseable (never errors).
    pub fn load(app: &tauri::AppHandle) -> Self {
        let Some(p) = Self::path(app) else {
            return Self::default();
        };
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                log::warn!("config parse error at {}: {e}", p.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let p = Self::path(app).ok_or_else(|| "no app config dir available".to_string())?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
        }
        let s = toml::to_string_pretty(self).map_err(|e| format!("serialize config: {e}"))?;
        std::fs::write(&p, s).map_err(|e| format!("write config: {e}"))?;
        Ok(())
    }
}
