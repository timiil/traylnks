// Generates a 1024×1024 source icon (blue gradient + white "play" triangle)
// for TrayLnks. Run `cargo tauri icon` on the output to build the full icon set.
//
// Supersampled at 4× then Triangle-downscaled for smooth edges.

use image::{imageops, ImageBuffer, Rgba};
use std::fs;
use std::path::PathBuf;

const OUT: u32 = 1024;
const SS: u32 = 4;

fn main() {
    let big = OUT * SS;
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(big, big);
    for y in 0..big {
        for x in 0..big {
            img.put_pixel(x, y, Rgba(pixel(x, y, big)));
        }
    }
    let small = imageops::resize(&img, OUT, OUT, imageops::FilterType::Triangle);

    let out_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/icons");
    fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("icon.png");
    small.save(&out).unwrap();
    println!("wrote {}", out.display());
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t) as u8
}

fn pixel(x: u32, y: u32, size: u32) -> [u8; 4] {
    let fx = x as f32 / size as f32;
    let fy = y as f32 / size as f32;

    // vertical gradient: light blue (top) -> indigo (bottom)
    let top = [91u8, 141, 239];
    let bot = [43u8, 58, 140];
    let mut c = [
        lerp(top[0], bot[0], fy),
        lerp(top[1], bot[1], fy),
        lerp(top[2], bot[2], fy),
    ];

    // subtle rounded "card" inset, slightly lighter, to give depth
    if in_rounded_rect(fx, fy, 0.12, 0.12, 0.88, 0.88, 0.14) {
        let card = [c[0].saturating_add(14), c[1].saturating_add(16), c[2].saturating_add(22)];
        c = card;
    }

    // white "play" triangle (launch motif)
    if in_triangle(fx, fy, 0.40, 0.28, 0.40, 0.72, 0.72, 0.50) {
        c = [255, 255, 255];
    }

    [c[0], c[1], c[2], 255]
}

fn in_rounded_rect(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if px < x0 || px > x1 || py < y0 || py > y1 {
        return false;
    }
    // corners
    let cx = if px < x0 + r { x0 + r } else if px > x1 - r { x1 - r } else { px };
    let cy = if py < y0 + r { y0 + r } else if py > y1 - r { y1 - r } else { py };
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= r * r
}

fn in_triangle(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> bool {
    let sign = |p0x: f32, p0y: f32, p1x: f32, p1y: f32, p2x: f32, p2y: f32| {
        (p0x - p2x) * (p1y - p2y) - (p1x - p2x) * (p0y - p2y)
    };
    let d1 = sign(px, py, ax, ay, bx, by);
    let d2 = sign(px, py, bx, by, cx, cy);
    let d3 = sign(px, py, cx, cy, ax, ay);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}
