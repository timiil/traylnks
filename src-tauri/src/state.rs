//! Shared, thread-safe application state and the Tauri `setup` hook.

use crate::cloud::provider::CloudProvider;
use crate::cloud::service::{self, SyncStatus};
use crate::config::Config;
use parking_lot::{Mutex, RwLock};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

    // -- cloud sync --
    /// The active cloud provider (constructed from config at startup).
    pub provider: Arc<dyn CloudProvider>,
    pub sync_status: RwLock<SyncStatus>,
    /// "Sync now" signal shared with the loop task.
    pub sync_now: Mutex<Option<Arc<tokio::sync::Notify>>>,
    pub sync_handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Re-entry guard for the sync cycle.
    pub sync_busy: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        let hostname = gethostname::gethostname().to_string_lossy().to_lowercase();
        // Default config so we can pick a provider; `setup` loads the real one
        // and may swap the provider if `cloud_provider` ever differs.
        let provider = service::provider_for(&Config::default().cloud_provider);
        Self {
            config: RwLock::new(Config::default()),
            hostname,
            last_scan: Mutex::new(None),
            watcher: Mutex::new(None),
            watcher_ok: AtomicBool::new(false),
            provider,
            sync_status: RwLock::new(SyncStatus::default()),
            sync_now: Mutex::new(None),
            sync_handle: Mutex::new(None),
            sync_busy: AtomicBool::new(false),
        }
    }
}

/// One-time setup: load config, build the tray, start the watcher + cloud sync.
pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();

    let cfg = Config::load(handle);
    *handle.state::<AppState>().config.write() = cfg.clone();

    crate::tray::build_tray(handle)?;

    if let Some(wp) = cfg.watch_path.clone() {
        crate::watcher::restart(handle, Some(wp));
    }

    // Start the cloud-sync loop if enabled (no-op otherwise).
    crate::cloud::service::start(handle);

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
    app.state::<AppState>().last_scan.lock().and_then(|t| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    })
}

pub fn watcher_ok(app: &AppHandle<Wry>) -> bool {
    app.state::<AppState>().watcher_ok.load(Ordering::SeqCst)
}
