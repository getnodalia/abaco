// SPDX-License-Identifier: MIT OR Apache-2.0
//! Builds the two icon files the installers need that no tool in the repository makes otherwise,
//! from the app's portrait icon set in `assets/icons/`.
//!
//! ```text
//! cargo run -p abaco-app --example icons -- assets/app-icon      (or packaging/icons.sh)
//! ```
//!
//! Reads `assets/icons/hicolor/<n>x<n>/apps/dev.danielmiguelt.Abaco.png` (16 to 512 px) and
//! `assets/icons/abaco-1024.png`, and writes under the folder given:
//!
//! * `abaco.ico`: Windows (PNG entries from 16 to 256 px);
//! * `abaco.icns`: macOS (PNG entries from 16 to 1024 px).
//!
//! The PNGs are embedded as they are, never decoded or resampled, so the output is byte for byte
//! the same on every run. The Linux packages use `assets/icons/hicolor` directly. To change the
//! picture, replace the PNGs in `assets/icons/` (keep the names) and run this again.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const APP_ID: &str = abaco_app::APP_ID;
const ICO_SIZES: [usize; 7] = [16, 24, 32, 48, 64, 128, 256];

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The committed PNG of the icon at `size`.
fn source(size: usize) -> Result<Vec<u8>> {
    let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    let path = if size == 1024 {
        icons.join("abaco-1024.png")
    } else {
        icons
            .join("hicolor")
            .join(format!("{size}x{size}"))
            .join("apps")
            .join(format!("{APP_ID}.png"))
    };
    Ok(fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
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
    let mut data = Vec::new();
    for size in ICO_SIZES {
        data.push(source(size)?);
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
        let image = source(size)?;
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
    write(&dir.join("abaco.ico"), &ico()?)?;
    write(&dir.join("abaco.icns"), &icns()?)?;
    Ok(())
}
