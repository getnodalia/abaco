// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writes Ábaco's app icon in the files the installers need, from the same code that draws the
//! window icon (`abaco_app::icon`), so there is one picture and no drawing to keep in step.
//!
//! ```text
//! cargo run -p abaco-app --example icons -- assets/app-icon      (or packaging/icons.sh)
//! ```
//!
//! Writes, under the folder given:
//!
//! * `hicolor/<n>x<n>/apps/dev.danielmiguelt.Abaco.png`: the Linux icon theme sizes;
//! * `abaco-1024.png`: the full-size master;
//! * `abaco.ico`: Windows (PNG entries from 16 to 256 px);
//! * `abaco.icns`: macOS, on Apple's grid (an 824/1024 body inside a transparent margin).
//!
//! The output is byte for byte the same on every run. To use another picture later, replace the
//! files (or this example's source of pixels) and keep the names: the packaging scripts and
//! `ATTRIBUTION.md` refer to them.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use abaco_app::icon::rgba;

const APP_ID: &str = abaco_app::APP_ID;
const HICOLOR_SIZES: [usize; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
const ICO_SIZES: [usize; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn png(size: usize, pixels: &[u8]) -> Result<Vec<u8>> {
    let side = u32::try_from(size)?;
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels)?;
    Ok(out)
}

/// The icon at `size`, filling the square: the Windows and Linux form.
fn full(size: usize) -> Result<Vec<u8>> {
    png(size, &rgba(size))
}

/// The icon at `size` on Apple's grid: a body of 824/1024 centred in a transparent margin.
fn on_apple_grid(size: usize) -> Result<Vec<u8>> {
    let body = (size * 824 + 512) / 1024;
    let inner = rgba(body);
    let offset = (size - body) / 2;
    let mut canvas = vec![0u8; size * size * 4];
    for row in 0..body {
        let from = row * body * 4;
        let to = ((row + offset) * size + offset) * 4;
        let source = inner
            .get(from..from + body * 4)
            .ok_or("icon row out of range")?;
        let target = canvas
            .get_mut(to..to + body * 4)
            .ok_or("canvas row out of range")?;
        target.copy_from_slice(source);
    }
    png(size, &canvas)
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn ico() -> Result<Vec<u8>> {
    let images = ICO_SIZES.map(full);
    let mut data = Vec::new();
    for image in images {
        data.push(image?);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&u16::try_from(data.len())?.to_le_bytes());
    let mut offset = 6 + 16 * data.len();
    for (size, image) in ICO_SIZES.iter().zip(&data) {
        // 256 is written as 0 in the one-byte width and height fields.
        let side = u8::try_from(*size).unwrap_or(0);
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&u32::try_from(image.len())?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset)?.to_le_bytes());
        offset += image.len();
    }
    for image in &data {
        out.extend_from_slice(image);
    }
    Ok(out)
}

fn icns() -> Result<Vec<u8>> {
    // (type, pixel size): PNG payloads, the 1x and 2x sizes of 16 to 512 points.
    const TYPES: [(&[u8; 4], usize); 11] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"icp6", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
        (b"ic11", 32),
        (b"ic12", 64),
        (b"ic13", 256),
        (b"ic14", 512),
    ];
    let mut body = Vec::new();
    for (kind, size) in TYPES {
        let image = on_apple_grid(size)?;
        body.extend_from_slice(kind);
        body.extend_from_slice(&u32::try_from(image.len() + 8)?.to_be_bytes());
        body.extend_from_slice(&image);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&u32::try_from(body.len() + 8)?.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

fn main() -> Result<()> {
    let dir: PathBuf = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: icons <output folder>, e.g. assets/app-icon")?;
    for size in HICOLOR_SIZES {
        let path = dir
            .join("hicolor")
            .join(format!("{size}x{size}"))
            .join("apps")
            .join(format!("{APP_ID}.png"));
        write(&path, &full(size)?)?;
    }
    write(&dir.join("abaco-1024.png"), &full(1024)?)?;
    write(&dir.join("abaco.ico"), &ico()?)?;
    write(&dir.join("abaco.icns"), &icns()?)?;
    Ok(())
}
