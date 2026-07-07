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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Local watch path — the single source of the tray menu. `None` on first run.
    #[serde(default)]
    pub watch_path: Option<PathBuf>,

    /// Optional cloud-synced path. v0.1 only stores it; cloud sync is out of scope.
    #[serde(default)]
    pub cloud_path: Option<PathBuf>,

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
            cloud_path: None,
            start_minimized: true,
            autostart: false,
        }
    }
}

impl Config {
    /// `%APPDATA%\com.traylnks.app\config.toml` on Windows.
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
