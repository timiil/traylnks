//! Local terminal snapshots, capture job lifecycle, and viewer IPC.
#[cfg(windows)]
mod graphics;
mod model;
mod storage;
#[cfg(windows)]
mod windows;

use base64::Engine;
#[cfg(windows)]
use chrono::Local;
use model::{Snap, SnapSummary};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, Wry};

#[derive(Debug, Clone, Default, Serialize)]
pub struct JobStatus {
    pub running: bool,
    pub message: String,
    pub snap_id: Option<String>,
    pub result: Option<SnapSummary>,
}

#[derive(Default)]
pub struct SnapState {
    pub job: Mutex<JobStatus>,
    #[cfg(windows)]
    pub foreground_before_menu: std::sync::atomic::AtomicIsize,
}

/// The tray's native menu can temporarily own the foreground via a hidden
/// window. Remember the visible window before that menu opens.
pub fn remember_foreground(app: &AppHandle<Wry>) {
    #[cfg(windows)]
    if let Some(hwnd) = windows::visible_foreground() {
        app.state::<crate::state::AppState>()
            .snaps
            .foreground_before_menu
            .store(hwnd, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(not(windows))]
    let _ = app;
}

fn root(app: &AppHandle<Wry>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|p| p.join("snaps"))
        .map_err(|e| format!("Find snaps folder: {e}"))
}

#[tauri::command]
pub fn get_snap_status(app: AppHandle<Wry>) -> JobStatus {
    app.state::<crate::state::AppState>()
        .snaps
        .job
        .lock()
        .clone()
}

fn publish(app: &AppHandle<Wry>, status: JobStatus) {
    *app.state::<crate::state::AppState>().snaps.job.lock() = status.clone();
    let _ = app.emit("snaps://status", &status);
    crate::tray::snap_tooltip(app, &status.message);
}

#[tauri::command]
pub fn take_snap(app: AppHandle<Wry>) -> Result<JobStatus, String> {
    let directory = root(&app)?;
    let initial = {
        let state = app.state::<crate::state::AppState>();
        let mut job = state.snaps.job.lock();
        if job.running {
            return Ok(job.clone());
        }
        *job = JobStatus {
            running: true,
            message: "TakeSnap: discovering terminals…".into(),
            ..Default::default()
        };
        job.clone()
    };
    #[cfg(windows)]
    windows::allow_foreground();
    let _ = app.emit("snaps://status", &initial);
    crate::tray::snap_tooltip(&app, &initial.message);
    crate::tray::rebuild_now(&app);
    let worker_app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("terminal-snapshot".into())
        .spawn(move || {
            let result = capture(&worker_app, &directory);
            let status = match result {
                Ok(Some(summary)) => JobStatus {
                    running: false,
                    message: format!(
                        "Snap {}: {} · {}/{} tabs complete",
                        summary.id, summary.status, summary.complete_count, summary.tab_count
                    ),
                    snap_id: Some(summary.id.clone()),
                    result: Some(summary),
                },
                Ok(None) => JobStatus {
                    message: "未发现 Windows Terminal / No Windows Terminal windows found".into(),
                    ..Default::default()
                },
                Err(error) => {
                    log::error!("TakeSnap: {error}");
                    JobStatus {
                        message: format!("TakeSnap failed: {error}"),
                        ..Default::default()
                    }
                }
            };
            publish(&worker_app, status);
            crate::tray::rebuild_now(&worker_app);
        })
    {
        let message = format!("Start TakeSnap: {error}");
        publish(
            &app,
            JobStatus {
                message: message.clone(),
                ..Default::default()
            },
        );
        crate::tray::rebuild_now(&app);
        return Err(message);
    }
    Ok(initial)
}

#[cfg(windows)]
fn capture(
    app: &AppHandle<Wry>,
    directory: &std::path::Path,
) -> Result<Option<SnapSummary>, String> {
    let now = Local::now();
    let pending = storage::PendingSnap::create(directory, now)?;
    let id = pending.id.clone();
    let progress = |message: String| {
        publish(
            app,
            JobStatus {
                running: true,
                message,
                snap_id: Some(id.clone()),
                result: None,
            },
        )
    };
    let foreground = app
        .state::<crate::state::AppState>()
        .snaps
        .foreground_before_menu
        .load(std::sync::atomic::Ordering::Relaxed);
    let (windows, warnings) = windows::collect(&pending.path, foreground, progress)?;
    if windows.is_empty() {
        return Ok(None);
    }
    progress("Saving snapshot…".into());
    let snap = Snap {
        version: 1,
        id,
        captured_at: now.to_rfc3339(),
        created_at_ms: now.timestamp_millis(),
        hostname: crate::state::hostname(app),
        windows,
        warnings,
    };
    pending.commit(&snap).map(Some)
}

#[cfg(not(windows))]
fn capture(
    _app: &AppHandle<Wry>,
    _directory: &std::path::Path,
) -> Result<Option<SnapSummary>, String> {
    Err("TakeSnap is supported on Windows only".into())
}

#[tauri::command]
pub async fn list_snaps(app: AppHandle<Wry>) -> Result<Vec<SnapSummary>, String> {
    let directory = root(&app)?;
    tauri::async_runtime::spawn_blocking(move || storage::list(&directory))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct SnapDetail {
    pub snap: Snap,
    pub record_text: String,
}

#[tauri::command]
pub async fn get_snap_detail(app: AppHandle<Wry>, snap_id: String) -> Result<SnapDetail, String> {
    let directory = root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let snap = storage::detail(&directory, &snap_id)?;
        Ok(SnapDetail {
            record_text: snap.record_text(),
            snap,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_snap_image(
    app: AppHandle<Wry>,
    snap_id: String,
    tab_id: String,
) -> Result<String, String> {
    let directory = root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = storage::image(&directory, &snap_id, &tab_id)?;
        Ok(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Close the viewer back to the tray without stopping an active capture.
#[tauri::command]
pub fn hide_snaps(app: AppHandle<Wry>) -> Result<(), String> {
    app.get_webview_window("snaps")
        .ok_or_else(|| "ViewSnaps window is unavailable".to_string())?
        .hide()
        .map_err(|error| format!("Hide ViewSnaps: {error}"))
}

pub fn show_viewer(app: &AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window("snaps") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to("snaps", "snaps://shown", ());
    }
}
