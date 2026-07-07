//! Launching menu targets (`.lnk`/`.cmd`/`.ps1`) with foreground focus, and
//! opening folders in Explorer.

use std::path::Path;

/// Open a folder in Explorer via the OS default handler. No focus logic needed —
/// used by the "Open Launcher Folder" tray item.
pub fn open_folder(path: &Path) -> Result<(), String> {
    open::that(path).map_err(|e| format!("open {}: {}", path.display(), e))
}

/// Launch a menu target (`.lnk`/`.cmd`/`.ps1`) and bring its window to the
/// foreground. Non-fatal: callers log the error and keep going.
///
/// On non-Windows hosts this is a no-op so `cargo fmt`/`clippy`/`check` work.
#[cfg(windows)]
pub fn launch_target(path: &Path) -> Result<(), String> {
    crate::launch_windows::launch_and_focus(path)
        .map_err(|e| format!("launch {}: {}", path.display(), e))
}

#[cfg(not(windows))]
pub fn launch_target(_path: &Path) -> Result<(), String> {
    Ok(())
}
