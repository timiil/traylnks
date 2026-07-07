//! Tauri commands invoked by the Settings frontend.

use crate::config::Config;
use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};

#[derive(Serialize)]
pub struct Diagnostics {
    pub app_version: String,
    pub config_path: Option<String>,
    pub watch_path: Option<String>,
    pub cloud_path: Option<String>,
    pub watcher_ok: bool,
    pub last_scan_unix: Option<u64>,
    pub hostname: String,
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
    Diagnostics {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        config_path: Config::path(&app).map(|p| p.to_string_lossy().into_owned()),
        watch_path: cfg.watch_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        cloud_path: cfg.cloud_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        watcher_ok: crate::state::watcher_ok(&app),
        last_scan_unix: crate::state::last_scan_unix(&app),
        hostname: st.hostname.clone(),
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
        Some(wp) => crate::launch::open(&wp).map_err(|e| e.to_string()),
        None => Err("no watch folder configured".into()),
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    let res = if enabled {
        mgr.enable()
    } else {
        mgr.disable()
    };
    res.map_err(|e| e.to_string())?;

    // persist the preference
    let mut cfg = app.state::<AppState>().config.read().clone();
    cfg.autostart = enabled;
    cfg.save(&app).map_err(|e| e.to_string())?;
    *app.state::<AppState>().config.write() = cfg;
    Ok(())
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
