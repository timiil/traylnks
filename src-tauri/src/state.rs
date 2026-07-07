//! Shared, thread-safe application state and the Tauri `setup` hook.

use crate::config::Config;
use parking_lot::{Mutex, RwLock};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
use tauri::{App, AppHandle, Manager, Wry};

pub struct AppState {
    pub config: RwLock<Config>,
    /// Lowercased current hostname, used for `.station` matching.
    pub hostname: String,
    pub last_scan: Mutex<Option<SystemTime>>,
    /// Stored so the watcher is not dropped (dropping stops watching).
    pub watcher: Mutex<Option<crate::watcher::AppDebouncer>>,
    pub watcher_ok: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        let hostname = gethostname::gethostname()
            .to_string_lossy()
            .to_lowercase();
        Self {
            config: RwLock::new(Config::default()),
            hostname,
            last_scan: Mutex::new(None),
            watcher: Mutex::new(None),
            watcher_ok: AtomicBool::new(false),
        }
    }
}

/// One-time setup: load config, build the tray, start the watcher.
pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();

    let cfg = Config::load(handle);
    *handle.state::<AppState>().config.write() = cfg.clone();

    crate::tray::build_tray(handle)?;

    if let Some(wp) = cfg.watch_path.clone() {
        crate::watcher::restart(handle, Some(wp));
    }

    Ok(())
}

// --- small read/write helpers used across modules --------------------------

pub fn hostname(app: &AppHandle<Wry>) -> String {
    app.state::<AppState>().hostname.clone()
}

pub fn watch_path(app: &AppHandle<Wry>) -> Option<PathBuf> {
    app.state::<AppState>().config.read().watch_path.clone()
}

pub fn set_last_scan(app: &AppHandle<Wry>, t: SystemTime) {
    *app.state::<AppState>().last_scan.lock() = Some(t);
}

pub fn last_scan_unix(app: &AppHandle<Wry>) -> Option<u64> {
    app.state::<AppState>()
        .last_scan
        .lock()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok().map(|d| d.as_secs()))
}

pub fn watcher_ok(app: &AppHandle<Wry>) -> bool {
    app.state::<AppState>().watcher_ok.load(Ordering::SeqCst)
}
