// SPDX-License-Identifier: MIT OR Apache-2.0
//! The app icon, drawn here from shapes: a counting frame (an abacus) in white on Ábaco's
//! accent, `#A23B72`. Nothing is read from a file; the picture is original work, flat, with no
//! gradients or glow, and the same pixels are the window icon, the logo in the title bar and
//! the mark on the About page.

/// Ábaco's accent as the icon's background.
pub const ACCENT: [u8; 3] = [0xA2, 0x3B, 0x72];

/// A rounded rectangle's signed distance: negative inside.
fn rounded_rect(p: (f32, f32), centre: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let qx = (p.0 - centre.0).abs() - (half.0 - radius);
    let qy = (p.1 - centre.1).abs() - (half.1 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// An ellipse's approximate signed distance: negative inside.
fn ellipse(p: (f32, f32), centre: (f32, f32), radii: (f32, f32)) -> f32 {
    let x = (p.0 - centre.0) / radii.0;
    let y = (p.1 - centre.1) / radii.1;
    ((x * x + y * y).sqrt() - 1.0) * radii.0.min(radii.1)
}

/// The colour at `(u, v)` in the unit square, as `[r, g, b, a]` with alpha 0 or 1.
fn sample(u: f32, v: f32) -> [f32; 4] {
    let p = (u, v);
    if rounded_rect(p, (0.5, 0.5), (0.5, 0.5), 0.22) > 0.0 {
        return [0.0; 4];
    }
    let white = [1.0, 1.0, 1.0, 1.0];
    let accent = [
        f32::from(ACCENT[0]) / 255.0,
        f32::from(ACCENT[1]) / 255.0,
        f32::from(ACCENT[2]) / 255.0,
        1.0,
    ];
    let mut colour = accent;
    // The frame: an outline.
    let outer = rounded_rect(p, (0.5, 0.5), (0.31, 0.29), 0.045);
    if outer <= 0.0 && outer > -0.038 {
        colour = white;
    }
    // Four rods, each carrying five beads split into two groups.
    let rods = [0.335, 0.445, 0.555, 0.665];
    let left_counts = [2, 1, 3, 4];
    for (rod, left) in rods.iter().zip(left_counts) {
        if (v - rod).abs() < 0.006 && (0.2..=0.8).contains(&u) {
            colour = [1.0, 1.0, 1.0, 1.0];
        }
        for bead in 0..5 {
            let x = if bead < left {
                0.262 + 0.082 * bead as f32
            } else {
                0.738 - 0.082 * (4 - bead) as f32
            };
            if ellipse(p, (x, *rod), (0.036, 0.046)) <= 0.0 {
                colour = white;
            }
        }
    }
    colour
}

/// The icon as `size * size * 4` bytes of straight (not premultiplied) RGBA.
pub fn rgba(size: usize) -> Vec<u8> {
    const SAMPLES: usize = 3;
    let mut out = Vec::with_capacity(size * size * 4);
    let n = SAMPLES as f32;
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = (x as f32 + (sx as f32 + 0.5) / n) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / n) / size as f32;
                    let c = sample(u, v);
                    for k in 0..3 {
                        sum[k] += c[k] * c[3];
                    }
                    sum[3] += c[3];
                }
            }
            let alpha = sum[3] / (n * n);
            for k in 0..3 {
                let channel = if sum[3] > 0.0 { sum[k] / sum[3] } else { 0.0 };
                out.push((channel * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            out.push((alpha * 255.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_a_rounded_square_of_the_accent_with_a_white_frame() {
        let size = 64;
        let px = rgba(size);
        assert_eq!(px.len(), size * size * 4);
        let at = |x: usize, y: usize| &px[(y * size + x) * 4..(y * size + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "the corner is transparent");
        assert_eq!(at(size / 2, 3), &[ACCENT[0], ACCENT[1], ACCENT[2], 255]);
        // The frame's left edge is white.
        assert_eq!(
            at((0.2 * size as f32) as usize, size / 2)[..3],
            [255, 255, 255]
        );
        let (pixels, rest) = px.as_chunks::<4>();
        assert!(rest.is_empty());
        assert!(pixels
            .iter()
            .any(|p| p[..3] == [255, 255, 255] && p[3] == 255));
    }
}
