// SPDX-License-Identifier: MIT OR Apache-2.0
//! The arithmetic of drawing a figure, with no drawing in it: line specifications such as
//! `r--o`, round tick values, the range an axes covers and the projection of 3-D data. The
//! window's figure panel uses these and only paints.

use abaco_script::plot::{Axes, Kind};

/// A colour named by a `plot` specification. The window maps each to a theme token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Red,
    Green,
    Blue,
    Cyan,
    Magenta,
    Yellow,
    Black,
    White,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dash {
    Solid,
    Dashed,
    Dotted,
    DashDot,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Circle,
    Plus,
    Star,
    Point,
    Cross,
    Square,
    Diamond,
    Up,
    Down,
}

/// What a `plot` specification such as `r--o` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSpec {
    pub hue: Option<Hue>,
    pub dash: Dash,
    pub marker: Option<Marker>,
}

impl LineSpec {
    /// Reads a specification. Anything it does not know is ignored, as `plot` does.
    pub fn parse(spec: &str) -> LineSpec {
        let mut out = LineSpec {
            hue: None,
            dash: Dash::Solid,
            marker: None,
        };
        let mut explicit_dash = false;
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;
        while let Some(&c) = chars.get(i) {
            let next = chars.get(i + 1).copied();
            match c {
                '-' if next == Some('-') => {
                    out.dash = Dash::Dashed;
                    explicit_dash = true;
                    i += 1;
                }
                '-' if next == Some('.') => {
                    out.dash = Dash::DashDot;
                    explicit_dash = true;
                    i += 1;
                }
                '-' => {
                    out.dash = Dash::Solid;
                    explicit_dash = true;
                }
                ':' => {
                    out.dash = Dash::Dotted;
                    explicit_dash = true;
                }
                'r' => out.hue = Some(Hue::Red),
                'g' => out.hue = Some(Hue::Green),
                'b' => out.hue = Some(Hue::Blue),
                'c' => out.hue = Some(Hue::Cyan),
                'm' => out.hue = Some(Hue::Magenta),
                'y' => out.hue = Some(Hue::Yellow),
                'k' => out.hue = Some(Hue::Black),
                'w' => out.hue = Some(Hue::White),
                'o' => out.marker = Some(Marker::Circle),
                '+' => out.marker = Some(Marker::Plus),
                '*' => out.marker = Some(Marker::Star),
                '.' => out.marker = Some(Marker::Point),
                'x' => out.marker = Some(Marker::Cross),
                's' => out.marker = Some(Marker::Square),
                'd' => out.marker = Some(Marker::Diamond),
                '^' => out.marker = Some(Marker::Up),
                'v' => out.marker = Some(Marker::Down),
                _ => {}
            }
            i += 1;
        }
        // A marker alone draws no line.
        if out.marker.is_some() && !explicit_dash {
            out.dash = Dash::None;
        }
        out
    }
}

/// Round values to label an axis from `lo` to `hi`, about `target` of them.
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo || target == 0 {
        return Vec::new();
    }
    let raw = (hi - lo) / target as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    let first = (lo / step).ceil();
    let last = (hi / step).floor();
    if !(first.is_finite() && last.is_finite()) || last - first > 1000.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut k = first;
    while k <= last {
        // Multiplying avoids the drift of repeated addition (0.30000000000000004).
        let v = k * step;
        out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        k += 1.0;
    }
    out
}

/// Powers of ten covering `lo..=hi` for a logarithmic axis (both positive).
pub fn log_ticks(lo: f64, hi: f64) -> Vec<f64> {
    if !(lo > 0.0 && hi > lo && lo.is_finite() && hi.is_finite()) {
        return Vec::new();
    }
    let (a, b) = (lo.log10().ceil() as i32, hi.log10().floor() as i32);
    if b - a > 300 {
        return Vec::new();
    }
    (a..=b).map(|e| 10f64.powi(e)).collect()
}

/// A tick value as short text.
pub fn tick_text(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    let a = v.abs();
    if !(1e-4..1e6).contains(&a) {
        let s = format!("{v:.3e}");
        // 1.000e3 → 1e3
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        return format!("{m}e{e}");
    }
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The range an axes covers on one axis, with room around the data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub lo: f64,
    pub hi: f64,
}

impl Range {
    fn from_values(values: impl Iterator<Item = f64>, log: bool) -> Option<Range> {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for v in values {
            if !v.is_finite() || (log && v <= 0.0) {
                continue;
            }
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo <= hi).then_some(Range { lo, hi })
    }

    /// Pads the range and widens one that is a single value.
    fn padded(self, log: bool, pad: f64) -> Range {
        if self.hi - self.lo <= f64::EPSILON * self.lo.abs().max(1.0) {
            return if log {
                Range {
                    lo: self.lo / 2.0,
                    hi: self.hi * 2.0,
                }
            } else {
                let w = self.lo.abs().max(1.0) * 0.5;
                Range {
                    lo: self.lo - w,
                    hi: self.hi + w,
                }
            };
        }
        if log {
            let (a, b) = (self.lo.log10(), self.hi.log10());
            let p = (b - a) * pad;
            Range {
                lo: 10f64.powf(a - p),
                hi: 10f64.powf(b + p),
            }
        } else {
            let p = (self.hi - self.lo) * pad;
            Range {
                lo: self.lo - p,
                hi: self.hi + p,
            }
        }
    }
}

/// Bars and stems start at zero, so zero belongs in their range.
fn includes_zero(kind: Kind) -> bool {
    matches!(kind, Kind::Bar | Kind::Hist | Kind::Stem)
}

/// The horizontal and vertical range to draw `axes` in, honouring `xlim`, `ylim` and the log
/// settings. `None` for an axes with nothing to draw.
pub fn ranges(axes: &Axes) -> Option<(Range, Range)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for s in &axes.series {
        xs.extend(s.x.iter().copied());
        ys.extend(s.y.iter().copied());
        if includes_zero(s.kind) {
            ys.push(0.0);
        }
    }
    for s in &axes.surfaces {
        xs.extend(s.x.iter().copied());
        ys.extend(s.y.iter().copied());
    }
    let fit = |values: Vec<f64>, log: bool, limit: Option<(f64, f64)>| -> Option<Range> {
        if let Some((a, b)) = limit {
            if a.is_finite() && b.is_finite() && b > a && !(log && a <= 0.0) {
                return Some(Range { lo: a, hi: b });
            }
        }
        Range::from_values(values.into_iter(), log).map(|r| r.padded(log, 0.04))
    };
    let x = fit(xs, axes.xlog, axes.xlim);
    let y = fit(ys, axes.ylog, axes.ylim);
    match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    }
}

/// Maps `v` in `range` to 0..=1 (logarithmically when `log`).
pub fn unit(v: f64, range: Range, log: bool) -> f64 {
    if log {
        if v <= 0.0 || range.lo <= 0.0 {
            return f64::NAN;
        }
        (v.log10() - range.lo.log10()) / (range.hi.log10() - range.lo.log10())
    } else {
        (v - range.lo) / (range.hi - range.lo)
    }
}

/// Whether `axes` is a 3-D drawing.
pub fn is_3d(axes: &Axes) -> bool {
    axes.series.iter().any(|s| s.kind == Kind::Line3) || axes.surfaces.iter().any(|s| !s.flat)
}

/// The data's cube: ranges of x, y and z for a 3-D axes.
pub fn ranges_3d(axes: &Axes) -> Option<[Range; 3]> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut zs = Vec::new();
    for s in &axes.series {
        xs.extend(s.x.iter().copied());
        ys.extend(s.y.iter().copied());
        zs.extend(s.z.iter().copied());
    }
    for s in &axes.surfaces {
        xs.extend(s.x.iter().copied());
        ys.extend(s.y.iter().copied());
        zs.extend(s.z.iter().copied());
    }
    let r = |v: Vec<f64>| Range::from_values(v.into_iter(), false).map(|r| r.padded(false, 0.0));
    Some([r(xs)?, r(ys)?, r(zs)?])
}

/// A view of a 3-D axes: azimuth and elevation in degrees, as MATLAB's `view` (default −37.5, 30).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub azimuth: f64,
    pub elevation: f64,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera {
            azimuth: -37.5,
            elevation: 30.0,
        }
    }
}

impl Camera {
    /// Projects a point of the unit cube (each coordinate 0..=1) to the screen: `(right, up,
    /// depth)`, with the cube centred on the origin and `depth` growing away from the viewer.
    pub fn project(self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let (x, y, z) = (x - 0.5, y - 0.5, z - 0.5);
        let (sa, ca) = self.azimuth.to_radians().sin_cos();
        let (se, ce) = self.elevation.to_radians().sin_cos();
        let right = x * ca + y * sa;
        let depth_plane = -x * sa + y * ca;
        let up = z * ce - depth_plane * se;
        let depth = depth_plane * ce + z * se;
        (right, up, depth)
    }

    /// Turns the view by a drag of `dx`, `dy` pixels.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        self.azimuth = (self.azimuth - dx * 0.5).rem_euclid(360.0);
        if self.azimuth > 180.0 {
            self.azimuth -= 360.0;
        }
        self.elevation = (self.elevation + dy * 0.5).clamp(-90.0, 90.0);
    }
}

/// The 0-based row and column of cell `index` of a grid with `cols` columns.
pub fn grid_cell(index: usize, cols: usize) -> (usize, usize) {
    let cols = cols.max(1);
    (index / cols, index % cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use abaco_script::plot::Series;

    fn series(kind: Kind, x: &[f64], y: &[f64]) -> Series {
        Series {
            kind,
            x: x.to_vec(),
            y: y.to_vec(),
            z: Vec::new(),
            spec: String::new(),
            label: String::new(),
        }
    }

    #[test]
    fn specs_read_colour_dash_and_marker() {
        let s = LineSpec::parse("r--o");
        assert_eq!(s.hue, Some(Hue::Red));
        assert_eq!(s.dash, Dash::Dashed);
        assert_eq!(s.marker, Some(Marker::Circle));
        let s = LineSpec::parse("b:");
        assert_eq!(
            (s.hue, s.dash, s.marker),
            (Some(Hue::Blue), Dash::Dotted, None)
        );
        let s = LineSpec::parse("g-.");
        assert_eq!(s.dash, Dash::DashDot);
        let s = LineSpec::parse("k*");
        assert_eq!(s.dash, Dash::None, "a marker alone draws no line");
        assert_eq!(LineSpec::parse("").dash, Dash::Solid);
        assert_eq!(LineSpec::parse("zzz").hue, None);
    }

    #[test]
    fn ticks_are_round_and_inside_the_range() {
        assert_eq!(
            nice_ticks(0.0, 10.0, 5),
            vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
        let t = nice_ticks(-1.0, 1.0, 4);
        assert_eq!(t, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        let t = nice_ticks(0.0, 0.3, 3);
        assert!(t.iter().all(|v| (0.0..=0.3 + 1e-12).contains(v)), "{t:?}");
        assert!(t.contains(&0.1) && t.contains(&0.2), "no drift: {t:?}");
        assert!(nice_ticks(1.0, 1.0, 5).is_empty());
        assert!(nice_ticks(f64::NAN, 1.0, 5).is_empty());
        assert!(nice_ticks(0.0, 1e300, 5).len() <= 1001);
    }

    #[test]
    fn log_ticks_are_powers_of_ten() {
        assert_eq!(log_ticks(0.5, 2000.0), vec![1.0, 10.0, 100.0, 1000.0]);
        assert!(log_ticks(-1.0, 10.0).is_empty());
        assert!(log_ticks(1e-300, 1e300).is_empty() || log_ticks(1e-300, 1e300).len() <= 601);
    }

    #[test]
    fn tick_text_is_short() {
        assert_eq!(tick_text(0.0), "0");
        assert_eq!(tick_text(2.5), "2.5");
        assert_eq!(tick_text(100.0), "100");
        assert_eq!(tick_text(1e6), "1e6");
        assert_eq!(tick_text(0.00002), "2e-5");
    }

    #[test]
    fn ranges_pad_the_data_and_honour_limits() {
        let mut a = Axes::default();
        a.series.push(series(Kind::Line, &[0.0, 10.0], &[1.0, 3.0]));
        let (x, y) = ranges(&a).unwrap();
        assert!(x.lo < 0.0 && x.hi > 10.0);
        assert!(y.lo < 1.0 && y.hi > 3.0);
        a.xlim = Some((2.0, 4.0));
        assert_eq!(ranges(&a).unwrap().0, Range { lo: 2.0, hi: 4.0 });
        assert!(ranges(&Axes::default()).is_none());
    }

    #[test]
    fn bars_include_zero_and_a_flat_line_gets_room() {
        let mut a = Axes::default();
        a.series.push(series(Kind::Bar, &[1.0, 2.0], &[5.0, 6.0]));
        assert!(ranges(&a).unwrap().1.lo <= 0.0);
        let mut b = Axes::default();
        b.series.push(series(Kind::Line, &[0.0, 1.0], &[2.0, 2.0]));
        let y = ranges(&b).unwrap().1;
        assert!(y.hi > y.lo);
    }

    #[test]
    fn non_finite_values_and_log_axes_are_survived() {
        let mut a = Axes::default();
        a.series.push(series(
            Kind::Line,
            &[f64::NAN, 1.0, 10.0, -5.0],
            &[1.0, f64::INFINITY, 2.0, 3.0],
        ));
        a.xlog = true;
        let (x, _) = ranges(&a).unwrap();
        assert!(x.lo > 0.0 && x.hi >= 10.0);
        assert!(unit(-1.0, x, true).is_nan());
        let u = unit(1.0, Range { lo: 1.0, hi: 100.0 }, true);
        assert!(u.abs() < 1e-12);
        assert!((unit(10.0, Range { lo: 1.0, hi: 100.0 }, true) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn the_camera_projects_the_cube() {
        let cam = Camera {
            azimuth: 0.0,
            elevation: 0.0,
        };
        let (r, u, d) = cam.project(1.0, 0.5, 1.0);
        assert!((r - 0.5).abs() < 1e-12 && (u - 0.5).abs() < 1e-12 && d.abs() < 1e-12);
        // From above, z no longer moves the point up the screen.
        let top = Camera {
            azimuth: 0.0,
            elevation: 90.0,
        };
        let (_, up0, _) = top.project(0.5, 0.5, 0.0);
        let (_, up1, _) = top.project(0.5, 0.5, 1.0);
        assert!((up0 - up1).abs() < 1e-12);
        let mut c = Camera::default();
        c.orbit(1e6, 1e6);
        assert!(c.elevation <= 90.0 && c.azimuth.abs() <= 180.0);
    }

    #[test]
    fn three_d_is_detected_and_grid_cells_are_row_major() {
        let mut a = Axes::default();
        assert!(!is_3d(&a));
        let mut s = series(Kind::Line3, &[0.0, 1.0], &[0.0, 1.0]);
        s.z = vec![0.0, 1.0];
        a.series.push(s);
        assert!(is_3d(&a));
        assert!(ranges_3d(&a).is_some());
        assert_eq!(grid_cell(5, 3), (1, 2));
        assert_eq!(grid_cell(0, 0), (0, 0));
    }
}
