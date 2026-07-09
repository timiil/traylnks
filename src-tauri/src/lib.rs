//! TrayLnks — Windows tray link launcher (Tauri v2 backend).

mod cloud;
mod commands;
mod config;
mod icon;
mod launch;
#[cfg(windows)]
mod launch_windows;
mod menu_tree;
mod state;
mod station;
mod tray;
mod watcher;

use tauri::WindowEvent;

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir { file_name: None },
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch just focuses the Settings window (no second tray).
            commands::show_settings(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::new())
        .setup(state::setup)
        .on_menu_event(tray::handle_menu_event)
        .on_window_event(|window, event| {
            // Hide Settings on close instead of destroying it (re-open is instant).
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_config,
            commands::get_hostname,
            commands::get_diagnostics,
            commands::refresh,
            commands::pick_watch_folder,
            commands::open_launcher_folder,
            commands::show_settings_cmd,
            commands::set_autostart,
            commands::cloud_start_auth,
            commands::cloud_poll_auth,
            commands::cloud_disconnect,
            commands::cloud_list_folders,
            commands::cloud_set_folder,
            commands::get_cloud_status,
            commands::sync_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TrayLnks");
}
