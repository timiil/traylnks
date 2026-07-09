//! Scheduled background sync loop + "Sync now" trigger.
//!
//! The loop runs on `tauri::async_runtime` (tokio): each tick it sleeps for the
//! configured interval, racing against a `tokio::sync::Notify` so "Sync now"
//! can fire it immediately. The interval is re-read from config every tick, so
//! a UI change takes effect without restarting the loop.
//!
//! `sync_busy` prevents the scheduled tick and a manual "Sync now" from
//! overlapping. Downloaded files land in `watch_path`, where the existing file
//! watcher already rebuilds the tray menu — no coupling to tray/menu code.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};
use tokio::sync::Notify;

use super::provider::CloudProvider;
use super::sync::{self, SyncOutcome};
use super::{AccountInfo, CloudError};
use crate::config::Config;
use crate::state::AppState;

const MANIFEST_NAME: &str = "sync_manifest.json";
/// Grace period before the first sync after startup (lets the app settle).
const STARTUP_GRACE_SECS: u64 = 15;

#[derive(Clone, Default, Serialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub connected: bool,
    pub provider: String,
    pub cloud_folder: Option<String>,
    pub interval_min: u32,
    pub last_sync_unix: Option<u64>,
    pub last_ok: bool,
    pub in_progress: bool,
    pub last_error: Option<String>,
    pub uploaded: u32,
    pub downloaded: u32,
    pub failed: u32,
    pub account: Option<AccountInfo>,
}

fn now_unix() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

/// Construct the configured provider. Today only `"aliyun"`.
pub fn provider_for(id: &str) -> Arc<dyn CloudProvider> {
    match id {
        "aliyun" => Arc::new(super::aliyun::AliyunProvider::new()),
        other => {
            log::warn!("unknown cloud provider '{other}', falling back to aliyun");
            Arc::new(super::aliyun::AliyunProvider::new())
        }
    }
}

/// Fresh status snapshot for the UI: config-driven fields merged with the last
/// run's recorded outcome.
pub fn status(app: &AppHandle<Wry>) -> SyncStatus {
    let st = app.state::<AppState>();
    let cfg = st.config.read().clone();
    let recorded = st.sync_status.read().clone();
    SyncStatus {
        enabled: cfg.cloud_enabled,
        connected: st.provider.is_connected(),
        provider: cfg.cloud_provider,
        cloud_folder: cfg.cloud_folder,
        interval_min: cfg.cloud_sync_interval_min,
        account: st.provider.account(),
        ..recorded
    }
}

/// Start the loop if cloud sync is enabled + a folder is chosen. Idempotent
/// alongside [`stop`]: callers pair them via [`restart`].
pub fn start(app: &AppHandle<Wry>) {
    let cfg = app.state::<AppState>().config.read().clone();
    if !cfg.cloud_enabled || cfg.cloud_folder.is_none() {
        return;
    }
    let notify = Arc::new(Notify::new());
    *app.state::<AppState>().sync_now.lock() = Some(notify.clone());
    let app2 = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            let mins = app2
                .state::<AppState>()
                .config
                .read()
                .cloud_sync_interval_min
                .max(1) as u64;
            let dur = if first {
                first = false;
                Duration::from_secs(STARTUP_GRACE_SECS)
            } else {
                Duration::from_secs(60 * mins)
            };
            tokio::select! {
                _ = tokio::time::sleep(dur) => {}
                _ = notify.notified() => {}
            }
            if !app2.state::<AppState>().config.read().cloud_enabled {
                break;
            }
            run_once(&app2).await;
        }
    });
    *app.state::<AppState>().sync_handle.lock() = Some(handle);
}

pub fn stop(app: &AppHandle<Wry>) {
    if let Some(h) = app.state::<AppState>().sync_handle.lock().take() {
        h.abort();
    }
    *app.state::<AppState>().sync_now.lock() = None;
}

/// Stop + start. Called from `set_config` whenever cloud settings change.
pub fn restart(app: &AppHandle<Wry>) {
    stop(app);
    start(app);
}

/// After a successful connect, default the cloud folder to
/// `/traylnks/<hostname>` if the user hasn't chosen one. Per-machine subfolder
/// keeps each workstation's links organized under a shared `/traylnks` root.
pub fn ensure_default_folder(app: &AppHandle<Wry>) -> Result<(), String> {
    let st = app.state::<AppState>();
    let empty = st
        .config
        .read()
        .cloud_folder
        .as_deref()
        .map_or(true, |s| s.is_empty());
    if !empty {
        return Ok(());
    }
    let mut cfg = st.config.read().clone();
    cfg.cloud_folder = Some(format!("/traylnks/{}", st.hostname));
    cfg.save(app).map_err(|e| e.to_string())?;
    *st.config.write() = cfg;
    log::info!("aliyun: default cloud folder set to /traylnks/{}", st.hostname);
    restart(app);
    Ok(())
}

/// Wake the loop for an immediate run (used by the "Sync now" command).
pub fn trigger_now(app: &AppHandle<Wry>) -> Result<(), String> {
    let st = app.state::<AppState>();
    let cfg = st.config.read().clone();
    if !cfg.cloud_enabled {
        return Err("cloud sync is not enabled".into());
    }
    if cfg.watch_path.is_none() || cfg.cloud_folder.is_none() {
        return Err("set a watch path and cloud folder first".into());
    }
    if st.sync_busy.load(Ordering::SeqCst) {
        return Err("a sync is already running".into());
    }
    // Clone the Arc out so the lock guard (a temporary borrowing `st`) is
    // dropped at the `;`, not carried into the match.
    let notify = st.sync_now.lock().as_ref().cloned();
    match notify {
        Some(n) => {
            n.notify_one();
            Ok(())
        }
        None => Err("sync service is not running".into()),
    }
}

/// One sync cycle. Guards against re-entry via `sync_busy`.
async fn run_once(app: &AppHandle<Wry>) {
    let st = app.state::<AppState>();
    if st.sync_busy.swap(true, Ordering::SeqCst) {
        log::info!("sync: previous run still busy, skipping");
        return;
    }
    st.sync_status.write().in_progress = true;

    let cfg = app.state::<AppState>().config.read().clone();
    let result = run_with_config(app, &cfg).await;

    {
        let st = app.state::<AppState>();
        let mut s = st.sync_status.write();
        s.last_sync_unix = Some(now_unix());
        match result {
            Ok(o) => {
                s.last_ok = o.ok;
                s.uploaded = o.uploaded;
                s.downloaded = o.downloaded;
                s.failed = o.failed;
                s.last_error = if o.ok {
                    None
                } else {
                    o.errors.first().cloned()
                };
                s.in_progress = false;
            }
            Err(e) => {
                s.last_ok = false;
                s.last_error = Some(e.to_string());
                s.in_progress = false;
            }
        }
    }
    app.state::<AppState>()
        .sync_busy
        .store(false, Ordering::SeqCst);
}

async fn run_with_config(app: &AppHandle<Wry>, cfg: &Config) -> Result<SyncOutcome, CloudError> {
    if !cfg.cloud_enabled {
        return Ok(SyncOutcome::default());
    }
    let watch = cfg
        .watch_path
        .clone()
        .ok_or_else(|| CloudError::Other("cloud sync enabled but watch_path not set".into()))?;
    let folder = cfg
        .cloud_folder
        .clone()
        .ok_or_else(|| CloudError::Other("cloud sync enabled but cloud_folder not set".into()))?;
    let manifest_path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|d| d.join(MANIFEST_NAME))
        .ok_or_else(|| CloudError::Other("no app config dir available".into()))?;
    let provider = app.state::<AppState>().provider.clone();
    sync::run(
        provider.as_ref(),
        &watch,
        &folder,
        &cfg.cloud_provider,
        &manifest_path,
    )
    .await
}
