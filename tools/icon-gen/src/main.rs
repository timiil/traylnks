//! Writes the TrayLnks source app icon into `src-tauri/icons/icon.png`.
//!
//! The source artwork is an AI-generated 1024x1024 PNG kept in
//! `tools/icon-gen/assets/traylnks-icon-source.png`. Run `cargo tauri icon` on
//! the output to build the full Tauri icon set.

use image::GenericImageView;
use std::fs;
use std::path::PathBuf;

const OUT: u32 = 1024;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest_dir.join("assets/traylnks-icon-source.png");
    let out_dir = manifest_dir.join("../../src-tauri/icons");
    let out = out_dir.join("icon.png");

    let img = image::open(&source)
        .unwrap_or_else(|e| panic!("failed to read source icon {}: {e}", source.display()));
    let (w, h) = img.dimensions();
    assert_eq!(
        (w, h),
        (OUT, OUT),
        "source icon must be {OUT}x{OUT}, got {w}x{h}"
    );

    fs::create_dir_all(&out_dir).unwrap();
    fs::copy(&source, &out).unwrap_or_else(|e| {
        panic!(
            "failed to copy {} to {}: {e}",
            source.display(),
            out.display()
        )
    });
    println!("wrote {}", out.display());
}
