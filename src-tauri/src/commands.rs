//! Tauri commands invoked by the Settings frontend.

use crate::config::Config;
use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};

#[derive(Serialize)]
pub struct Diagnostics {
    pub app_version: String,
    pub config_path: Option<String>,
    pub log_dir: Option<String>,
    pub watch_path: Option<String>,
    pub watcher_ok: bool,
    pub last_scan_unix: Option<u64>,
    pub hostname: String,
    pub cloud_enabled: bool,
    pub cloud_provider: String,
    pub cloud_folder: Option<String>,
    pub cloud_connected: bool,
    pub cloud_last_sync_unix: Option<u64>,
}

#[tauri::command]
pub fn get_config(app: AppHandle<Wry>) -> Config {
    app.state::<AppState>().config.read().clone()
}

/// Save config, restart the watcher, and rebuild the menu.
#[tauri::command]
pub fn set_config(app: AppHandle<Wry>, cfg: Config) -> Result<(), String> {
    cfg.save(&app).map_err(|e| e.to_string())?;
    let watch = cfg.watch_path.clone();
    *app.state::<AppState>().config.write() = cfg;
    crate::watcher::restart(&app, watch);
    crate::tray::rebuild_now(&app);
    // Re-evaluate the cloud-sync loop against the new config.
    crate::cloud::service::restart(&app);
    Ok(())
}

#[tauri::command]
pub fn get_hostname(app: AppHandle<Wry>) -> String {
    crate::state::hostname(&app)
}

#[tauri::command]
pub fn get_diagnostics(app: AppHandle<Wry>) -> Diagnostics {
    let st = app.state::<AppState>();
    let cfg = st.config.read().clone();
    let cloud = crate::cloud::service::status(&app);
    Diagnostics {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        config_path: Config::path(&app).map(|p| p.to_string_lossy().into_owned()),
        log_dir: app
            .path()
            .app_log_dir()
            .ok()
            .map(|d| d.to_string_lossy().into_owned()),
        watch_path: cfg
            .watch_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        watcher_ok: crate::state::watcher_ok(&app),
        last_scan_unix: crate::state::last_scan_unix(&app),
        hostname: st.hostname.clone(),
        cloud_enabled: cloud.enabled,
        cloud_provider: cloud.provider,
        cloud_folder: cloud.cloud_folder,
        cloud_connected: cloud.connected,
        cloud_last_sync_unix: cloud.last_sync_unix,
    }
}

#[tauri::command]
pub fn refresh(app: AppHandle<Wry>) -> Result<(), String> {
    crate::tray::rebuild_now(&app);
    Ok(())
}

#[tauri::command]
pub fn pick_watch_folder(app: AppHandle<Wry>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .set_title("Select watch folder")
        .blocking_pick_folder();
    Ok(picked.map(|f| f.to_string()))
}

#[tauri::command]
pub fn open_launcher_folder(app: AppHandle<Wry>) -> Result<(), String> {
    match crate::state::watch_path(&app) {
        Some(wp) => crate::launch::open_folder(&wp).map_err(|e| e.to_string()),
        None => Err("no watch folder configured".into()),
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    let res = if enabled { mgr.enable() } else { mgr.disable() };
    res.map_err(|e| e.to_string())?;

    // persist the preference
    let mut cfg = app.state::<AppState>().config.read().clone();
    cfg.autostart = enabled;
    cfg.save(&app).map_err(|e| e.to_string())?;
    *app.state::<AppState>().config.write() = cfg;
    Ok(())
}

// ---- cloud sync (provider-agnostic; Aliyun today) ------------------------

#[tauri::command]
pub async fn cloud_start_auth(app: AppHandle<Wry>) -> Result<crate::cloud::AuthStart, String> {
    app.state::<AppState>()
        .provider
        .start_auth()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cloud_poll_auth(
    app: AppHandle<Wry>,
    session: String,
) -> Result<crate::cloud::AuthStatus, String> {
    let provider = app.state::<AppState>().provider.clone();
    let status = provider
        .poll_auth(&session)
        .await
        .map_err(|e| e.to_string())?;
    // Confirmed → exchange the captured code for tokens immediately, so the
    // frontend only needs to poll (no separate finalize command).
    if status == crate::cloud::AuthStatus::Confirmed {
        provider
            .finalize_auth(&session)
            .await
            .map_err(|e| e.to_string())?;
        // First-time connect: default the cloud folder if none chosen.
        let _ = crate::cloud::service::ensure_default_folder(&app);
    }
    Ok(status)
}

#[tauri::command]
pub async fn cloud_disconnect(app: AppHandle<Wry>) -> Result<(), String> {
    app.state::<AppState>()
        .provider
        .disconnect()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cloud_list_folders(
    app: AppHandle<Wry>,
    path: Option<String>,
) -> Result<Vec<crate::cloud::RemoteNode>, String> {
    let p = path
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/".to_string());
    app.state::<AppState>()
        .provider
        .list_folders(&p)
        .await
        .map_err(|e| e.to_string())
}

/// Validate a cloud folder path (format only; existence is resolved at sync
/// time). Returns the canonical trimmed path to store in config.
#[tauri::command]
pub async fn cloud_set_folder(_app: AppHandle<Wry>, path: String) -> Result<String, String> {
    let p = path.trim();
    if p.is_empty() {
        return Err("cloud folder path is empty".into());
    }
    if !p.starts_with('/') {
        return Err("cloud folder path must start with '/'".into());
    }
    Ok(p.to_string())
}

#[tauri::command]
pub fn get_cloud_status(app: AppHandle<Wry>) -> crate::cloud::service::SyncStatus {
    crate::cloud::service::status(&app)
}

#[tauri::command]
pub fn sync_now(app: AppHandle<Wry>) -> Result<(), String> {
    crate::cloud::service::trigger_now(&app)
}

#[tauri::command]
pub fn show_settings_cmd(app: AppHandle<Wry>) {
    show_settings(&app);
}

/// Show (and focus) the settings window. Also the single-instance callback.
pub fn show_settings(app: &AppHandle<Wry>) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}
