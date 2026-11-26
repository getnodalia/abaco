// SPDX-License-Identifier: MIT OR Apache-2.0
//! The document: a column of cells. A maths cell has its input on the left and its result, with
//! the unit, beside it; an error is shown under the input, with the line and column and the
//! offending line quoted. A text cell is prose, drawn as such until it is selected.

use std::ops::Range;

use abaco_app::commands::Cmd;
use abaco_app::engine::{ErrorView, Grid, Outcome, Shown};
use abaco_app::highlight::{tokens, Token};
use abaco_app::i18n::{tr, trf};
use abaco_app::workbook::{CellState, Kind, Msg, Workbook};
use egui::text::{LayoutJob, TextFormat};
use egui::{
    pos2, vec2, Align, CornerRadius, Frame, Id, Layout, Margin, RichText, Sense, Stroke, Ui,
};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::color;

use crate::widgets::{icon, icon_button, secondary_button};

/// Below this width the result goes under the input instead of beside it.
const WIDE: f32 = 620.0;

/// Draws every cell and the buttons that add one.
pub fn show(ui: &mut Ui, wb: &mut Workbook, look: &Look, msgs: &mut Vec<Msg>) {
    let selected = wb.selected;
    let focus = wb.focus;
    let reveal = wb.reveal;
    let mut focused = false;
    let mut revealed = false;
    let width = ui.available_width();
    for cell in &mut wb.cells {
        let id = cell.id;
        let is_selected = selected == Some(id);
        let edit_id = Id::unique(("abaco-cell", id.0));
        if focus == Some(id) {
            ui.memory_mut(|m| m.request_focus(edit_id));
            focused = true;
        }
        let card = card(ui, look, cell, edit_id, is_selected, width, msgs);
        if is_selected {
            toolbar(ui, look, card.rect, id, msgs);
        }
        if reveal == Some(id) {
            ui.scroll_to_rect(card.rect, None);
            revealed = true;
        }
        ui.add_space(8.0);
    }
    if focused {
        wb.focus = None;
        if let Some(id) = focus {
            wb.selected = Some(id);
        }
    }
    if revealed {
        wb.reveal = None;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        for cmd in [Cmd::AddMath, Cmd::AddText] {
            if secondary_button(ui, look, cmd.translated()).clicked() {
                msgs.push(match cmd {
                    Cmd::AddText => Msg::AddText { after: None },
                    _ => Msg::AddMath { after: None },
                });
            }
        }
    });
    ui.add_space(24.0);
}

fn toolbar(ui: &mut Ui, look: &Look, card: egui::Rect, id: abaco_doc::CellId, msgs: &mut Vec<Msg>) {
    let size = 26.0;
    let buttons = [
        (Cmd::MoveUp, Msg::MoveUp(id)),
        (Cmd::MoveDown, Msg::MoveDown(id)),
        (Cmd::Duplicate, Msg::Duplicate(id)),
        (Cmd::DeleteCell, Msg::Remove(id)),
    ];
    let width = size * buttons.len() as f32;
    let rect = egui::Rect::from_min_size(
        pos2(card.right() - width - 8.0, card.top() + 6.0),
        vec2(width, size),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing = vec2(0.0, 0.0);
    for (cmd, msg) in buttons {
        let name = cmd.icon().unwrap_or("more");
        if icon_button(&mut child, look, name, cmd.translated(), size, false).clicked() {
            msgs.push(msg);
        }
    }
}

struct Card {
    rect: egui::Rect,
}

fn card(
    ui: &mut Ui,
    look: &Look,
    cell: &mut CellState,
    edit_id: Id,
    selected: bool,
    width: f32,
    msgs: &mut Vec<Msg>,
) -> Card {
    let p = &look.palette;
    let edge = if selected {
        color(p.accent_default)
    } else {
        color(p.border_subtle)
    };
    let frame = Frame::NONE
        .fill(color(p.surface_raised))
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, edge))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8));
    let out = frame.show(ui, |ui| {
        ui.set_width(width - 22.0);
        match cell.kind {
            Kind::Math => math_cell(ui, look, cell, edit_id, selected, msgs),
            Kind::Text => text_cell(ui, look, cell, edit_id, selected, msgs),
        }
    });
    Card {
        rect: out.response.rect,
    }
}

fn field_frame(look: &Look, selected: bool) -> Frame {
    let p = &look.palette;
    Frame::NONE
        .fill(color(p.control_field_bg))
        .stroke(Stroke::new(
            1.0,
            if selected {
                color(p.border_focus)
            } else {
                color(p.border_default)
            },
        ))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(8, 6))
}

fn math_cell(
    ui: &mut Ui,
    look: &Look,
    cell: &mut CellState,
    edit_id: Id,
    selected: bool,
    msgs: &mut Vec<Msg>,
) {
    let wide = ui.available_width() >= WIDE;
    let left = if wide {
        (ui.available_width() * 0.54).clamp(260.0, 700.0)
    } else {
        ui.available_width()
    };
    let span = match &cell.outcome {
        Outcome::Err { error, .. } => error.span,
        _ => None,
    };
    let id = cell.id;
    let mut changed = false;
    let mut focused = false;
    let mut input = |ui: &mut Ui, cell: &mut CellState| {
        field_frame(look, selected).show(ui, |ui| {
            ui.set_width(left - 18.0);
            let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                let job = highlight_job(look, text.as_str(), span, wrap);
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let response = ui.add(
                egui::TextEdit::multiline(&mut cell.buffer)
                    .id(edit_id)
                    .font(eng::value_font())
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Type a formula, for example 9.81 m/s^2 * 80 kg"))
                    .layouter(&mut layouter),
            );
            changed |= response.changed();
            focused |= response.has_focus();
        });
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            input(ui, cell);
            ui.vertical(|ui| {
                // Keep clear of the toolbar a selected cell shows in its corner.
                result(ui, look, &cell.outcome);
            });
        });
    } else {
        input(ui, cell);
        ui.add_space(4.0);
        result(ui, look, &cell.outcome);
    }
    if let Outcome::Err { error, .. } = &cell.outcome {
        ui.add_space(6.0);
        error_box(ui, look, error);
    }
    if let Outcome::Ok { warnings, .. } | Outcome::Err { warnings, .. } = &cell.outcome {
        for warning in warnings {
            ui.add_space(4.0);
            notice_line(ui, look, "info", warning, false);
        }
    }
    if (focused || changed) && !selected {
        msgs.push(Msg::Select(id));
    }
    if changed {
        msgs.push(Msg::Edited(id));
    }
}

/// One line with an icon: a warning of a cell that still has a value.
fn notice_line(ui: &mut Ui, look: &Look, name: &str, text: &str, danger: bool) {
    let p = &look.palette;
    let ink = if danger {
        color(p.status_danger_fg)
    } else {
        color(p.status_warning_fg)
    };
    ui.horizontal(|ui| {
        icon(ui, name, 14.0, ink);
        ui.label(RichText::new(text).font(eng::body()).color(ink));
    });
}

fn error_box(ui: &mut Ui, look: &Look, error: &ErrorView) {
    let p = &look.palette;
    let ink = color(p.status_danger_fg);
    Frame::NONE
        .fill(color(p.status_danger_bg))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                icon(ui, "warning", 14.0, ink);
                if let Some((line, column)) = error.position {
                    ui.label(
                        RichText::new(trf(
                            "Line {}, column {}",
                            &[&line.to_string(), &column.to_string()],
                        ))
                        .font(eng::label())
                        .color(ink),
                    );
                }
                ui.label(RichText::new(&error.message).font(eng::body()).color(ink));
            });
            if let Some(quote) = &error.quote {
                let mono = eng::value_font();
                ui.label(RichText::new(&quote.line).font(mono.clone()).color(ink));
                let pad = " ".repeat(quote.caret);
                let marks = "^".repeat(quote.width.max(1));
                ui.label(RichText::new(format!("{pad}{marks}")).font(mono).color(ink));
            }
        });
}

/// The value of a cell, beside its input.
fn result(ui: &mut Ui, look: &Look, outcome: &Outcome) {
    let p = &look.palette;
    match outcome {
        Outcome::Pending => {
            ui.label(
                RichText::new("…")
                    .font(eng::value_font())
                    .color(color(p.text_tertiary)),
            );
        }
        Outcome::Ok {
            value: Some(shown), ..
        } => value(ui, look, shown),
        Outcome::Ok {
            value: None,
            defined,
            ..
        } => {
            if !defined.is_empty() {
                ui.label(
                    RichText::new(trf("Defines {}", &[&defined.join(", ")]))
                        .font(eng::body())
                        .color(color(p.text_tertiary)),
                );
            }
        }
        Outcome::Err { .. } => {
            ui.label(
                RichText::new(tr("No value"))
                    .font(eng::body())
                    .color(color(p.text_tertiary)),
            );
        }
    }
}

fn value(ui: &mut Ui, look: &Look, shown: &Shown) {
    let p = &look.palette;
    let big = egui::FontId::new(15.0, egui::FontFamily::Monospace);
    if let Some(grid) = &shown.grid {
        matrix(ui, look, grid);
    } else if let Some((number, unit)) = &shown.parts {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new("=")
                    .font(big.clone())
                    .color(color(p.text_tertiary)),
            );
            ui.label(
                RichText::new(number)
                    .font(big.clone())
                    .color(color(p.text_primary)),
            );
            ui.label(RichText::new(unit).font(big).color(color(p.accent_text)));
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                RichText::new("=")
                    .font(big.clone())
                    .color(color(p.text_tertiary)),
            );
            ui.label(
                RichText::new(&shown.text)
                    .font(big)
                    .color(color(p.text_primary)),
            );
        });
    }
    if let Some(dimension) = &shown.dimension {
        ui.label(
            RichText::new(abaco_app::i18n::translate(dimension))
                .font(eng::caption())
                .color(color(p.text_tertiary)),
        );
    }
}

fn matrix(ui: &mut Ui, look: &Look, grid: &Grid) {
    let p = &look.palette;
    Frame::NONE
        .fill(color(p.surface_sunken))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            egui::Grid::new(ui.next_auto_id())
                .spacing(vec2(16.0, 2.0))
                .show(ui, |ui| {
                    for row in grid.cells.chunks(grid.shown_cols.max(1)) {
                        for text in row {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(text)
                                        .font(eng::value_font())
                                        .color(color(p.text_primary)),
                                );
                            });
                        }
                        ui.end_row();
                    }
                });
        });
    let dims = format!("{} × {}", grid.rows, grid.cols);
    let text = if grid.shown_rows < grid.rows || grid.shown_cols < grid.cols {
        trf(
            "{}, showing the first {} × {}",
            &[
                &dims,
                &grid.shown_rows.to_string(),
                &grid.shown_cols.to_string(),
            ],
        )
    } else {
        dims
    };
    ui.label(
        RichText::new(text)
            .font(eng::caption())
            .color(color(p.text_tertiary)),
    );
}

fn text_cell(
    ui: &mut Ui,
    look: &Look,
    cell: &mut CellState,
    edit_id: Id,
    selected: bool,
    msgs: &mut Vec<Msg>,
) {
    let p = &look.palette;
    let id = cell.id;
    if selected {
        let mut changed = false;
        let mut focused = false;
        field_frame(look, true).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let response = ui.add(
                egui::TextEdit::multiline(&mut cell.buffer)
                    .id(edit_id)
                    .font(eng::body())
                    .frame(egui::Frame::NONE)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr("Write a note. Start a line with # for a heading.")),
            );
            changed = response.changed();
            focused = response.has_focus();
        });
        let _ = focused;
        if changed {
            msgs.push(Msg::Edited(id));
        }
        return;
    }
    let response = if cell.is_blank() {
        ui.add(
            egui::Label::new(
                RichText::new(tr("Empty text cell"))
                    .font(eng::body())
                    .italics()
                    .color(color(p.text_tertiary)),
            )
            .sense(Sense::click()),
        )
    } else {
        let job = markdown_job(look, &cell.buffer, ui.available_width());
        ui.add(egui::Label::new(job).sense(Sense::click()))
    };
    if response.clicked() {
        msgs.push(Msg::Select(id));
        // The editor takes the keyboard on the next frame.
        ui.memory_mut(|m| m.request_focus(edit_id));
    }
}

// --- Text ---------------------------------------------------------------------------------------

/// The input's colours by token, from the palette.
fn ink(look: &Look, token: Token) -> egui::Color32 {
    let p = &look.palette;
    color(match token {
        Token::Comment => p.text_tertiary,
        Token::Number | Token::Name | Token::Space => p.text_primary,
        Token::Unit => p.accent_text,
        Token::Function => p.status_info_fg,
        Token::Keyword => p.status_warning_fg,
        Token::String => p.status_success_fg,
        Token::Punct => p.text_secondary,
    })
}

/// `text` as a layout job: tokens coloured, and the part the error points at underlined.
pub fn highlight_job(
    look: &Look,
    text: &str,
    span: Option<(usize, usize)>,
    wrap: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let font = eng::value_font();
    let cuts: Vec<usize> = span
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .filter(|c| *c <= text.len() && text.is_char_boundary(*c))
        .collect();
    let (from, to) = span.unwrap_or((0, 0));
    let danger = Stroke::new(1.5, color(look.palette.status_danger_fg));
    for (range, token) in tokens(text) {
        let mut pieces: Vec<Range<usize>> = Vec::new();
        let mut start = range.start;
        let mut inner: Vec<usize> = cuts
            .iter()
            .copied()
            .filter(|c| *c > range.start && *c < range.end)
            .collect();
        inner.sort_unstable();
        for cut in inner {
            pieces.push(start..cut);
            start = cut;
        }
        pieces.push(start..range.end);
        for piece in pieces {
            let Some(slice) = text.get(piece.clone()) else {
                continue;
            };
            let bad = span.is_some() && piece.start >= from && piece.end <= to.max(from + 1);
            job.append(
                slice,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: ink(look, token),
                    underline: if bad { danger } else { Stroke::NONE },
                    ..Default::default()
                },
            );
        }
    }
    job
}

/// Prose with `#` headings and `code` spans.
pub fn markdown_job(look: &Look, text: &str, wrap: f32) -> LayoutJob {
    let p = &look.palette;
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let lines: Vec<&str> = text.lines().collect();
    for (n, line) in lines.iter().enumerate() {
        let (font, ink_color, body) = if let Some(rest) = line.strip_prefix("## ") {
            (
                egui::FontId::new(16.0, egui::FontFamily::Name(eng::SEMIBOLD.into())),
                p.text_primary,
                rest,
            )
        } else if let Some(rest) = line.strip_prefix("# ") {
            (
                egui::FontId::new(21.0, egui::FontFamily::Name(eng::BOLD.into())),
                p.text_primary,
                rest,
            )
        } else {
            (eng::body(), p.text_primary, *line)
        };
        let heading = body.len() != line.len();
        for (i, part) in body.split('`').enumerate() {
            let code = i % 2 == 1 && !heading;
            job.append(
                part,
                0.0,
                TextFormat {
                    font_id: if code {
                        eng::value_font()
                    } else {
                        font.clone()
                    },
                    color: if code {
                        color(p.accent_text)
                    } else {
                        color(ink_color)
                    },
                    background: if code {
                        color(p.surface_sunken)
                    } else {
                        egui::Color32::TRANSPARENT
                    },
                    line_height: Some(if heading { 28.0 } else { 19.0 }),
                    ..Default::default()
                },
            );
        }
        if n + 1 < lines.len() {
            job.append("\n", 0.0, TextFormat::default());
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodalia_theme::{Accent, Mode};

    fn look() -> Look {
        Look::new(Mode::Light.palette_with(Accent::Abaco))
    }

    #[test]
    fn the_highlighted_job_holds_the_text_exactly() {
        for text in [
            "",
            "L = 2.5 m  # length",
            "x = [1, 2; 3, 4]\ny = 5 km -> mi",
            "é = 3 µm",
        ] {
            let job = highlight_job(&look(), text, None, 400.0);
            assert_eq!(job.text, text);
        }
    }

    #[test]
    fn only_the_span_an_error_points_at_is_underlined() {
        let text = "a = 1 m + 2 s";
        let job = highlight_job(&look(), text, Some((6, 13)), 400.0);
        assert_eq!(job.text, text);
        let underlined: String = job
            .sections
            .iter()
            .filter(|s| s.format.underline != Stroke::NONE)
            .map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(underlined, "m + 2 s");
    }

    #[test]
    fn a_span_that_is_not_on_a_char_boundary_is_ignored() {
        // A stale span from before an edit may cut a character in two.
        let text = "é = 1";
        let job = highlight_job(&look(), text, Some((1, 3)), 400.0);
        assert_eq!(job.text, text);
    }

    #[test]
    fn markdown_keeps_every_line_and_drops_the_heading_marks() {
        let job = markdown_job(&look(), "# Title\nSome `code` here\n\n## Sub", 400.0);
        assert_eq!(job.text, "Title\nSome code here\n\nSub");
    }
}
