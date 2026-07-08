//! Icon resolution + decoding (PRD §5).
//!
//! Priority: `.ico` > `.png` > `.jpg` > `.jpeg` > generated fallback icon.
//! v0.1 boundary: the ".lnk own icon" and "target exe icon" steps require the
//! Windows shell API and are deferred to v0.2.

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

/// Resolve a user-provided same-name icon, or assign a stable generated fallback.
pub fn load_item_icon(base_no_ext: &Path, target_path: &Path) -> Option<Image<'static>> {
    load_icon(base_no_ext).or_else(|| load_fallback_icon(target_path))
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

fn load_fallback_icon(seed_path: &Path) -> Option<Image<'static>> {
    let bytes = FALLBACK_ICON_BYTES[fallback_index(seed_path)];
    decode_bytes(bytes)
}

fn decode_bytes(bytes: &[u8]) -> Option<Image<'static>> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(Image::new_owned(rgba.into_raw(), w, h))
}

fn fallback_index(seed_path: &Path) -> usize {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in seed_path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash as usize) % FALLBACK_ICON_BYTES.len()
}

const FALLBACK_ICON_BYTES: [&[u8]; 50] = [
    include_bytes!("../assets/fallback-icons/icon-01.png"),
    include_bytes!("../assets/fallback-icons/icon-02.png"),
    include_bytes!("../assets/fallback-icons/icon-03.png"),
    include_bytes!("../assets/fallback-icons/icon-04.png"),
    include_bytes!("../assets/fallback-icons/icon-05.png"),
    include_bytes!("../assets/fallback-icons/icon-06.png"),
    include_bytes!("../assets/fallback-icons/icon-07.png"),
    include_bytes!("../assets/fallback-icons/icon-08.png"),
    include_bytes!("../assets/fallback-icons/icon-09.png"),
    include_bytes!("../assets/fallback-icons/icon-10.png"),
    include_bytes!("../assets/fallback-icons/icon-11.png"),
    include_bytes!("../assets/fallback-icons/icon-12.png"),
    include_bytes!("../assets/fallback-icons/icon-13.png"),
    include_bytes!("../assets/fallback-icons/icon-14.png"),
    include_bytes!("../assets/fallback-icons/icon-15.png"),
    include_bytes!("../assets/fallback-icons/icon-16.png"),
    include_bytes!("../assets/fallback-icons/icon-17.png"),
    include_bytes!("../assets/fallback-icons/icon-18.png"),
    include_bytes!("../assets/fallback-icons/icon-19.png"),
    include_bytes!("../assets/fallback-icons/icon-20.png"),
    include_bytes!("../assets/fallback-icons/icon-21.png"),
    include_bytes!("../assets/fallback-icons/icon-22.png"),
    include_bytes!("../assets/fallback-icons/icon-23.png"),
    include_bytes!("../assets/fallback-icons/icon-24.png"),
    include_bytes!("../assets/fallback-icons/icon-25.png"),
    include_bytes!("../assets/fallback-icons/icon-26.png"),
    include_bytes!("../assets/fallback-icons/icon-27.png"),
    include_bytes!("../assets/fallback-icons/icon-28.png"),
    include_bytes!("../assets/fallback-icons/icon-29.png"),
    include_bytes!("../assets/fallback-icons/icon-30.png"),
    include_bytes!("../assets/fallback-icons/icon-31.png"),
    include_bytes!("../assets/fallback-icons/icon-32.png"),
    include_bytes!("../assets/fallback-icons/icon-33.png"),
    include_bytes!("../assets/fallback-icons/icon-34.png"),
    include_bytes!("../assets/fallback-icons/icon-35.png"),
    include_bytes!("../assets/fallback-icons/icon-36.png"),
    include_bytes!("../assets/fallback-icons/icon-37.png"),
    include_bytes!("../assets/fallback-icons/icon-38.png"),
    include_bytes!("../assets/fallback-icons/icon-39.png"),
    include_bytes!("../assets/fallback-icons/icon-40.png"),
    include_bytes!("../assets/fallback-icons/icon-41.png"),
    include_bytes!("../assets/fallback-icons/icon-42.png"),
    include_bytes!("../assets/fallback-icons/icon-43.png"),
    include_bytes!("../assets/fallback-icons/icon-44.png"),
    include_bytes!("../assets/fallback-icons/icon-45.png"),
    include_bytes!("../assets/fallback-icons/icon-46.png"),
    include_bytes!("../assets/fallback-icons/icon-47.png"),
    include_bytes!("../assets/fallback-icons/icon-48.png"),
    include_bytes!("../assets/fallback-icons/icon-49.png"),
    include_bytes!("../assets/fallback-icons/icon-50.png"),
];
