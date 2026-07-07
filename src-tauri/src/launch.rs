//! Launching `.lnk` items and opening folders via the OS default handler.
//!
//! On Windows the `open` crate calls `ShellExecuteW`, which resolves MS shell
//! shortcuts natively — we never read or rebuild the `.lnk` target ourselves
//! (PRD §4: "Windows Shell is the executor").

use std::path::Path;

/// Open `path` with the system default handler (launches a `.lnk`, or opens a
/// folder in Explorer). Non-fatal: callers log the error and keep going.
pub fn open(path: &Path) -> Result<(), String> {
    open::that(path).map_err(|e| format!("open {}: {}", path.display(), e))
}
