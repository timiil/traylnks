//! Icon resolution + decoding (PRD §5).
//!
//! Priority: `.ico` > `.png` > `.jpg` > `.jpeg` > (deferred) > system default.
//! v0.1 boundary: the ".lnk own icon" and "target exe icon" steps require the
//! Windows shell API and are deferred to v0.2 — we fall back to `None`
//! (system-default menu item rendering) after `.jpeg`.

use std::path::{Path, PathBuf};
use tauri::image::Image;

/// Same-name image extensions in priority order.
const EXT_PRIORITY: &[&str] = &["ico", "png", "jpg", "jpeg"];

/// Longest edge we shrink large source images down to for menu use.
const MAX_EDGE: u32 = 64;

/// Find the highest-priority existing same-name image for `base_no_ext`.
pub fn resolve_icon_path(base_no_ext: &Path) -> Option<PathBuf> {
    for ext in EXT_PRIORITY {
        let candidate = base_no_ext.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolve + decode a same-name icon into an owned RGBA `Image<'static>`.
/// Any failure (missing, unreadable, unsupported, corrupt) → `None`.
pub fn load_icon(base_no_ext: &Path) -> Option<Image<'static>> {
    let path = resolve_icon_path(base_no_ext)?;
    match decode(&path) {
        Some(img) => Some(img),
        None => {
            log::warn!("failed to decode icon: {}", path.display());
            None
        }
    }
}

fn decode(path: &Path) -> Option<Image<'static>> {
    // The `image` crate auto-detects format (png/jpeg/bmp/ico via features).
    let img = image::open(path).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let (w2, h2, bytes) = maybe_resize(&rgba, w, h);
    Some(Image::new_owned(bytes, w2, h2))
}

/// Downscale so the longest edge is at most `MAX_EDGE`; small images are copied as-is.
fn maybe_resize(rgba: &image::RgbaImage, w: u32, h: u32) -> (u32, u32, Vec<u8>) {
    let longest = w.max(h);
    if longest <= MAX_EDGE {
        return (w, h, rgba.as_raw().clone());
    }
    let scale = MAX_EDGE as f32 / longest as f32;
    let nw = (((w as f32) * scale).round() as u32).max(1);
    let nh = (((h as f32) * scale).round() as u32).max(1);
    let resized = image::imageops::resize(rgba, nw, nh, image::imageops::FilterType::Triangle);
    (nw, nh, resized.into_raw())
}
