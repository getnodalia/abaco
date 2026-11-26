// SPDX-License-Identifier: MIT OR Apache-2.0
//! Everything that opens over the document: command search, Preferences, Keyboard shortcuts,
//! About and the question about unsaved changes. Each is a modal, closed with Escape, its close
//! button or a click outside it.

use abaco_app::commands::{Category, Cmd};
use abaco_app::i18n::{self, tr, trf, Lang};
use abaco_app::settings::Appearance;
use egui::{vec2, Align, Align2, Id, Key, Layout, RichText, Sense, Ui};
use nodalia_theme::about::{self, Info, Link};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::color;

use crate::app::{App, Overlay, Pending};
use crate::widgets::{creator, primary_button, secondary_button};

type Shown = (Overlay, bool);

fn modal(
    ctx: &egui::Context,
    look: &Look,
    id: &'static str,
    width: f32,
    top: bool,
    add: impl FnOnce(&mut Ui),
) -> bool {
    let p = &look.palette;
    let frame = eng::island(look)
        .inner_margin(16)
        .stroke(egui::Stroke::new(1.0, color(p.border_subtle)));
    let mut modal = egui::Modal::new(Id::unique(id)).frame(frame);
    if top {
        modal = modal.area(
            egui::Area::new(Id::unique(id))
                .kind(egui::UiKind::Modal)
                .anchor(Align2::CENTER_TOP, vec2(0.0, 96.0))
                .order(egui::Order::Foreground),
        );
    }
    let response = modal.show(ctx, |ui| {
        ui.set_width(width.min(ctx.content_rect().width() - 40.0));
        add(ui);
    });
    response.should_close()
}

pub fn show(ctx: &egui::Context, app: &mut App, look: &Look) {
    let overlay = std::mem::replace(&mut app.overlay, Overlay::None);
    let (next, close) = match overlay {
        Overlay::None => return,
        Overlay::Search(query) => search(ctx, app, look, query),
        Overlay::Preferences => preferences(ctx, app, look),
        Overlay::Shortcuts(filter) => shortcuts(ctx, look, filter),
        Overlay::About => about_window(ctx, app, look),
        Overlay::Confirm(pending) => confirm(ctx, app, look, pending),
    };
    // A command run from inside an overlay may have opened another one: keep that.
    if matches!(app.overlay, Overlay::None) && !close {
        app.overlay = next;
    }
}

fn heading(ui: &mut Ui, look: &Look, title: &str, id: &'static str) -> bool {
    about::heading(ui, &creator(look), title, Id::unique((id, "close"))).closed
}

fn section(ui: &mut Ui, look: &Look, title: &str) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(title)
            .font(eng::panel_title())
            .color(color(look.palette.text_secondary)),
    );
    ui.add_space(4.0);
}

// --- Command search -----------------------------------------------------------------------------

fn search(ctx: &egui::Context, app: &mut App, look: &Look, mut query: String) -> Shown {
    let mac = cfg!(target_os = "macos");
    let p = look.palette;
    let sel_id = Id::unique("abaco-search-selection");
    let mut sel: usize = ctx.data(|d| d.get_temp(sel_id)).unwrap_or(0);
    let needle = query.to_lowercase();
    let matches: Vec<Cmd> = Cmd::ALL
        .iter()
        .copied()
        .filter(|c| needle.is_empty() || c.translated().to_lowercase().contains(&needle))
        .collect();
    sel = sel.min(matches.len().saturating_sub(1));
    let (down, up, enter) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::Enter),
        )
    });
    if down && sel + 1 < matches.len() {
        sel += 1;
    }
    if up {
        sel = sel.saturating_sub(1);
    }
    let mut chosen = if enter {
        matches.get(sel).copied()
    } else {
        None
    };
    let dismissed = modal(ctx, look, "abaco-search", 560.0, true, |ui| {
        crate::widgets::field(look).show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .frame(egui::Frame::NONE)
                    .font(eng::body())
                    .hint_text(tr("Type a command"))
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY),
            );
            if !response.has_focus() && !response.lost_focus() {
                response.request_focus();
            }
        });
        ui.add_space(8.0);
        if matches.is_empty() {
            ui.label(
                RichText::new(tr("No command matches."))
                    .font(eng::body())
                    .color(color(p.text_secondary)),
            );
        }
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .id_salt("search-scroll")
            .show(ui, |ui| {
                for (i, cmd) in matches.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if i == sel || response.hovered() {
                        ui.painter().rect_filled(rect, 7.0, color(p.accent_subtle));
                    }
                    let ink = if i == sel {
                        color(p.accent_text)
                    } else {
                        color(p.text_primary)
                    };
                    ui.painter().text(
                        rect.left_center() + vec2(10.0, 0.0),
                        Align2::LEFT_CENTER,
                        cmd.translated(),
                        eng::body(),
                        ink,
                    );
                    if let Some(s) = cmd.shortcut_label(mac) {
                        ui.painter().text(
                            rect.right_center() - vec2(10.0, 0.0),
                            Align2::RIGHT_CENTER,
                            s,
                            eng::value_font(),
                            color(p.text_secondary),
                        );
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, cmd.translated())
                    });
                    if response.clicked() {
                        chosen = Some(*cmd);
                    }
                }
            });
    });
    ctx.data_mut(|d| d.insert_temp(sel_id, sel));
    if let Some(cmd) = chosen {
        app.run(cmd);
        return (Overlay::None, true);
    }
    if dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Search(query), false)
    }
}

// --- Preferences --------------------------------------------------------------------------------

fn preferences(ctx: &egui::Context, app: &mut App, look: &Look) -> Shown {
    let mut changed = false;
    let mut closed = false;
    let dismissed = modal(ctx, look, "abaco-preferences", 520.0, false, |ui| {
        closed = heading(ui, look, tr("Preferences"), "prefs");
        egui::ScrollArea::vertical()
            .id_salt("prefs-scroll")
            .max_height((ui.ctx().content_rect().height() - 180.0).max(200.0))
            .show(ui, |ui| {
                let s = &mut app.settings;
                section(ui, look, tr("Appearance"));
                ui.horizontal(|ui| {
                    for (a, label) in [
                        (Appearance::System, tr("System")),
                        (Appearance::Light, tr("Light")),
                        (Appearance::Dark, tr("Dark")),
                    ] {
                        if ui.selectable_label(s.appearance == a, label).clicked() {
                            s.appearance = a;
                            changed = true;
                        }
                    }
                });
                changed |= ui
                    .checkbox(&mut s.high_contrast, tr("High contrast"))
                    .changed();
                changed |= ui
                    .checkbox(&mut s.reduced_motion, tr("Reduce motion"))
                    .changed();
                if crate::chrome::NDL_CONTROLS_POSSIBLE {
                    changed |= ui
                        .checkbox(
                            &mut s.native_window_controls,
                            tr("Use the desktop's own window buttons"),
                        )
                        .changed();
                }

                section(ui, look, tr("Language"));
                ui.horizontal(|ui| {
                    for (code, label) in
                        [("auto", tr("System")), ("es", "Español"), ("en", "English")]
                    {
                        if ui.selectable_label(s.language == code, label).clicked() {
                            s.language = code.to_owned();
                            i18n::set(Lang::from_setting(code));
                            changed = true;
                        }
                    }
                });

                section(ui, look, tr("Numbers"));
                ui.horizontal(|ui| {
                    ui.label(tr("Significant digits in a new document"));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut s.significant_digits)
                                .range(1..=17)
                                .speed(0.1),
                        )
                        .changed();
                });
                ui.label(
                    RichText::new(tr(
                        "A document keeps the number of digits it was saved with.",
                    ))
                    .font(eng::caption())
                    .color(color(look.palette.text_tertiary)),
                );
            });
    });
    if changed {
        app.save_settings();
    }
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Preferences, false)
    }
}

// --- Keyboard shortcuts -------------------------------------------------------------------------

fn shortcuts(ctx: &egui::Context, look: &Look, mut filter: String) -> Shown {
    let mac = cfg!(target_os = "macos");
    let p = look.palette;
    let mut closed = false;
    let dismissed = modal(ctx, look, "abaco-shortcuts", 560.0, false, |ui| {
        closed = heading(ui, look, tr("Keyboard shortcuts"), "keys");
        ui.add_space(8.0);
        crate::widgets::field(look).show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .frame(egui::Frame::NONE)
                    .font(eng::body())
                    .hint_text(tr("Filter commands…"))
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY),
            );
        });
        ui.add_space(8.0);
        let needle = filter.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("keys-scroll")
            .max_height((ui.ctx().content_rect().height() - 220.0).max(200.0))
            .show(ui, |ui| {
                for category in Category::ALL {
                    let rows: Vec<Cmd> = Cmd::ALL
                        .iter()
                        .copied()
                        .filter(|c| c.category() == category)
                        .filter(|c| {
                            needle.is_empty() || c.translated().to_lowercase().contains(&needle)
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    section(ui, look, tr(category.label()));
                    for cmd in rows {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(cmd.translated())
                                    .font(eng::body())
                                    .color(color(p.text_primary)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(s) = cmd.shortcut_label(mac) {
                                    ui.label(
                                        RichText::new(s)
                                            .font(eng::value_font())
                                            .color(color(p.text_secondary)),
                                    );
                                }
                            });
                        });
                    }
                }
            });
    });
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Shortcuts(filter), false)
    }
}

// --- About --------------------------------------------------------------------------------------

fn about_window(ctx: &egui::Context, app: &mut App, look: &Look) -> Shown {
    let version = trf("Version {}", &[env!("CARGO_PKG_VERSION")]);
    let links = [
        Link {
            label: tr("Website"),
            url: "https://getnodalia.com/abaco",
        },
        Link {
            label: tr("Source code"),
            url: "https://github.com/getnodalia/abaco",
        },
        Link {
            label: tr("Report a problem"),
            url: "https://github.com/getnodalia/abaco/issues",
        },
    ];
    let footer = format!(
        "MIT OR Apache-2.0 · © Nodalia · {}",
        tr("Icon: original work.")
    );
    let info = Info {
        name: abaco_app::NAME,
        version: &version,
        tagline: tr(
            "Maths documents with units, matrices and functions that recompute as you write.",
        ),
        suite: tr("Part of Nodalia · Engineering"),
        links: &links,
        footer: &footer,
    };
    let texture = app.icon(ctx);
    let mut closed = false;
    let dismissed = modal(
        ctx,
        look,
        "abaco-about",
        about::measures::WIDTH,
        false,
        |ui| {
            closed = heading(ui, look, tr("About Ábaco"), "about");
            let page = about::page(ui, &creator(look), &info, |ui, rect| {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            });
            if let Some(link) = page.picked.and_then(|i| links.get(i)) {
                ui.ctx().open_url(egui::OpenUrl::new_tab(link.url));
            }
        },
    );
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::About, false)
    }
}

// --- Unsaved changes ----------------------------------------------------------------------------

fn confirm(ctx: &egui::Context, app: &mut App, look: &Look, pending: Pending) -> Shown {
    let name = app.dirty_title();
    let mut answer: Option<u8> = None;
    let dismissed = modal(ctx, look, "abaco-confirm", 440.0, false, |ui| {
        if heading(ui, look, tr("Save your changes?"), "confirm") {
            answer = Some(2);
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(trf("{} has changes that are not saved.", &[&name]))
                .font(eng::body())
                .color(color(look.palette.text_primary)),
        );
        ui.add_space(16.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, look, tr("Save")).clicked() {
                answer = Some(0);
            }
            if secondary_button(ui, look, tr("Don't save")).clicked() {
                answer = Some(1);
            }
            if secondary_button(ui, look, tr("Cancel")).clicked() {
                answer = Some(2);
            }
        });
    });
    match answer {
        Some(0) => {
            app.save_then(pending);
            (Overlay::None, true)
        }
        Some(1) => {
            app.proceed(pending);
            (Overlay::None, true)
        }
        Some(_) => (Overlay::None, true),
        None if dismissed => (Overlay::None, true),
        None => (Overlay::Confirm(pending), false),
    }
}
