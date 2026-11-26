// SPDX-License-Identifier: MIT OR Apache-2.0
//! The script workspace: the variable browser, the editor, the command window and the figures.
//! Like the rest of the window it only draws and collects: what the person asked for goes into
//! [`Out`] and the app carries it out after the frame.

use std::time::Duration;

use abaco_app::commands::Cmd;
use abaco_app::i18n::{tr, trf};
use abaco_app::script::engine::Job;
use abaco_app::script::session::{selection_or_line, Kind, Session};
use egui::text::{CCursor, CCursorRange};
use egui::{pos2, vec2, Align2, Id, Key, Modifiers, Rect, RichText, Sense, Ui};
use nodalia_theme::engineering::{self as eng, layout, Look};
use nodalia_theme::for_egui::color;

use crate::app::App;
use crate::figure::{self, FigureView};
use crate::ui::island;
use crate::widgets;

const MIN_CENTER: f32 = 420.0;
const EDITOR_SHARE: f32 = 0.55;
const INPUT_HEIGHT: f32 = 34.0;
const VAR_ROW: f32 = 40.0;

/// What the person did this frame.
#[derive(Default)]
pub struct Out {
    pub jobs: Vec<Job>,
    pub cmds: Vec<Cmd>,
}

/// Lays out the three columns of the script workspace inside `body`.
pub fn body(ui: &mut Ui, look: &Look, app: &mut App, body: Rect, out: &mut Out) {
    let gap = layout::GAP;
    let left_w = layout::BROWSER_WIDTH + 40.0;
    let right_w = (body.width() * 0.38).clamp(340.0, 720.0);
    let mut left = app.settings.show_workspace_panel;
    let mut right = app.settings.show_figures_panel;
    let room = |l: bool, r: bool| {
        body.width() - if l { left_w + gap } else { 0.0 } - if r { right_w + gap } else { 0.0 }
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
        script,
        figure_view,
        script_lost,
        ..
    } = app;
    if left {
        let r = Rect::from_min_max(body.min, pos2(body.left() + left_w, body.bottom()));
        island(ui, look, r, "workspace", |ui| {
            workspace_panel(ui, look, script, out)
        });
        x0 = r.right() + gap;
    }
    if right {
        let r = Rect::from_min_max(pos2(body.right() - right_w, body.top()), body.max);
        island(ui, look, r, "figures", |ui| {
            figures_panel(ui, look, script, figure_view)
        });
        x1 = r.left() - gap;
    }
    let editor_h = ((body.height() - gap) * EDITOR_SHARE).round();
    let top = Rect::from_min_max(pos2(x0, body.top()), pos2(x1, body.top() + editor_h));
    let bottom = Rect::from_min_max(pos2(x0, top.bottom() + gap), pos2(x1, body.bottom()));
    island(ui, look, top, "script-editor-island", |ui| {
        editor(ui, look, script, out)
    });
    island(ui, look, bottom, "command-window", |ui| {
        command_window(ui, look, script, *script_lost, out)
    });
}

// --- Editor -------------------------------------------------------------------------------------

fn editor(ui: &mut Ui, look: &Look, s: &mut Session, out: &mut Out) {
    let p = &look.palette;
    let mut name = s
        .file_name()
        .unwrap_or_else(|| tr("Untitled script").to_owned());
    if s.dirty {
        name.push_str(" •");
    }
    eng::panel_header(ui, look, &name, |ui| {
        if let Some(line) = s.error_line {
            ui.label(
                RichText::new(trf("Error on line {}", &[&line.to_string()]))
                    .font(eng::body())
                    .color(color(p.status_danger_fg)),
            );
        }
    });
    let id = Id::unique("script-editor");
    let focused = ui.memory(|m| m.has_focus(id));
    // Ctrl+Enter runs the selection, or the line the cursor is on.
    let run_part = focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    if run_part {
        let range = egui::TextEdit::load_state(ui.ctx(), id).and_then(|st| st.cursor.char_range());
        let (a, b) = range.map_or((0, 0), |r| {
            (usize::from(r.primary.index), usize::from(r.secondary.index))
        });
        let text = selection_or_line(&s.editor, a, b);
        if let Some(job) = s.run_text(&text) {
            out.jobs.push(job);
        }
    }
    egui::ScrollArea::both()
        .id_salt("script-editor-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let shown = egui::TextEdit::multiline(&mut s.editor)
                .id(id)
                .font(eng::value_font())
                .frame(egui::Frame::NONE)
                .lock_focus(true)
                .desired_width(f32::INFINITY)
                .desired_rows(12)
                .hint_text(tr(
                    "Write a script here. F5 runs it, Ctrl+Enter runs the selection.",
                ))
                .show(ui);
            if shown.response.changed() {
                s.dirty = true;
                s.error_line = None;
            }
        });
}

// --- Command window -----------------------------------------------------------------------------

fn command_window(ui: &mut Ui, look: &Look, s: &mut Session, lost: bool, out: &mut Out) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Command window"), |ui| {
        if s.running {
            ui.label(
                RichText::new(tr("Running…"))
                    .font(eng::body())
                    .color(color(p.accent_text)),
            );
        } else if lost {
            ui.label(
                RichText::new(tr("The script interpreter stopped"))
                    .font(eng::body())
                    .color(color(p.status_danger_fg)),
            );
        }
    });
    let log_h = (ui.available_height() - INPUT_HEIGHT - 8.0).max(40.0);
    log(ui, look, s, log_h);
    ui.add_space(6.0);
    input_row(ui, look, s, out);
}

/// The output so far, one display row per line of text, drawn only where it is visible.
fn log(ui: &mut Ui, look: &Look, s: &Session, height: f32) {
    let p = &look.palette;
    let font = eng::value_font();
    let row_h = (font.size * 1.4).round();
    let char_w = font.size * 0.62;
    let rows: Vec<(Kind, &str)> = s
        .lines
        .iter()
        .flat_map(|l| l.text.split('\n').map(move |t| (l.kind, t)))
        .collect();
    let widest = rows
        .iter()
        .map(|(_, t)| t.chars().count())
        .max()
        .unwrap_or(0);
    let width = (widest as f32 * char_w + 16.0).max(ui.available_width());
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .id_salt("command-log")
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            for i in range {
                let Some((kind, text)) = rows.get(i) else {
                    continue;
                };
                let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                let ink = match kind {
                    Kind::Input => p.accent_text,
                    Kind::Output => p.text_primary,
                    Kind::Warning => p.status_warning_fg,
                    Kind::Error => p.status_danger_fg,
                    Kind::Info => p.text_tertiary,
                };
                ui.painter().text(
                    rect.left_center() + vec2(4.0, 0.0),
                    Align2::LEFT_CENTER,
                    text,
                    font.clone(),
                    color(ink),
                );
            }
        });
}

fn input_row(ui: &mut Ui, look: &Look, s: &mut Session, out: &mut Out) {
    let p = &look.palette;
    let id = Id::unique("script-command-input");
    // The arrows browse the history, so they are taken before the field moves its cursor.
    if ui.memory(|m| m.has_focus(id)) {
        let up = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
        let down = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
        if up || down {
            if up {
                s.history_up();
            } else {
                s.history_down();
            }
            let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let end = CCursor::new(s.input.chars().count());
            state.cursor.set_char_range(Some(CCursorRange::one(end)));
            state.store(ui.ctx(), id);
        }
    }
    widgets::field(look).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(">>")
                    .font(eng::value_font())
                    .color(color(p.accent_text)),
            );
            let shown = egui::TextEdit::singleline(&mut s.input)
                .id(id)
                .font(eng::value_font())
                .frame(egui::Frame::NONE)
                .hint_text(tr("Type a command and press Enter"))
                .align(Align2::LEFT_CENTER)
                .desired_width(f32::INFINITY)
                .show(ui);
            let enter = shown.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if enter {
                if let Some(job) = s.submit() {
                    out.jobs.push(job);
                }
                shown.response.request_focus();
            }
        });
    });
}

// --- Variables ----------------------------------------------------------------------------------

fn workspace_panel(ui: &mut Ui, look: &Look, s: &mut Session, out: &mut Out) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Workspace"), |ui| {
        ui.label(
            RichText::new(s.vars.len().to_string())
                .font(eng::value_font())
                .color(color(p.text_tertiary)),
        );
    });
    let list_h = (ui.available_height() * 0.5).max(80.0);
    let mut picked: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("variables-scroll")
        .max_height(list_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if s.vars.is_empty() {
                ui.label(
                    RichText::new(tr("No variables yet."))
                        .font(eng::body())
                        .color(color(p.text_tertiary)),
                );
            }
            for v in &s.vars {
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), VAR_ROW), Sense::click());
                let selected = s.selected_var.as_deref() == Some(v.name.as_str());
                if selected {
                    ui.painter().rect_filled(rect, 7.0, color(p.accent_subtle));
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 7.0, color(p.control_bg_hover));
                }
                let clip = ui.painter().with_clip_rect(rect.shrink2(vec2(2.0, 0.0)));
                clip.text(
                    rect.left_top() + vec2(8.0, 5.0),
                    Align2::LEFT_TOP,
                    &v.name,
                    eng::value_font(),
                    color(p.accent_text),
                );
                clip.text(
                    rect.right_top() + vec2(-8.0, 6.0),
                    Align2::RIGHT_TOP,
                    v.class.clone(),
                    eng::label(),
                    color(p.text_tertiary),
                );
                clip.text(
                    rect.left_top() + vec2(8.0, 22.0),
                    Align2::LEFT_TOP,
                    &v.summary,
                    eng::body(),
                    color(p.text_secondary),
                );
                if response.clicked() {
                    picked = Some(v.name.clone());
                }
            }
        });
    if let Some(name) = picked {
        out.jobs.push(s.select_var(&name));
    }
    ui.add_space(8.0);
    let selected = s.selected_var.clone();
    eng::panel_header(ui, look, tr("Value"), |ui| {
        if let Some(name) = &selected {
            if widgets::icon_button(ui, look, "delete", tr("Delete the variable"), 28.0, false)
                .clicked()
            {
                out.jobs.push(Job::Remove(name.clone()));
            }
        }
    });
    egui::ScrollArea::both()
        .id_salt("inspector-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if selected.is_none() {
                ui.label(
                    RichText::new(tr("Select a variable to see what it holds."))
                        .font(eng::body())
                        .color(color(p.text_tertiary)),
                );
            } else {
                ui.add(
                    egui::Label::new(
                        RichText::new(&s.inspected)
                            .font(eng::value_font())
                            .color(color(p.text_primary)),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
            }
        });
}

// --- Figures ------------------------------------------------------------------------------------

fn figures_panel(ui: &mut Ui, look: &Look, s: &mut Session, view: &mut FigureView) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Figures"), |_| {});
    if s.figures.list.is_empty() {
        ui.label(
            RichText::new(tr("No figures yet. Try plot(1:10)."))
                .font(eng::body())
                .color(color(p.text_tertiary)),
        );
        return;
    }
    let numbers: Vec<usize> = s.figures.list.iter().map(|(n, _)| *n).collect();
    if !numbers.contains(&s.figure) {
        s.figure = numbers.first().copied().unwrap_or(0);
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for n in &numbers {
            let label = format!("{} {n}", tr("Figure"));
            if eng::ribbon_tab(ui, look, &label, s.figure == *n).clicked() {
                s.figure = *n;
            }
        }
    });
    ui.add_space(4.0);
    let current = s.figure;
    if let Some((_, fig)) = s.figures.list.iter().find(|(n, _)| *n == current) {
        figure::show(ui, look, fig, view, Id::unique(("figure", current)));
    }
}

// --- Status bar ---------------------------------------------------------------------------------

pub fn status(ui: &mut Ui, look: &Look, app: &App) {
    let p = &look.palette;
    let s = &app.script;
    eng::status_bar(
        ui,
        |ui| {
            eng::status_value(
                ui,
                look,
                &trf("Variables: {}", &[&s.vars.len().to_string()]),
            );
            ui.label(RichText::new("·").color(color(p.text_tertiary)));
            if app.script_lost {
                ui.label(
                    RichText::new(tr("The script interpreter stopped"))
                        .color(color(p.status_danger_fg)),
                );
            } else if s.running {
                ui.label(RichText::new(tr("Running…")).color(color(p.accent_text)));
            } else if let Some(ms) = s.last_millis {
                let time = crate::panels::duration_text(Duration::from_millis(ms));
                ui.label(
                    RichText::new(trf("Last run: {}", &[&time])).color(color(p.text_secondary)),
                );
            } else {
                ui.label(RichText::new(tr("Ready")).color(color(p.text_secondary)));
            }
        },
        |ui| {
            eng::status_value(
                ui,
                look,
                &trf("Figures: {}", &[&s.figures.list.len().to_string()]),
            );
        },
    );
}
