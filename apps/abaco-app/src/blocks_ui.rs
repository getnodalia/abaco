// SPDX-License-Identifier: MIT OR Apache-2.0
//! The block-diagram workspace: the block library, the canvas where blocks are placed and wired,
//! the inspector that edits a block's parameters, and the plots of the last run. Like the rest of
//! the window it only draws and collects: what the person asked for goes into [`Out`] and the
//! app carries it out after the frame.

use std::sync::OnceLock;
use std::time::Duration;

use abaco_app::blocks::engine::Job;
use abaco_app::blocks::session::{Session, Status, GRID};
use abaco_app::commands::Cmd;
use abaco_app::i18n::{tr, translate, trf};
use abaco_blocks::{palette, Method, PaletteEntry, CATEGORIES};
use egui::epaint::CubicBezierShape;
use egui::{pos2, vec2, Align2, Color32, Id, Key, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use nodalia_theme::engineering::{self as eng, layout, Look};
use nodalia_theme::for_egui::color;
use serde_json::{Map, Number, Value};

use crate::app::App;
use crate::figure::{self, FigureView};
use crate::ui::island;
use crate::widgets;

const MIN_CENTER: f32 = 420.0;
const PALETTE_W: f32 = 236.0;
const ROW_H: f32 = 30.0;
/// A block's width and its minimum height, in world units.
const BLOCK_W: f64 = 100.0;
const BLOCK_MIN_H: f64 = 50.0;
/// Height one port takes, in world units.
const PORT_STEP: f64 = 26.0;
const PORT_R: f32 = 4.5;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 2.5;

/// What the person did this frame.
#[derive(Default)]
pub struct Out {
    pub jobs: Vec<Job>,
    pub cmds: Vec<Cmd>,
    pub toasts: Vec<String>,
}

/// A wire being dragged from a port.
#[derive(Clone, Debug, PartialEq)]
struct Wiring {
    block: String,
    port: usize,
    /// The drag started on an output (it ends on an input), else the other way round.
    from_output: bool,
}

/// A block being moved: where it started and where the pointer was.
#[derive(Clone, Debug)]
struct Moving {
    block: String,
    start: (f64, f64),
    pointer: Pos2,
}

/// What the canvas remembers between frames.
pub struct BlocksView {
    /// Screen offset of the world origin from the canvas's top left corner.
    pan: Vec2,
    zoom: f32,
    /// Fit the diagram to the canvas on the next frame (set after opening one).
    pub fit_next: bool,
    /// The world point at the middle of the canvas: where a new block goes.
    center: (f64, f64),
    wiring: Option<Wiring>,
    moving: Option<Moving>,
    /// The text field being typed in: its id and what it holds.
    buffer: Option<(Id, String)>,
    /// A parameter edit the engine refused, and the block it was for.
    error: Option<(String, String)>,
    filter: String,
}

impl Default for BlocksView {
    fn default() -> BlocksView {
        BlocksView {
            pan: Vec2::new(40.0, 40.0),
            zoom: 1.0,
            fit_next: true,
            center: (200.0, 120.0),
            wiring: None,
            moving: None,
            buffer: None,
            error: None,
            filter: String::new(),
        }
    }
}

/// The block library, parsed once.
fn entries() -> &'static [PaletteEntry] {
    static ENTRIES: OnceLock<Vec<PaletteEntry>> = OnceLock::new();
    ENTRIES.get_or_init(palette)
}

/// Lays out the three columns of the block workspace inside `body`.
pub fn body(ui: &mut Ui, look: &Look, app: &mut App, body: Rect, out: &mut Out) {
    let gap = layout::GAP;
    let right_w = (body.width() * 0.34).clamp(320.0, 560.0);
    let mut left = app.settings.show_workspace_panel;
    let mut right = app.settings.show_figures_panel;
    let room = |l: bool, r: bool| {
        body.width() - if l { PALETTE_W + gap } else { 0.0 } - if r { right_w + gap } else { 0.0 }
    };
    if room(left, right) < MIN_CENTER {
        right = false;
    }
    if room(left, right) < MIN_CENTER {
        left = false;
    }
    let mut x0 = body.left();
    let mut x1 = body.right();
    let App {
        blocks,
        blocks_view,
        figure_view,
        blocks_lost,
        ..
    } = app;
    if left {
        let r = Rect::from_min_max(body.min, pos2(body.left() + PALETTE_W, body.bottom()));
        island(ui, look, r, "blocks-library", |ui| {
            library(ui, look, blocks, blocks_view, out)
        });
        x0 = r.right() + gap;
    }
    if right {
        let r = Rect::from_min_max(pos2(body.right() - right_w, body.top()), body.max);
        let top_h = ((r.height() - gap) * 0.55).round();
        let top = Rect::from_min_size(r.min, vec2(r.width(), top_h));
        let bottom = Rect::from_min_max(pos2(r.left(), top.bottom() + gap), r.max);
        island(ui, look, top, "blocks-inspector", |ui| {
            inspector(ui, look, blocks, blocks_view, out)
        });
        island(ui, look, bottom, "blocks-plots", |ui| {
            plots(ui, look, blocks, figure_view, *blocks_lost)
        });
        x1 = r.left() - gap;
    }
    let center = Rect::from_min_max(pos2(x0, body.top()), pos2(x1, body.bottom()));
    island(ui, look, center, "blocks-canvas-island", |ui| {
        canvas(ui, look, blocks, blocks_view, out)
    });
}

// --- Library ------------------------------------------------------------------------------------

fn library(ui: &mut Ui, look: &Look, s: &mut Session, view: &mut BlocksView, out: &mut Out) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Blocks"), |ui| {
        ui.label(
            RichText::new(s.model.blocks.len().to_string())
                .font(eng::value_font())
                .color(color(p.text_tertiary)),
        );
    });
    widgets::field(look).show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut view.filter)
                .font(eng::body())
                .frame(egui::Frame::NONE)
                .hint_text(tr("Search blocks"))
                .align(Align2::LEFT_CENTER)
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(6.0);
    let needle = view.filter.trim().to_lowercase();
    let mut add: Option<&PaletteEntry> = None;
    egui::ScrollArea::vertical()
        .id_salt("blocks-library-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for category in CATEGORIES {
                let shown: Vec<&PaletteEntry> = entries()
                    .iter()
                    .filter(|e| e.category == category)
                    .filter(|e| {
                        needle.is_empty()
                            || e.name.to_lowercase().contains(&needle)
                            || translate(e.name).to_lowercase().contains(&needle)
                    })
                    .collect();
                if shown.is_empty() {
                    continue;
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(translate(category).to_uppercase())
                        .font(eng::label())
                        .color(color(p.text_tertiary)),
                );
                for entry in shown {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, 7.0, color(p.control_bg_hover));
                    }
                    ui.painter().text(
                        rect.left_center() + vec2(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        translate(entry.name),
                        eng::body(),
                        color(p.text_primary),
                    );
                    if response
                        .on_hover_text(tr("Click to add the block to the diagram"))
                        .clicked()
                    {
                        add = Some(entry);
                    }
                }
            }
        });
    if let Some(entry) = add {
        let mut at = view.center;
        // Do not stack a new block exactly on one that is already there.
        while s.model.blocks.iter().any(|b| {
            b.position
                .is_some_and(|q| (q.0 - at.0).abs() < GRID && (q.1 - at.1).abs() < GRID)
        }) {
            at = (at.0 + GRID * 3.0, at.1 + GRID * 3.0);
        }
        if let Err(message) = s.add_block(entry, at) {
            out.toasts.push(message);
        }
    }
}

// --- Canvas -------------------------------------------------------------------------------------

/// A block as drawn: its name and where it is on the screen.
struct Placed {
    name: String,
    kind: String,
    rect: Rect,
    ins: usize,
    outs: usize,
    world: (f64, f64),
}

fn block_size(ins: usize, outs: usize) -> (f64, f64) {
    let ports = ins.max(outs).max(1) as f64;
    (BLOCK_W, (ports * PORT_STEP + 10.0).max(BLOCK_MIN_H))
}

fn port_pos(rect: Rect, count: usize, index: usize, output: bool) -> Pos2 {
    let step = rect.height() / count.max(1) as f32;
    let y = rect.top() + step * (index as f32 + 0.5);
    pos2(if output { rect.right() } else { rect.left() }, y)
}

fn to_screen(origin: Pos2, view_pan: Vec2, zoom: f32, world: (f64, f64)) -> Pos2 {
    origin + view_pan + vec2(world.0 as f32, world.1 as f32) * zoom
}

fn to_world(origin: Pos2, view_pan: Vec2, zoom: f32, screen: Pos2) -> (f64, f64) {
    let v = (screen - origin - view_pan) / zoom;
    (f64::from(v.x), f64::from(v.y))
}

fn curve(a: Pos2, b: Pos2, zoom: f32) -> [Pos2; 4] {
    let dx = ((b.x - a.x) * 0.5).abs().max(40.0 * zoom);
    [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b]
}

fn bezier_at(c: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    pos2(
        c[0].x * w[0] + c[1].x * w[1] + c[2].x * w[2] + c[3].x * w[3],
        c[0].y * w[0] + c[1].y * w[1] + c[2].y * w[2] + c[3].y * w[3],
    )
}

fn distance_to_curve(c: &[Pos2; 4], p: Pos2) -> f32 {
    (0..=24)
        .map(|i| bezier_at(c, i as f32 / 24.0).distance(p))
        .fold(f32::INFINITY, f32::min)
}

fn canvas(ui: &mut Ui, look: &Look, s: &mut Session, view: &mut BlocksView, out: &mut Out) {
    let p = &look.palette;
    let title = s.file_name().unwrap_or_else(|| {
        if s.model.name.trim().is_empty() {
            tr("Untitled diagram").to_owned()
        } else {
            s.model.name.clone()
        }
    });
    let mut fit = false;
    eng::panel_header(ui, look, &title, |ui| {
        if s.dirty {
            ui.label(RichText::new("•").color(color(p.accent_text)));
        }
        if widgets::icon_button(
            ui,
            look,
            "fit",
            tr("Fit the diagram to the window"),
            28.0,
            false,
        )
        .clicked()
        {
            fit = true;
        }
    });
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 10.0, color(p.surface_sunken));
    let origin = rect.min;

    // Where each block is, on screen.
    let placed = |s: &Session, pan: Vec2, zoom: f32| -> Vec<Placed> {
        s.model
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (ins, outs) = b.kind.ports().unwrap_or((0, 0));
                let (w, h) = block_size(ins, outs);
                let world = b
                    .position
                    .unwrap_or((20.0 + (i % 6) as f64 * 150.0, 20.0 + (i / 6) as f64 * 90.0));
                let min = to_screen(origin, pan, zoom, world);
                Placed {
                    name: b.name.clone(),
                    kind: b.kind.type_name(),
                    rect: Rect::from_min_size(min, vec2(w as f32 * zoom, h as f32 * zoom)),
                    ins,
                    outs,
                    world,
                }
            })
            .collect()
    };

    if fit || std::mem::take(&mut view.fit_next) {
        fit_view(view, rect, &placed(s, Vec2::ZERO, 1.0));
    }

    let background = ui.interact(rect, Id::unique("blocks-canvas"), Sense::click_and_drag());
    if background.hovered() {
        let (zoom_delta, scroll, pointer) =
            ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
        if zoom_delta != 1.0 {
            if let Some(at) = pointer {
                let new = (view.zoom * zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);
                // Keep the world point under the pointer where it is.
                let world = (at - origin - view.pan) / view.zoom;
                view.pan = at - origin - world * new;
                view.zoom = new;
            }
        } else if scroll != Vec2::ZERO {
            view.pan += scroll;
        }
    }
    if background.dragged() && view.moving.is_none() && view.wiring.is_none() {
        view.pan += background.drag_delta();
    }
    let (pan, zoom) = (view.pan, view.zoom);
    view.center = to_world(origin, pan, zoom, rect.center());

    draw_grid(&painter, look, rect, pan, zoom);
    let blocks = placed(s, pan, zoom);

    // Lines, under the blocks.
    let rect_of = |name: &str| blocks.iter().find(|b| b.name == name);
    let mut curves: Vec<(usize, [Pos2; 4])> = Vec::new();
    for (i, line) in s.model.lines.iter().enumerate() {
        if let (Some(a), Some(b)) = (rect_of(&line.from.0), rect_of(&line.to.0)) {
            let from = port_pos(a.rect, a.outs, line.from.1, true);
            let to = port_pos(b.rect, b.ins, line.to.1, false);
            curves.push((i, curve(from, to, zoom)));
        }
    }
    for (i, c) in &curves {
        let selected = s.selected_line == Some(*i);
        let stroke = if selected {
            Stroke::new(3.0, color(p.accent_default))
        } else {
            Stroke::new(2.0, color(p.border_strong))
        };
        painter.add(CubicBezierShape::from_points_stroke(
            *c,
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
        let tip = c[3];
        let a = 5.0 * zoom.max(0.6);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, tip + vec2(-a * 1.6, -a), tip + vec2(-a * 1.6, a)],
            stroke.color,
            Stroke::NONE,
        ));
    }

    // Blocks.
    let mut select: Option<String> = None;
    let mut moves: Vec<(String, (f64, f64))> = Vec::new();
    let mut started: Option<Wiring> = None;
    for b in &blocks {
        let selected = s.selected.as_deref() == Some(b.name.as_str());
        let response = ui.interact(
            b.rect,
            Id::unique(("block", &b.name)),
            Sense::click_and_drag(),
        );
        if response.clicked() || response.drag_started() {
            select = Some(b.name.clone());
        }
        if response.drag_started() {
            if let Some(at) = response.interact_pointer_pos() {
                view.moving = Some(Moving {
                    block: b.name.clone(),
                    start: b.world,
                    pointer: at,
                });
            }
        }
        if response.dragged() {
            if let (Some(m), Some(at)) = (&view.moving, response.interact_pointer_pos()) {
                if m.block == b.name {
                    let d = (at - m.pointer) / zoom;
                    moves.push((
                        b.name.clone(),
                        (m.start.0 + f64::from(d.x), m.start.1 + f64::from(d.y)),
                    ));
                }
            }
        }
        if response.drag_stopped() {
            view.moving = None;
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let radius = (8.0 * zoom).max(4.0);
        painter.rect_filled(b.rect, radius, color(p.surface_raised));
        painter.rect_stroke(
            b.rect,
            radius,
            if selected {
                Stroke::new(2.0, color(p.accent_default))
            } else {
                Stroke::new(1.0, color(p.border_default))
            },
            egui::StrokeKind::Inside,
        );
        let clip = painter.with_clip_rect(b.rect.shrink2(vec2(6.0 * zoom, 0.0)));
        let big = (13.0 * zoom).clamp(8.0, 26.0);
        let small = (10.0 * zoom).clamp(7.0, 20.0);
        if zoom >= 0.3 {
            clip.text(
                b.rect.center() - vec2(0.0, small * 0.7),
                Align2::CENTER_CENTER,
                &b.name,
                egui::FontId::proportional(big),
                color(p.text_primary),
            );
            clip.text(
                b.rect.center() + vec2(0.0, big * 0.7),
                Align2::CENTER_CENTER,
                translate(&b.kind),
                egui::FontId::proportional(small),
                color(p.text_tertiary),
            );
        }
        for (count, output) in [(b.ins, false), (b.outs, true)] {
            for k in 0..count {
                let at = port_pos(b.rect, count, k, output);
                let hit = Rect::from_center_size(at, vec2(18.0, 18.0));
                let port =
                    ui.interact(hit, Id::unique(("port", &b.name, output, k)), Sense::drag());
                let over = port.hovered()
                    || view.wiring.as_ref().is_some_and(|w| {
                        w.block == b.name && w.port == k && w.from_output == output
                    });
                if port.drag_started() {
                    started = Some(Wiring {
                        block: b.name.clone(),
                        port: k,
                        from_output: output,
                    });
                }
                if over {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                let r = PORT_R * zoom.clamp(0.7, 1.6);
                painter.circle_filled(
                    at,
                    if over { r + 1.5 } else { r },
                    color(if over {
                        p.accent_default
                    } else {
                        p.border_strong
                    }),
                );
            }
        }
    }
    if let Some(name) = select {
        s.selected = Some(name);
        s.selected_line = None;
    }
    for (name, at) in moves {
        s.move_block(&name, at);
    }
    if started.is_some() {
        view.wiring = started;
    }

    // A wire being dragged, and where it lands.
    if let Some(w) = view.wiring.clone() {
        let pointer = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
        if let (Some(pointer), Some(from)) = (pointer, blocks.iter().find(|b| b.name == w.block)) {
            let (count, output) = if w.from_output {
                (from.outs, true)
            } else {
                (from.ins, false)
            };
            let anchor = port_pos(from.rect, count, w.port, output);
            let c = if w.from_output {
                curve(anchor, pointer, zoom)
            } else {
                curve(pointer, anchor, zoom)
            };
            painter.add(CubicBezierShape::from_points_stroke(
                c,
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.0, color(p.accent_default)),
            ));
            if ui.input(|i| i.pointer.any_released()) {
                // The nearest port of the other kind, within reach.
                let target = blocks
                    .iter()
                    .flat_map(|b| {
                        let (count, output) = if w.from_output {
                            (b.ins, false)
                        } else {
                            (b.outs, true)
                        };
                        (0..count)
                            .map(move |k| (b.name.as_str(), k, port_pos(b.rect, count, k, output)))
                    })
                    .map(|(n, k, at)| (n, k, at.distance(pointer)))
                    .filter(|(_, _, d)| *d <= 16.0)
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                if let Some((name, k, _)) = target {
                    let result = if w.from_output {
                        s.connect((&w.block, w.port), (name, k))
                    } else {
                        s.connect((name, k), (&w.block, w.port))
                    };
                    if let Err(message) = result {
                        out.toasts.push(message);
                    }
                }
                view.wiring = None;
            }
        } else {
            view.wiring = None;
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        view.moving = None;
    }

    // A click on empty canvas selects the line under it, or nothing.
    if background.clicked() {
        if let Some(at) = background.interact_pointer_pos() {
            let hit = curves
                .iter()
                .map(|(i, c)| (*i, distance_to_curve(c, at)))
                .filter(|(_, d)| *d <= 7.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            s.selected = None;
            s.selected_line = hit.map(|(i, _)| i);
        }
    }
    if s.model.blocks.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("Add blocks from the library on the left, then drag from an output to an input."),
            eng::body(),
            color(p.text_tertiary),
        );
    }
    // Delete removes the selection, unless a text field is being typed in.
    let typing = ui.ctx().egui_wants_keyboard_input();
    if !typing
        && ui.rect_contains_pointer(rect)
        && ui.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
    {
        out.cmds.push(Cmd::DeleteBlock);
    }
}

fn fit_view(view: &mut BlocksView, rect: Rect, blocks: &[Placed]) {
    let Some(first) = blocks.first() else {
        view.pan = vec2(40.0, 40.0);
        view.zoom = 1.0;
        return;
    };
    let mut bounds = first.rect;
    for b in blocks {
        bounds = bounds.union(b.rect);
    }
    // `blocks` were placed with a zero pan and unit zoom, so these are world units.
    let pad = 40.0;
    let zoom = ((rect.width() - 2.0 * pad) / bounds.width().max(1.0))
        .min((rect.height() - 2.0 * pad) / bounds.height().max(1.0))
        .clamp(MIN_ZOOM, 1.25);
    let center = bounds.center() - rect.min;
    view.zoom = zoom;
    view.pan = rect.size() * 0.5 - center * zoom;
}

fn draw_grid(painter: &egui::Painter, look: &Look, rect: Rect, pan: Vec2, zoom: f32) {
    let step = GRID as f32 * 2.0 * zoom;
    if step < 9.0 {
        return;
    }
    let ink = color(look.palette.border_subtle);
    let off = vec2(pan.x.rem_euclid(step), pan.y.rem_euclid(step));
    let mut x = rect.left() + off.x;
    while x < rect.right() {
        let mut y = rect.top() + off.y;
        while y < rect.bottom() {
            painter.circle_filled(pos2(x, y), 1.0, ink);
            y += step;
        }
        x += step;
    }
}

// --- Inspector ----------------------------------------------------------------------------------

fn inspector(ui: &mut Ui, look: &Look, s: &mut Session, view: &mut BlocksView, out: &mut Out) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Inspector"), |_| {});
    if view
        .error
        .as_ref()
        .is_some_and(|(b, _)| Some(b) != s.selected.as_ref())
    {
        view.error = None;
    }
    egui::ScrollArea::vertical()
        .id_salt("blocks-inspector-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(name) = s.selected.clone() {
                block_inspector(ui, look, s, view, &name, out);
            } else if let Some(i) = s.selected_line {
                match s.model.lines.get(i) {
                    Some(l) => {
                        ui.label(
                            RichText::new(trf(
                                "{} output {} → {} input {}",
                                &[
                                    &l.from.0,
                                    &(l.from.1 + 1).to_string(),
                                    &l.to.0,
                                    &(l.to.1 + 1).to_string(),
                                ],
                            ))
                            .font(eng::body())
                            .color(color(p.text_primary)),
                        );
                        ui.add_space(6.0);
                        if widgets::secondary_button(ui, look, tr("Delete the line")).clicked() {
                            out.cmds.push(Cmd::DeleteBlock);
                        }
                    }
                    None => s.selected_line = None,
                }
            } else {
                diagram_inspector(ui, look, s, view);
            }
        });
}

/// A label and, beside it, what edits the value.
fn row(ui: &mut Ui, look: &Look, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(140.0, 28.0), Sense::hover());
        ui.painter().with_clip_rect(rect).text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            eng::body(),
            color(look.palette.text_secondary),
        );
        add(ui);
    });
}

/// A single-line field that holds what is typed until it is committed with Enter or by leaving
/// it; `Some` is the committed text.
fn text_field(
    ui: &mut Ui,
    look: &Look,
    id: Id,
    current: &str,
    view: &mut BlocksView,
) -> Option<String> {
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text = match &view.buffer {
        Some((b, t)) if *b == id => t.clone(),
        _ => current.to_owned(),
    };
    let mut committed = None;
    widgets::field(look).show(ui, |ui| {
        let shown = egui::TextEdit::singleline(&mut text)
            .id(id)
            .font(eng::value_font())
            .frame(egui::Frame::NONE)
            .align(Align2::LEFT_CENTER)
            .desired_width(f32::INFINITY)
            .show(ui);
        if shown.response.changed() || focused {
            view.buffer = Some((id, text.clone()));
        }
        if shown.response.lost_focus() {
            view.buffer = None;
            if text != current {
                committed = Some(text.clone());
            }
        }
    });
    committed
}

fn number_text(v: f64) -> String {
    format!("{v}")
}

fn block_inspector(
    ui: &mut Ui,
    look: &Look,
    s: &mut Session,
    view: &mut BlocksView,
    name: &str,
    out: &mut Out,
) {
    let p = &look.palette;
    let Some(block) = s.model.block(name) else {
        s.selected = None;
        return;
    };
    let kind = block.kind.type_name();
    let (ins, outs) = block.kind.ports().unwrap_or((0, 0));
    ui.label(
        RichText::new(translate(&kind))
            .font(eng::body())
            .color(color(p.text_tertiary)),
    );
    ui.add_space(4.0);
    let mut rename: Option<String> = None;
    row(ui, look, tr("Name"), |ui| {
        rename = text_field(ui, look, Id::unique(("block-name", name)), name, view);
    });
    if let Some(new) = rename {
        if let Err(message) = s.rename_block(name, &new) {
            view.error = Some((name.to_owned(), message));
        } else {
            view.error = None;
        }
        return;
    }
    row(ui, look, tr("Ports"), |ui| {
        ui.label(
            RichText::new(trf(
                "Inputs: {} · outputs: {}",
                &[&ins.to_string(), &outs.to_string()],
            ))
            .font(eng::body())
            .color(color(p.text_primary)),
        );
    });
    ui.add_space(4.0);
    let Some(params) = s.params(name) else {
        return;
    };
    let mut edited: Option<Map<String, Value>> = None;
    for (key, value) in &params {
        let label = abaco_app::blocks::field_label(key);
        if key == "model" {
            let n = value
                .get("blocks")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            row(ui, look, tr("Contents"), |ui| {
                ui.label(
                    RichText::new(trf("{} blocks", &[&n.to_string()]))
                        .font(eng::body())
                        .color(color(p.text_primary)),
                );
            });
            continue;
        }
        let id = Id::unique(("param", name, key));
        let mut new_value: Option<Result<Value, String>> = None;
        row(ui, look, translate(&label), |ui| match value {
            Value::Bool(b) => {
                let mut v = *b;
                if ui.checkbox(&mut v, "").changed() {
                    new_value = Some(Ok(Value::Bool(v)));
                }
            }
            Value::Number(n) => {
                let current = n.as_f64().map_or_else(|| n.to_string(), number_text);
                if let Some(text) = text_field(ui, look, id, &current, view) {
                    new_value = Some(parse_number(&text, n.is_i64() || n.is_u64()));
                }
            }
            Value::String(t) => {
                if let Some(text) = text_field(ui, look, id, t, view) {
                    new_value = Some(Ok(Value::String(text)));
                }
            }
            other => {
                let current = other.to_string();
                if let Some(text) = text_field(ui, look, id, &current, view) {
                    new_value = Some(serde_json::from_str(&text).map_err(|e| e.to_string()));
                }
            }
        });
        match new_value {
            Some(Ok(v)) => {
                let mut map = edited.take().unwrap_or_else(|| params.clone());
                map.insert(key.clone(), v);
                edited = Some(map);
            }
            Some(Err(message)) => view.error = Some((name.to_owned(), message)),
            None => {}
        }
    }
    if let Some(map) = edited {
        match s.set_params(name, map) {
            Ok(removed) => {
                view.error = None;
                if removed > 0 {
                    out.toasts.push(trf(
                        "{} lines to ports that no longer exist were removed.",
                        &[&removed.to_string()],
                    ));
                }
            }
            Err(message) => view.error = Some((name.to_owned(), message)),
        }
    }
    if let Some((_, message)) = &view.error {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                RichText::new(message)
                    .font(eng::body())
                    .color(color(p.status_danger_fg)),
            )
            .wrap(),
        );
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, look, tr("Duplicate")).clicked() {
            s.duplicate_block(name);
        }
        if widgets::secondary_button(ui, look, tr("Delete")).clicked() {
            out.cmds.push(Cmd::DeleteBlock);
        }
    });
}

fn parse_number(text: &str, integer: bool) -> Result<Value, String> {
    let t = text.trim();
    if integer {
        if let Ok(n) = t.parse::<i64>() {
            return Ok(Value::Number(n.into()));
        }
    }
    let v: f64 = t
        .parse()
        .map_err(|_| trf("The text {} is not a number.", &[t]))?;
    Number::from_f64(v)
        .map(Value::Number)
        .ok_or_else(|| tr("The number must be finite.").to_owned())
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Euler => tr("Euler (fixed step)"),
        Method::Heun => tr("Heun (fixed step)"),
        Method::Bs3 => tr("Bogacki–Shampine (fixed step)"),
        Method::Rk4 => tr("Runge–Kutta 4 (fixed step)"),
        Method::Dopri5 => tr("Dormand–Prince 5(4), like ode45"),
        Method::Bs23 => tr("Bogacki–Shampine 3(2), like ode23"),
        Method::Rosenbrock23 => tr("Rosenbrock 2(3), stiff, like ode23s"),
    }
}

const METHODS: [Method; 7] = [
    Method::Euler,
    Method::Heun,
    Method::Bs3,
    Method::Rk4,
    Method::Dopri5,
    Method::Bs23,
    Method::Rosenbrock23,
];

/// A row editing a finite number; `true` when it changed.
fn number_row(
    ui: &mut Ui,
    look: &Look,
    view: &mut BlocksView,
    label: &str,
    key: &str,
    value: &mut f64,
    minimum: Option<f64>,
) -> bool {
    let mut changed = false;
    row(ui, look, label, |ui| {
        let id = Id::unique(("solver", key));
        if let Some(text) = text_field(ui, look, id, &number_text(*value), view) {
            if let Ok(v) = text.trim().parse::<f64>() {
                if v.is_finite() && minimum.is_none_or(|m| v >= m) {
                    *value = v;
                    changed = true;
                }
            }
        }
    });
    changed
}

/// The solver settings, shown when nothing is selected.
fn diagram_inspector(ui: &mut Ui, look: &Look, s: &mut Session, view: &mut BlocksView) {
    let p = &look.palette;
    ui.label(
        RichText::new(tr("Simulation"))
            .font(eng::label())
            .color(color(p.text_tertiary)),
    );
    ui.add_space(4.0);
    let mut changed = false;
    row(ui, look, tr("Method"), |ui| {
        egui::ComboBox::from_id_salt("blocks-method")
            .selected_text(method_name(s.model.solver.method))
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                for m in METHODS {
                    if ui
                        .selectable_label(s.model.solver.method == m, method_name(m))
                        .clicked()
                    {
                        s.model.solver.method = m;
                        changed = true;
                    }
                }
            });
    });
    let solver = &mut s.model.solver;
    changed |= number_row(
        ui,
        look,
        view,
        tr("Start time"),
        "start",
        &mut solver.start_time,
        None,
    );
    changed |= number_row(
        ui,
        look,
        view,
        tr("Stop time"),
        "stop",
        &mut solver.stop_time,
        None,
    );
    let mut step = solver.step.unwrap_or(0.0);
    let mut max_step = solver.max_step.unwrap_or(0.0);
    let (mut step_set, mut max_set) = (false, false);
    row(ui, look, tr("Step (0 = automatic)"), |ui| {
        let id = Id::unique(("solver", "step"));
        if let Some(text) = text_field(ui, look, id, &number_text(step), view) {
            if let Ok(v) = text.trim().parse::<f64>() {
                if v.is_finite() && v >= 0.0 {
                    step = v;
                    step_set = true;
                }
            }
        }
    });
    row(ui, look, tr("Max step (0 = none)"), |ui| {
        let id = Id::unique(("solver", "max-step"));
        if let Some(text) = text_field(ui, look, id, &number_text(max_step), view) {
            if let Ok(v) = text.trim().parse::<f64>() {
                if v.is_finite() && v >= 0.0 {
                    max_step = v;
                    max_set = true;
                }
            }
        }
    });
    if step_set {
        solver.step = (step > 0.0).then_some(step);
        changed = true;
    }
    if max_set {
        solver.max_step = (max_step > 0.0).then_some(max_step);
        changed = true;
    }
    changed |= number_row(
        ui,
        look,
        view,
        tr("Relative tolerance"),
        "rtol",
        &mut solver.rtol,
        Some(0.0),
    );
    changed |= number_row(
        ui,
        look,
        view,
        tr("Absolute tolerance"),
        "atol",
        &mut solver.atol,
        Some(0.0),
    );
    if changed {
        s.dirty = true;
    }
    ui.add_space(10.0);
    ui.label(
        RichText::new(tr("Select a block to edit its parameters."))
            .font(eng::body())
            .color(color(p.text_tertiary)),
    );
}

// --- Plots --------------------------------------------------------------------------------------

fn plots(ui: &mut Ui, look: &Look, s: &Session, view: &mut FigureView, lost: bool) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Plots"), |ui| {
        if s.running {
            ui.label(
                RichText::new(tr("Running…"))
                    .font(eng::body())
                    .color(color(p.accent_text)),
            );
        }
    });
    if lost {
        ui.label(
            RichText::new(tr("The simulator stopped"))
                .font(eng::body())
                .color(color(p.status_danger_fg)),
        );
        return;
    }
    if let Status::Failed(message) = &s.status {
        ui.add(
            egui::Label::new(
                RichText::new(message)
                    .font(eng::body())
                    .color(color(p.status_danger_fg)),
            )
            .wrap(),
        );
        ui.add_space(6.0);
    }
    if !s.has_result {
        ui.label(
            RichText::new(tr(
                "Add a Scope block and run the simulation to see the signals.",
            ))
            .font(eng::body())
            .color(color(p.text_tertiary)),
        );
        return;
    }
    figure::show(ui, look, &s.figure, view, Id::unique("blocks-figure"));
}

// --- Status bar ---------------------------------------------------------------------------------

pub fn status(ui: &mut Ui, look: &Look, app: &App) {
    let p = &look.palette;
    let s = &app.blocks;
    eng::status_bar(
        ui,
        |ui| {
            eng::status_value(
                ui,
                look,
                &trf("Blocks: {}", &[&s.model.blocks.len().to_string()]),
            );
            ui.label(RichText::new("·").color(color(p.text_tertiary)));
            if app.blocks_lost {
                ui.label(
                    RichText::new(tr("The simulator stopped")).color(color(p.status_danger_fg)),
                );
            } else if s.running {
                ui.label(RichText::new(tr("Running…")).color(color(p.accent_text)));
            } else {
                match &s.status {
                    Status::Idle => {
                        ui.label(RichText::new(tr("Ready")).color(color(p.text_secondary)));
                    }
                    Status::Done {
                        millis,
                        steps,
                        stopped_by,
                        ..
                    } => {
                        let time = crate::panels::duration_text(Duration::from_millis(*millis));
                        let text = match stopped_by {
                            Some(by) => trf(
                                "Stopped by {} · {} · {} steps",
                                &[by, &time, &steps.to_string()],
                            ),
                            None => trf("Simulated in {} · {} steps", &[&time, &steps.to_string()]),
                        };
                        ui.label(RichText::new(text).color(color(p.text_secondary)));
                    }
                    Status::Stopped => {
                        ui.label(
                            RichText::new(tr("Simulation stopped")).color(color(p.text_secondary)),
                        );
                    }
                    Status::Failed(_) => {
                        ui.label(
                            RichText::new(tr("The simulation failed"))
                                .color(color(p.status_danger_fg)),
                        );
                    }
                }
            }
        },
        |ui| {
            eng::status_value(
                ui,
                look,
                &trf("Lines: {}", &[&s.model.lines.len().to_string()]),
            );
        },
    );
}
