//! Recursive file watcher with ~500ms debounce (PRD §9).
//!
//! The debouncer callback does no menu work itself — it just triggers a rebuild.
//! The returned `Debouncer` is stored in `AppState` so it is never dropped
//! (dropping it would silently stop watching).

use crate::state::AppState;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Manager, Wry};

/// Platform-recommended watcher debouncer (Windows: ReadDirectoryChangesWatcher).
pub type AppDebouncer = notify_debouncer_full::Debouncer<
    notify::RecommendedWatcher,
    notify_debouncer_full::FileIdMap,
>;

const DEBOUNCE_MS: u64 = 500;

/// Stop any existing watcher and start a fresh one on `watch_path` (if given).
pub fn restart(app: &AppHandle<Wry>, watch_path: Option<PathBuf>) {
    // drop the previous watcher
    *app.state::<AppState>().watcher.lock() = None;

    match watch_path {
        Some(wp) if wp.is_dir() => match start(app.clone(), wp) {
            Ok(debouncer) => {
                *app.state::<AppState>().watcher.lock() = Some(debouncer);
                app.state::<AppState>()
                    .watcher_ok
                    .store(true, Ordering::SeqCst);
            }
            Err(e) => {
                log::error!("watcher start failed: {e}");
                app.state::<AppState>()
                    .watcher_ok
                    .store(false, Ordering::SeqCst);
            }
        },
        _ => {
            app.state::<AppState>()
                .watcher_ok
                .store(false, Ordering::SeqCst);
        }
    }
}

fn start(app: AppHandle<Wry>, watch_path: PathBuf) -> notify::Result<AppDebouncer> {
    let cb_app = app.clone();
    let mut debouncer = notify_debouncer_full::new_debouncer(
        Duration::from_millis(DEBOUNCE_MS),
        None,
        move |_events| {
            // Any create/delete/modify/rename in the tree → rebuild the menu.
            crate::tray::rebuild_now(&cb_app);
        },
    )?;
    debouncer
        .watcher()
        .watch(watch_path.as_path(), RecursiveMode::Recursive)?;
    log::info!("watching {}", watch_path.display());
    Ok(debouncer)
}
