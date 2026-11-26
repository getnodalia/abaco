// SPDX-License-Identifier: MIT OR Apache-2.0
//! Draws the figures a script asked for. The interpreter only records series and surfaces; the
//! arithmetic (ticks, ranges, line specifications, the 3-D camera) is in
//! [`abaco_app::script::plot`], and this paints the result with theme colours.

use abaco_app::script::plot::{
    grid_cell, is_3d, log_ticks, nice_ticks, ranges, ranges_3d, tick_text, unit, Camera, Dash, Hue,
    LineSpec, Marker, Range,
};
use abaco_script::plot::{Axes, Figure, Kind, Series};
use egui::epaint::TextShape;
use egui::{pos2, vec2, Align2, Color32, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::color;

/// The view state of the figure panel that the interpreter knows nothing about.
#[derive(Default)]
pub struct FigureView {
    pub camera: Camera,
}

/// One cell of a surface, projected.
struct Quad {
    depth: f64,
    pts: [Pos2; 4],
    level: f64,
    last_col: bool,
    last_row: bool,
}

/// Longest polyline drawn in one piece; longer ones are thinned to this many points.
const MAX_POINTS: usize = 20_000;

/// The colours series cycle through, as theme tokens.
fn cycle(look: &Look, i: usize) -> Color32 {
    let p = &look.palette;
    let all = [
        p.accent_default,
        p.status_danger_fg,
        p.status_success_fg,
        p.status_warning_fg,
        p.status_info_fg,
        p.text_secondary,
    ];
    color(all[i % all.len()])
}

fn hue_color(look: &Look, hue: Hue) -> Color32 {
    let p = &look.palette;
    color(match hue {
        Hue::Red => p.status_danger_fg,
        Hue::Green => p.status_success_fg,
        Hue::Blue => p.status_info_fg,
        Hue::Cyan => p.accent_default,
        Hue::Magenta => p.text_link,
        Hue::Yellow => p.status_warning_fg,
        Hue::Black => p.text_primary,
        Hue::White => p.text_tertiary,
    })
}

/// Blue to green to amber to red, for heights.
fn ramp(look: &Look, t: f64) -> Color32 {
    let p = &look.palette;
    let stops = [
        color(p.status_info_fg),
        color(p.status_success_fg),
        color(p.status_warning_fg),
        color(p.status_danger_fg),
    ];
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let x = t * (stops.len() - 1) as f64;
    let i = (x.floor() as usize).min(stops.len() - 2);
    stops[i].lerp_to_gamma(stops[i + 1], (x - i as f64) as f32)
}

/// Draws `figure` filling `rect`; dragging a 3-D axes turns the view.
pub fn show(ui: &mut Ui, look: &Look, figure: &Figure, view: &mut FigureView, id: Id) {
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let painter = ui.painter_at(rect);
    let rows = figure.rows.clamp(1, 8);
    let cols = figure.cols.clamp(1, 8);
    let cell = vec2(rect.width() / cols as f32, rect.height() / rows as f32);
    for (i, axes) in figure.axes.iter().enumerate().take(rows * cols) {
        let (r, c) = grid_cell(i, cols);
        let area = Rect::from_min_size(rect.min + vec2(c as f32 * cell.x, r as f32 * cell.y), cell)
            .shrink(6.0);
        if area.width() < 60.0 || area.height() < 60.0 {
            continue;
        }
        if is_3d(axes) {
            let plot_rect = frame(&painter, look, area, axes, false);
            let response = ui.interact(plot_rect, id.with(("axes", i)), Sense::drag());
            if response.dragged() {
                let d = response.drag_delta();
                view.camera.orbit(f64::from(d.x), f64::from(d.y));
            }
            draw_3d(
                &painter.with_clip_rect(plot_rect),
                look,
                plot_rect,
                axes,
                view.camera,
            );
        } else {
            let plot_rect = frame(&painter, look, area, axes, true);
            draw_2d(&painter, look, plot_rect, axes);
            legend(&painter, look, plot_rect, axes);
        }
    }
}

fn text(painter: &Painter, at: Pos2, anchor: Align2, s: &str, font: egui::FontId, ink: Color32) {
    painter.text(at, anchor, s, font, ink);
}

/// The plotting rectangle inside `area`, with the title and labels drawn around it.
fn frame(painter: &Painter, look: &Look, area: Rect, axes: &Axes, ticks: bool) -> Rect {
    let p = &look.palette;
    let left = if ticks { 52.0 } else { 14.0 } + if axes.ylabel.is_empty() { 0.0 } else { 16.0 };
    let bottom = if ticks { 26.0 } else { 12.0 } + if axes.xlabel.is_empty() { 0.0 } else { 18.0 };
    let top = if axes.title.is_empty() { 8.0 } else { 28.0 };
    let plot = Rect::from_min_max(
        pos2(area.left() + left, area.top() + top),
        pos2(area.right() - 10.0, area.bottom() - bottom),
    );
    if plot.width() < 20.0 || plot.height() < 20.0 {
        return Rect::from_min_size(area.min, vec2(1.0, 1.0));
    }
    painter.rect_filled(plot, 4.0, color(p.control_field_bg));
    painter.rect_stroke(
        plot,
        4.0,
        Stroke::new(1.0, color(p.border_default)),
        egui::StrokeKind::Inside,
    );
    if !axes.title.is_empty() {
        text(
            painter,
            pos2(plot.center().x, area.top() + 4.0),
            Align2::CENTER_TOP,
            &axes.title,
            eng::label(),
            color(p.text_primary),
        );
    }
    if !axes.xlabel.is_empty() {
        text(
            painter,
            pos2(plot.center().x, area.bottom() - 2.0),
            Align2::CENTER_BOTTOM,
            &axes.xlabel,
            eng::caption(),
            color(p.text_secondary),
        );
    }
    if !axes.ylabel.is_empty() {
        let galley =
            painter.layout_no_wrap(axes.ylabel.clone(), eng::caption(), color(p.text_secondary));
        let at = pos2(area.left() + 2.0, plot.center().y + galley.size().x / 2.0);
        painter.add(
            TextShape::new(at, galley, color(p.text_secondary))
                .with_angle(-std::f32::consts::FRAC_PI_2),
        );
    }
    plot
}

fn thin(len: usize) -> usize {
    len.div_ceil(MAX_POINTS).max(1)
}

fn stroke_for(look: &Look, spec: &LineSpec, index: usize) -> Stroke {
    let ink = spec
        .hue
        .map_or_else(|| cycle(look, index), |h| hue_color(look, h));
    Stroke::new(1.75, ink)
}

fn polyline(painter: &Painter, points: &[Pos2], stroke: Stroke, dash: Dash) {
    if points.len() < 2 {
        return;
    }
    match dash {
        Dash::Solid => {
            painter.add(Shape::line(points.to_vec(), stroke));
        }
        Dash::Dashed => {
            painter.extend(Shape::dashed_line(points, stroke, 8.0, 5.0));
        }
        Dash::Dotted => {
            painter.extend(Shape::dashed_line(points, stroke, 1.5, 4.0));
        }
        Dash::DashDot => {
            painter.extend(Shape::dashed_line(points, stroke, 8.0, 4.0));
        }
        Dash::None => {}
    }
}

fn marker(painter: &Painter, at: Pos2, kind: Marker, ink: Color32, fill: Color32) {
    let r = 3.5;
    let s = Stroke::new(1.5, ink);
    match kind {
        Marker::Circle => {
            painter.circle(at, r, fill, s);
        }
        Marker::Point => {
            painter.circle_filled(at, 2.0, ink);
        }
        Marker::Plus => {
            painter.line_segment([at - vec2(r, 0.0), at + vec2(r, 0.0)], s);
            painter.line_segment([at - vec2(0.0, r), at + vec2(0.0, r)], s);
        }
        Marker::Cross => {
            painter.line_segment([at - vec2(r, r), at + vec2(r, r)], s);
            painter.line_segment([at - vec2(r, -r), at + vec2(r, -r)], s);
        }
        Marker::Star => {
            painter.line_segment([at - vec2(r, 0.0), at + vec2(r, 0.0)], s);
            painter.line_segment([at - vec2(0.0, r), at + vec2(0.0, r)], s);
            painter.line_segment([at - vec2(r, r) * 0.7, at + vec2(r, r) * 0.7], s);
            painter.line_segment([at - vec2(r, -r) * 0.7, at + vec2(r, -r) * 0.7], s);
        }
        Marker::Square => {
            painter.rect(
                Rect::from_center_size(at, vec2(2.0 * r, 2.0 * r)),
                0.0,
                fill,
                s,
                egui::StrokeKind::Middle,
            );
        }
        Marker::Diamond => {
            painter.add(Shape::convex_polygon(
                vec![
                    at + vec2(0.0, -r - 1.0),
                    at + vec2(r + 1.0, 0.0),
                    at + vec2(0.0, r + 1.0),
                    at + vec2(-r - 1.0, 0.0),
                ],
                fill,
                s,
            ));
        }
        Marker::Up => {
            painter.add(Shape::convex_polygon(
                vec![
                    at + vec2(0.0, -r - 1.0),
                    at + vec2(r + 1.0, r),
                    at + vec2(-r - 1.0, r),
                ],
                fill,
                s,
            ));
        }
        Marker::Down => {
            painter.add(Shape::convex_polygon(
                vec![
                    at + vec2(0.0, r + 1.0),
                    at + vec2(r + 1.0, -r),
                    at + vec2(-r - 1.0, -r),
                ],
                fill,
                s,
            ));
        }
    }
}

// --- Two dimensions ---------------------------------------------------------------------------

struct Map {
    rect: Rect,
    x: Range,
    y: Range,
    xlog: bool,
    ylog: bool,
}

impl Map {
    fn at(&self, x: f64, y: f64) -> Option<Pos2> {
        let u = unit(x, self.x, self.xlog);
        let v = unit(y, self.y, self.ylog);
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        // Far-off values are clamped so the painter never gets a coordinate it cannot draw.
        let u = u.clamp(-50.0, 50.0) as f32;
        let v = v.clamp(-50.0, 50.0) as f32;
        Some(pos2(
            self.rect.left() + u * self.rect.width(),
            self.rect.bottom() - v * self.rect.height(),
        ))
    }
}

fn axis_ticks(range: Range, log: bool, target: usize) -> Vec<f64> {
    if log {
        log_ticks(range.lo, range.hi)
    } else {
        nice_ticks(range.lo, range.hi, target)
    }
}

/// Draws the 2-D axes in `rect`. `full` clips to the whole figure: the curves are clipped to the
/// plotting rectangle here, the tick labels outside it are not.
fn draw_2d(full: &Painter, look: &Look, rect: Rect, axes: &Axes) {
    let p = &look.palette;
    let painter = &full.with_clip_rect(rect);
    let Some((mut rx, mut ry)) = ranges(axes) else {
        text(
            painter,
            rect.center(),
            Align2::CENTER_CENTER,
            "—",
            eng::body(),
            color(p.text_tertiary),
        );
        return;
    };
    if axes.equal && !axes.xlog && !axes.ylog {
        // The same number of units per pixel on both axes.
        let (sx, sy) = (
            (rx.hi - rx.lo) / f64::from(rect.width()),
            (ry.hi - ry.lo) / f64::from(rect.height()),
        );
        let s = sx.max(sy);
        let (cx, cy) = ((rx.lo + rx.hi) / 2.0, (ry.lo + ry.hi) / 2.0);
        rx = Range {
            lo: cx - s * f64::from(rect.width()) / 2.0,
            hi: cx + s * f64::from(rect.width()) / 2.0,
        };
        ry = Range {
            lo: cy - s * f64::from(rect.height()) / 2.0,
            hi: cy + s * f64::from(rect.height()) / 2.0,
        };
    }
    let map = Map {
        rect,
        x: rx,
        y: ry,
        xlog: axes.xlog,
        ylog: axes.ylog,
    };
    let xt = axis_ticks(rx, axes.xlog, (rect.width() / 80.0) as usize + 1);
    let yt = axis_ticks(ry, axes.ylog, (rect.height() / 45.0) as usize + 1);
    let grid = Stroke::new(1.0, color(p.border_subtle));
    for &v in &xt {
        if let Some(pt) = map.at(v, ry.lo) {
            if axes.grid {
                painter.line_segment([pos2(pt.x, rect.top()), pos2(pt.x, rect.bottom())], grid);
            }
            painter.line_segment(
                [pos2(pt.x, rect.bottom()), pos2(pt.x, rect.bottom() - 4.0)],
                Stroke::new(1.0, color(p.border_strong)),
            );
        }
    }
    for &v in &yt {
        if let Some(pt) = map.at(rx.lo, v) {
            if axes.grid {
                painter.line_segment([pos2(rect.left(), pt.y), pos2(rect.right(), pt.y)], grid);
            }
            painter.line_segment(
                [pos2(rect.left(), pt.y), pos2(rect.left() + 4.0, pt.y)],
                Stroke::new(1.0, color(p.border_strong)),
            );
        }
    }
    // Axis labels sit outside the plotting rectangle, so they use an unclipped painter.
    let outside = full.with_clip_rect(rect.expand(60.0));
    for &v in &xt {
        if let Some(pt) = map.at(v, ry.lo) {
            text(
                &outside,
                pos2(pt.x, rect.bottom() + 3.0),
                Align2::CENTER_TOP,
                &tick_text(v),
                eng::caption(),
                color(p.text_secondary),
            );
        }
    }
    for &v in &yt {
        if let Some(pt) = map.at(rx.lo, v) {
            text(
                &outside,
                pos2(rect.left() - 5.0, pt.y),
                Align2::RIGHT_CENTER,
                &tick_text(v),
                eng::caption(),
                color(p.text_secondary),
            );
        }
    }
    for s in &axes.surfaces {
        if s.flat {
            heat_map(painter, look, &map, s);
        }
    }
    let zero = if axes.ylog { None } else { map.at(rx.lo, 0.0) };
    for (i, s) in axes.series.iter().enumerate() {
        series(painter, look, &map, s, i, zero.map(|z| z.y));
    }
}

fn series(
    painter: &Painter,
    look: &Look,
    map: &Map,
    s: &Series,
    index: usize,
    zero_y: Option<f32>,
) {
    let spec = LineSpec::parse(&s.spec);
    let stroke = stroke_for(look, &spec, index);
    let n = s.x.len().min(s.y.len());
    let step = thin(n);
    let pts: Vec<Option<Pos2>> = (0..n)
        .step_by(step)
        .map(|i| map.at(s.x[i], s.y[i]))
        .collect();
    let panel = color(look.palette.control_field_bg);
    // Runs of finite points; a gap (NaN) breaks the line.
    let runs = |pts: &[Option<Pos2>]| -> Vec<Vec<Pos2>> {
        let mut out: Vec<Vec<Pos2>> = vec![Vec::new()];
        for p in pts {
            match p {
                Some(p) => {
                    if let Some(run) = out.last_mut() {
                        run.push(*p);
                    }
                }
                None => {
                    if out.last().is_some_and(|r| !r.is_empty()) {
                        out.push(Vec::new());
                    }
                }
            }
        }
        out
    };
    match s.kind {
        Kind::Line | Kind::Line3 => {
            for run in runs(&pts) {
                polyline(painter, &run, stroke, spec.dash);
            }
            if let Some(m) = spec.marker {
                for p in pts.iter().flatten() {
                    marker(painter, *p, m, stroke.color, panel);
                }
            }
        }
        Kind::Scatter => {
            let m = spec.marker.unwrap_or(Marker::Circle);
            for p in pts.iter().flatten() {
                marker(
                    painter,
                    *p,
                    m,
                    stroke.color,
                    stroke.color.gamma_multiply(0.35),
                );
            }
        }
        Kind::Stairs => {
            let mut steps: Vec<Pos2> = Vec::new();
            for w in pts.iter().flatten().collect::<Vec<_>>().windows(2) {
                steps.push(*w[0]);
                steps.push(pos2(w[1].x, w[0].y));
            }
            if let Some(last) = pts.iter().flatten().last() {
                steps.push(*last);
            }
            polyline(painter, &steps, stroke, Dash::Solid);
        }
        Kind::Stem => {
            let base = zero_y.unwrap_or(map.rect.bottom());
            for p in pts.iter().flatten() {
                painter.line_segment([pos2(p.x, base), *p], stroke);
                marker(
                    painter,
                    *p,
                    spec.marker.unwrap_or(Marker::Circle),
                    stroke.color,
                    panel,
                );
            }
        }
        Kind::Bar | Kind::Hist => {
            let base = zero_y.unwrap_or(map.rect.bottom());
            let xs: Vec<f32> = pts.iter().flatten().map(|p| p.x).collect();
            let gap = xs
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .filter(|d| *d > 0.0)
                .fold(f32::INFINITY, f32::min);
            let full = if gap.is_finite() { gap } else { 24.0 };
            let width = if s.kind == Kind::Hist {
                full
            } else {
                full * 0.8
            };
            for p in pts.iter().flatten() {
                let bar = Rect::from_x_y_ranges(
                    (p.x - width / 2.0)..=(p.x + width / 2.0),
                    p.y.min(base)..=p.y.max(base),
                );
                painter.rect_filled(bar, 1.0, stroke.color.gamma_multiply(0.75));
                painter.rect_stroke(
                    bar,
                    1.0,
                    Stroke::new(1.0, stroke.color),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

fn heat_map(painter: &Painter, look: &Look, map: &Map, s: &abaco_script::plot::Surface) {
    let (nx, ny) = (s.x.len(), s.y.len());
    if nx == 0 || ny == 0 || s.z.len() < nx.saturating_mul(ny) {
        return;
    }
    let finite = s.z.iter().copied().filter(|z| z.is_finite());
    let lo = finite.clone().fold(f64::INFINITY, f64::min);
    let hi = finite.fold(f64::NEG_INFINITY, f64::max);
    let span = if hi > lo { hi - lo } else { 1.0 };
    let edge = |v: &[f64], i: usize, up: bool| -> f64 {
        let at = v.get(i).copied().unwrap_or(0.0);
        let other = if up { i + 1 } else { i.wrapping_sub(1) };
        v.get(other).map_or(at, |o| (at + o) / 2.0)
    };
    for iy in 0..ny {
        for ix in 0..nx {
            let z = s.z[iy * nx + ix];
            if !z.is_finite() {
                continue;
            }
            let (x0, x1) = (edge(&s.x, ix, false), edge(&s.x, ix, true));
            let (y0, y1) = (edge(&s.y, iy, false), edge(&s.y, iy, true));
            let (x0, x1) = if ix == 0 {
                (s.x[0] - (x1 - s.x[0]), x1)
            } else if ix + 1 == nx {
                (x0, s.x[ix] + (s.x[ix] - x0))
            } else {
                (x0, x1)
            };
            let (y0, y1) = if iy == 0 {
                (s.y[0] - (y1 - s.y[0]), y1)
            } else if iy + 1 == ny {
                (y0, s.y[iy] + (s.y[iy] - y0))
            } else {
                (y0, y1)
            };
            if let (Some(a), Some(b)) = (map.at(x0, y0), map.at(x1, y1)) {
                painter.rect_filled(Rect::from_two_pos(a, b), 0.0, ramp(look, (z - lo) / span));
            }
        }
    }
}

fn legend(painter: &Painter, look: &Look, rect: Rect, axes: &Axes) {
    if !axes.legend {
        return;
    }
    let p = &look.palette;
    let entries: Vec<(usize, &Series)> = axes
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.label.is_empty())
        .collect();
    if entries.is_empty() {
        return;
    }
    let galleys: Vec<_> = entries
        .iter()
        .map(|(_, s)| {
            painter.layout_no_wrap(s.label.clone(), eng::caption(), color(p.text_primary))
        })
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 34.0;
    let height = galleys.len() as f32 * 18.0 + 8.0;
    let box_rect = Rect::from_min_size(
        pos2(rect.right() - width - 8.0, rect.top() + 8.0),
        vec2(width, height),
    );
    painter.rect_filled(box_rect, 6.0, color(p.surface_overlay));
    painter.rect_stroke(
        box_rect,
        6.0,
        Stroke::new(1.0, color(p.border_subtle)),
        egui::StrokeKind::Inside,
    );
    for (row, ((i, s), galley)) in entries.iter().zip(galleys).enumerate() {
        let y = box_rect.top() + 13.0 + row as f32 * 18.0;
        let spec = LineSpec::parse(&s.spec);
        let ink = stroke_for(look, &spec, *i).color;
        painter.line_segment(
            [
                pos2(box_rect.left() + 7.0, y),
                pos2(box_rect.left() + 23.0, y),
            ],
            Stroke::new(2.0, ink),
        );
        painter.galley(
            pos2(box_rect.left() + 28.0, y - galley.size().y / 2.0),
            galley,
            color(p.text_primary),
        );
    }
}

// --- Three dimensions -------------------------------------------------------------------------

fn draw_3d(painter: &Painter, look: &Look, rect: Rect, axes: &Axes, camera: Camera) {
    let p = &look.palette;
    let Some(cube) = ranges_3d(axes) else {
        return;
    };
    // Fit the projected cube in the rectangle, keeping its shape.
    let mut extent = 0.0f64;
    for corner in 0..8u8 {
        let (r, u, _) = camera.project(
            f64::from(corner & 1),
            f64::from((corner >> 1) & 1),
            f64::from((corner >> 2) & 1),
        );
        extent = extent.max(r.abs()).max(u.abs());
    }
    let scale = f64::from(rect.width().min(rect.height())) * 0.46 / extent.max(1e-6);
    let to_screen = |x: f64, y: f64, z: f64| -> (Pos2, f64) {
        let (r, u, d) = camera.project(x, y, z);
        (
            pos2(
                rect.center().x + (r * scale) as f32,
                rect.center().y - (u * scale) as f32,
            ),
            d,
        )
    };
    let norm = |v: f64, r: Range| unit(v, r, false);
    let edge = Stroke::new(1.0, color(p.border_default));
    for (a, b) in [
        ((0., 0., 0.), (1., 0., 0.)),
        ((0., 1., 0.), (1., 1., 0.)),
        ((0., 0., 1.), (1., 0., 1.)),
        ((0., 1., 1.), (1., 1., 1.)),
        ((0., 0., 0.), (0., 1., 0.)),
        ((1., 0., 0.), (1., 1., 0.)),
        ((0., 0., 1.), (0., 1., 1.)),
        ((1., 0., 1.), (1., 1., 1.)),
        ((0., 0., 0.), (0., 0., 1.)),
        ((1., 0., 0.), (1., 0., 1.)),
        ((0., 1., 0.), (0., 1., 1.)),
        ((1., 1., 0.), (1., 1., 1.)),
    ] {
        painter.line_segment(
            [to_screen(a.0, a.1, a.2).0, to_screen(b.0, b.1, b.2).0],
            edge,
        );
    }
    let label = |painter: &Painter, at: (f64, f64, f64), s: &str| {
        if !s.is_empty() {
            text(
                painter,
                to_screen(at.0, at.1, at.2).0,
                Align2::CENTER_CENTER,
                s,
                eng::caption(),
                color(p.text_secondary),
            );
        }
    };
    label(painter, (0.5, -0.1, 0.0), &axes.xlabel);
    label(painter, (1.1, 0.5, 0.0), &axes.ylabel);
    label(painter, (-0.1, -0.1, 0.5), &axes.zlabel);
    let zr = cube[2];
    for s in &axes.surfaces {
        if s.flat {
            continue;
        }
        let (nx, ny) = (s.x.len(), s.y.len());
        if nx < 2 || ny < 2 || s.z.len() < nx.saturating_mul(ny) {
            continue;
        }
        let point = |ix: usize, iy: usize| -> Option<(Pos2, f64)> {
            let z = *s.z.get(iy * nx + ix)?;
            if !z.is_finite() {
                return None;
            }
            let (pos, _) = to_screen(norm(s.x[ix], cube[0]), norm(s.y[iy], cube[1]), norm(z, zr));
            Some((pos, norm(z, zr)))
        };
        // Quads from the far side to the near, so nearer ones cover farther ones.
        let mut quads: Vec<Quad> = Vec::new();
        for iy in 0..ny - 1 {
            for ix in 0..nx - 1 {
                let corners = [
                    point(ix, iy),
                    point(ix + 1, iy),
                    point(ix + 1, iy + 1),
                    point(ix, iy + 1),
                ];
                if corners.iter().any(Option::is_none) {
                    continue;
                }
                let c: Vec<(Pos2, f64)> = corners.into_iter().flatten().collect();
                let depth = [(ix, iy), (ix + 1, iy), (ix + 1, iy + 1), (ix, iy + 1)]
                    .iter()
                    .map(|&(a, b)| {
                        let z = s.z.get(b * nx + a).copied().unwrap_or(0.0);
                        to_screen(norm(s.x[a], cube[0]), norm(s.y[b], cube[1]), norm(z, zr)).1
                    })
                    .sum::<f64>()
                    / 4.0;
                let level = c.iter().map(|q| q.1).sum::<f64>() / 4.0;
                quads.push(Quad {
                    depth,
                    pts: [c[0].0, c[1].0, c[2].0, c[3].0],
                    level,
                    last_col: ix + 2 == nx,
                    last_row: iy + 2 == ny,
                });
            }
        }
        quads.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        // The mesh lines are plain segments, drawn after each cell so nearer cells cover them.
        let mesh = Stroke::new(0.5, color(p.control_field_bg).gamma_multiply(0.6));
        for q in quads {
            // A mesh, not a feathered polygon: the anti-aliasing fringe of a nearly edge-on quad
            // grows a long spike at its sharp corners.
            let fill = ramp(look, q.level);
            let mut quad = egui::Mesh::default();
            for pt in q.pts {
                quad.colored_vertex(pt, fill);
            }
            quad.add_triangle(0, 1, 2);
            quad.add_triangle(0, 2, 3);
            painter.add(Shape::mesh(quad));
            let [a, b, c, d] = q.pts;
            painter.line_segment([a, b], mesh);
            painter.line_segment([a, d], mesh);
            if q.last_col {
                painter.line_segment([b, c], mesh);
            }
            if q.last_row {
                painter.line_segment([d, c], mesh);
            }
        }
    }
    for (i, s) in axes.series.iter().enumerate() {
        let spec = LineSpec::parse(&s.spec);
        let stroke = stroke_for(look, &spec, i);
        let n = s.x.len().min(s.y.len()).min(s.z.len());
        let pts: Vec<Pos2> = (0..n)
            .step_by(thin(n))
            .filter(|&k| s.x[k].is_finite() && s.y[k].is_finite() && s.z[k].is_finite())
            .map(|k| {
                to_screen(
                    norm(s.x[k], cube[0]),
                    norm(s.y[k], cube[1]),
                    norm(s.z[k], zr),
                )
                .0
            })
            .collect();
        polyline(
            painter,
            &pts,
            stroke,
            if spec.dash == Dash::None {
                Dash::None
            } else {
                spec.dash
            },
        );
        if let Some(m) = spec.marker {
            for pt in &pts {
                marker(painter, *pt, m, stroke.color, color(p.control_field_bg));
            }
        }
    }
}
