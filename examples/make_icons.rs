// SPDX-License-Identifier: GPL-3.0-or-later
//! Render the app icon: assets/icon-{1024,256}.png, assets/chattrace.ico and
//! an .iconset folder for `iconutil` (see scripts/make_icons.sh).
//!   cargo run --example make_icons -- <iconset-dir>

use std::path::Path;

fn render(svg: &Path, size: u32) -> Vec<u8> {
    let data = std::fs::read(svg).expect("read svg");
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).expect("parse svg");
    let mut pm = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let s = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    pm.encode_png().expect("png")
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let big = root.join("assets/logo.svg");
    let small = root.join("assets/logo-small.svg");
    let pick = |n: u32| if n <= 48 { &small } else { &big };
    std::fs::write(root.join("assets/icon-1024.png"), render(&big, 1024)).unwrap();
    std::fs::write(root.join("assets/icon-256.png"), render(&big, 256)).unwrap();

    // Windows .ico with PNG entries.
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let pngs: Vec<Vec<u8>> = sizes.iter().map(|&n| render(pick(n), n)).collect();
    let mut ico = vec![0u8, 0, 1, 0];
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (n, png) in sizes.iter().zip(&pngs) {
        let d = if *n >= 256 { 0 } else { *n as u8 };
        ico.extend_from_slice(&[d, d, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &pngs {
        ico.extend_from_slice(png);
    }
    std::fs::write(root.join("assets/chattrace.ico"), ico).unwrap();

    // macOS iconset.
    if let Some(dir) = std::env::args().nth(1) {
        let dir = Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        for n in [16u32, 32, 128, 256, 512] {
            std::fs::write(dir.join(format!("icon_{n}x{n}.png")), render(pick(n), n)).unwrap();
            std::fs::write(dir.join(format!("icon_{n}x{n}@2x.png")), render(pick(n * 2), n * 2)).unwrap();
        }
    }
    println!("icons written");
}
