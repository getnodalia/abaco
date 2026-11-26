// SPDX-License-Identifier: MIT OR Apache-2.0
//! The panels around the document: the outline, the units and constants with their converter,
//! the document's settings, the status bar and Home.

use std::path::Path;
use std::time::Duration;

use abaco_app::catalog::{self, ConstantRow, UnitRow};
use abaco_app::commands::Cmd;
use abaco_app::engine::Outcome;
use abaco_app::i18n::{tr, translate, trf};
use abaco_app::settings::{Settings, SidePanel};
use abaco_app::workbook::{Kind, Msg, Workbook};
use egui::{vec2, RichText, Ui};
use nodalia_theme::engineering::{self as eng, Look, StartCard, TreeHit, TreeRow};
use nodalia_theme::for_egui::color;

use crate::app::Pending;

/// What the units panel remembers between frames.
pub struct UnitsState {
    pub search: String,
    pub convert_from: String,
    pub convert_to: String,
    units: Vec<UnitRow>,
    categories: Vec<String>,
    constants: Option<(usize, Vec<ConstantRow>)>,
}

impl Default for UnitsState {
    fn default() -> UnitsState {
        UnitsState {
            search: String::new(),
            convert_from: "5 km".to_owned(),
            convert_to: String::new(),
            units: catalog::units(),
            categories: catalog::categories(),
            constants: None,
        }
    }
}

// --- Outline ------------------------------------------------------------------------------------

/// The cells as a list: a click selects one and scrolls to it.
pub fn outline(ui: &mut Ui, look: &Look, wb: &mut Workbook, msgs: &mut Vec<Msg>) {
    let (maths, texts) = wb.counts();
    eng::panel_header(ui, look, tr("Outline"), |ui| {
        ui.label(
            RichText::new(format!("{}", maths + texts))
                .font(eng::value_font())
                .color(color(look.palette.text_tertiary)),
        );
    });
    // Long titles are cut at the panel edge, not painted over the island's padding.
    let bounds = ui.available_rect_before_wrap();
    ui.set_clip_rect(ui.clip_rect().intersect(bounds));
    egui::ScrollArea::vertical()
        .id_salt("outline-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut picked = None;
            for cell in &wb.cells {
                let failed = matches!(cell.outcome, Outcome::Err { .. });
                let title = cell.title();
                let label = if title.is_empty() {
                    tr("Empty cell").to_owned()
                } else {
                    title
                };
                let icon = match (cell.kind, failed) {
                    (_, true) => "warning",
                    (Kind::Math, false) => "calculator",
                    (Kind::Text, false) => "text",
                };
                let (_, hit) = eng::tree_row(
                    ui,
                    look,
                    &TreeRow {
                        depth: 0,
                        expanded: None,
                        icon,
                        label: &label,
                        selected: wb.selected == Some(cell.id),
                        muted: cell.is_blank(),
                    },
                );
                if hit != TreeHit::None {
                    picked = Some(cell.id);
                }
            }
            if let Some(id) = picked {
                wb.reveal = Some(id);
                msgs.push(Msg::Select(id));
            }
        });
}

// --- Side panel ---------------------------------------------------------------------------------

pub fn side_panel(
    ui: &mut Ui,
    look: &Look,
    settings: &mut Settings,
    units: &mut UnitsState,
    wb: &Workbook,
    msgs: &mut Vec<Msg>,
) -> bool {
    let mut settings_changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (panel, cmd) in [
            (SidePanel::Units, Cmd::ShowUnits),
            (SidePanel::Document, Cmd::ShowDocumentSettings),
        ] {
            if eng::ribbon_tab(ui, look, tab_label(cmd), settings.side_panel == panel).clicked()
                && settings.side_panel != panel
            {
                settings.side_panel = panel;
                settings_changed = true;
            }
        }
    });
    ui.add_space(4.0);
    match settings.side_panel {
        SidePanel::Units => units_panel(ui, look, settings.significant_digits, units, wb, msgs),
        SidePanel::Document => document_panel(ui, look, wb, msgs),
    }
    settings_changed
}

fn search_field(ui: &mut Ui, look: &Look, text: &mut String, hint: &str) {
    crate::widgets::field(look).show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(text)
                .frame(egui::Frame::NONE)
                .font(eng::body())
                .hint_text(hint)
                .align(egui::Align2::LEFT_CENTER)
                .desired_width(f32::INFINITY),
        );
    });
}

fn units_panel(
    ui: &mut Ui,
    look: &Look,
    digits: usize,
    st: &mut UnitsState,
    wb: &Workbook,
    msgs: &mut Vec<Msg>,
) {
    let p = &look.palette;
    egui::ScrollArea::vertical()
        .id_salt("units-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            eng::panel_header(ui, look, tr("Convert"), |_| {});
            search_field(
                ui,
                look,
                &mut st.convert_from,
                tr("A quantity, for example 5 km"),
            );
            ui.add_space(4.0);
            search_field(ui, look, &mut st.convert_to, tr("To a unit (optional)"));
            ui.add_space(6.0);
            conversion(ui, look, st, digits);

            ui.add_space(10.0);
            eng::panel_header(ui, look, tr("Constants"), |_| {});
            if st.constants.as_ref().map(|c| c.0) != Some(digits) {
                st.constants = Some((digits, catalog::constants_table(digits)));
            }
            let can_insert = wb.selected.is_some();
            if let Some((_, rows)) = &st.constants {
                for row in rows {
                    let r = ui
                        .horizontal(|ui| {
                            ui.label(
                                RichText::new(row.name)
                                    .font(eng::value_font())
                                    .color(color(p.accent_text)),
                            );
                            ui.label(
                                RichText::new(translate(row.description))
                                    .font(eng::body())
                                    .color(color(p.text_secondary)),
                            );
                        })
                        .response
                        .interact(egui::Sense::click());
                    ui.label(
                        RichText::new(&row.value)
                            .font(eng::value_font())
                            .color(color(p.text_primary)),
                    );
                    ui.add_space(3.0);
                    if r.on_hover_text(tr("Click to insert in the selected cell"))
                        .clicked()
                        && can_insert
                    {
                        msgs.push(Msg::InsertIntoSelected(row.name.to_owned()));
                    }
                }
            }

            ui.add_space(10.0);
            eng::panel_header(ui, look, tr("Units"), |_| {});
            search_field(ui, look, &mut st.search, tr("Search units"));
            ui.add_space(4.0);
            let searching = !st.search.trim().is_empty();
            for category in &st.categories {
                let rows: Vec<&UnitRow> = st
                    .units
                    .iter()
                    .filter(|u| &u.category == category)
                    .filter(|u| {
                        !searching
                            || catalog::matches(
                                &st.search,
                                &[u.symbol, u.name, translate(u.name), category],
                            )
                    })
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(
                    RichText::new(translate(category))
                        .font(eng::label())
                        .color(color(p.text_primary)),
                )
                .id_salt(("units", category))
                .open(searching.then_some(true))
                .show(ui, |ui| {
                    for u in rows {
                        let r = ui
                            .horizontal(|ui| {
                                ui.add_sized(
                                    vec2(56.0, 18.0),
                                    egui::Label::new(
                                        RichText::new(u.symbol)
                                            .font(eng::value_font())
                                            .color(color(p.accent_text)),
                                    ),
                                );
                                ui.label(
                                    RichText::new(translate(u.name))
                                        .font(eng::body())
                                        .color(color(p.text_secondary)),
                                );
                            })
                            .response
                            .interact(egui::Sense::click());
                        if r.on_hover_text(tr("Click to insert in the selected cell"))
                            .clicked()
                            && can_insert
                        {
                            msgs.push(Msg::InsertIntoSelected(u.symbol.to_owned()));
                        }
                    }
                });
            }
        });
}

fn conversion(ui: &mut Ui, look: &Look, st: &UnitsState, digits: usize) {
    let p = &look.palette;
    let from = st.convert_from.trim();
    if from.is_empty() {
        return;
    }
    let to = st.convert_to.trim();
    let outcome: Result<Vec<(String, String)>, String> = if to.is_empty() {
        catalog::convert_all(from, digits)
            .map(|rows| rows.into_iter().map(|(u, v)| (u.to_owned(), v)).collect())
    } else {
        catalog::convert(from, to, digits).map(|v| vec![(to.to_owned(), v)])
    };
    match outcome {
        Err(message) if message.is_empty() => {}
        Err(message) => {
            ui.label(
                RichText::new(message)
                    .font(eng::body())
                    .color(color(p.status_danger_fg)),
            );
        }
        Ok(rows) => {
            egui::Grid::new("conversions")
                .num_columns(2)
                .spacing(vec2(12.0, 3.0))
                .show(ui, |ui| {
                    for (unit, value) in rows.iter().take(12) {
                        ui.label(
                            RichText::new(unit)
                                .font(eng::value_font())
                                .color(color(p.accent_text)),
                        );
                        ui.label(
                            RichText::new(value)
                                .font(eng::value_font())
                                .color(color(p.text_primary)),
                        );
                        ui.end_row();
                    }
                });
            if rows.len() > 12 {
                ui.label(
                    RichText::new(trf(
                        "and {} more. Name a unit above to see one.",
                        &[&(rows.len() - 12).to_string()],
                    ))
                    .font(eng::caption())
                    .color(color(p.text_tertiary)),
                );
            }
        }
    }
}

fn document_panel(ui: &mut Ui, look: &Look, wb: &Workbook, msgs: &mut Vec<Msg>) {
    let p = &look.palette;
    eng::panel_header(ui, look, tr("Evaluation"), |_| {});
    eng::properties(ui, "document", |ui| {
        eng::property(ui, look, tr("Significant digits"), |ui| {
            let mut digits = wb.digits;
            if ui
                .add(egui::DragValue::new(&mut digits).range(1..=17).speed(0.1))
                .changed()
            {
                msgs.push(Msg::SetDigits(digits));
            }
        });
        eng::property(ui, look, tr("Order"), |ui| {
            let mut strict = wb.strict_order;
            if ui
                .checkbox(&mut strict, tr("A cell sees only the cells above it"))
                .changed()
            {
                msgs.push(Msg::SetStrictOrder(strict));
            }
        });
    });
    ui.add_space(10.0);
    eng::panel_header(ui, look, tr("Document"), |_| {});
    let (maths, texts) = wb.counts();
    eng::properties(ui, "document-info", |ui| {
        eng::property(ui, look, tr("File"), |ui| {
            let text = wb.path.as_deref().map_or_else(
                || tr("Not saved yet").to_owned(),
                |p| p.display().to_string(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .font(eng::body())
                        .color(color(p.text_primary)),
                )
                .wrap(),
            );
        });
        eng::property(ui, look, tr("Maths cells"), |ui| {
            eng::value(ui, look, &maths.to_string())
        });
        eng::property(ui, look, tr("Text cells"), |ui| {
            eng::value(ui, look, &texts.to_string())
        });
        eng::property(ui, look, tr("With errors"), |ui| {
            eng::value(ui, look, &wb.errors.to_string())
        });
        eng::property(ui, look, tr("Last evaluation"), |ui| {
            eng::value(ui, look, &duration_text(wb.elapsed))
        });
    });
}

pub fn duration_text(d: Duration) -> String {
    let micros = d.as_micros();
    if micros < 1000 {
        format!("{micros} µs")
    } else if micros < 1_000_000 {
        format!("{:.1} ms", micros as f64 / 1000.0)
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

// --- Status bar ---------------------------------------------------------------------------------

pub fn status(ui: &mut Ui, look: &Look, wb: &Workbook, settings: &Settings, lost: bool) {
    let p = &look.palette;
    eng::status_bar(
        ui,
        |ui| {
            let (maths, texts) = wb.counts();
            eng::status_value(ui, look, &count_text(maths + texts));
            ui.label(RichText::new("·").color(color(p.text_tertiary)));
            if wb.errors > 0 {
                ui.label(
                    RichText::new(trf("{} with errors", &[&wb.errors.to_string()]))
                        .color(color(p.status_danger_fg)),
                );
            } else {
                ui.label(RichText::new(tr("No errors")).color(color(p.status_success_fg)));
            }
            ui.label(RichText::new("·").color(color(p.text_tertiary)));
            if lost {
                ui.label(
                    RichText::new(tr("The calculation engine stopped"))
                        .color(color(p.status_danger_fg)),
                );
            } else if wb.busy {
                ui.label(RichText::new(tr("Evaluating…")).color(color(p.text_secondary)));
            } else {
                ui.label(
                    RichText::new(evaluated_text(wb.evaluated, &duration_text(wb.elapsed)))
                        .color(color(p.text_secondary)),
                );
            }
        },
        |ui| {
            eng::status_value(
                ui,
                look,
                &trf("{} significant digits", &[&wb.digits.to_string()]),
            );
            let _ = settings;
        },
    );
}

// --- Home ---------------------------------------------------------------------------------------

/// What Home asked for.
pub enum HomeAction {
    New,
    Sample,
    Open,
    Recent(std::path::PathBuf),
}

pub fn home(
    ui: &mut Ui,
    look: &Look,
    icon: egui::TextureId,
    settings: &Settings,
) -> Option<HomeAction> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("home-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(12.0);
            eng::home_header(
                ui,
                look,
                Some(icon),
                abaco_app::NAME,
                tr("Maths documents that compute as you write."),
            );
            ui.label(
                RichText::new(tr("Start"))
                    .font(eng::panel_title())
                    .color(color(look.palette.text_primary)),
            );
            ui.add_space(8.0);
            let cards = [
                StartCard {
                    icon: "new",
                    name: tr("New document"),
                    detail: tr("A blank page with one cell"),
                },
                StartCard {
                    icon: "book",
                    name: tr("Open the example"),
                    detail: tr("A beam, a circuit, a function"),
                },
                StartCard {
                    icon: "open",
                    name: tr("Open…"),
                    detail: tr("A document from your computer"),
                },
            ];
            match eng::start_cards(ui, look, &cards) {
                Some(0) => action = Some(HomeAction::New),
                Some(1) => action = Some(HomeAction::Sample),
                Some(2) => action = Some(HomeAction::Open),
                _ => {}
            }
            ui.add_space(22.0);
            ui.label(
                RichText::new(tr("Recent documents"))
                    .font(eng::panel_title())
                    .color(color(look.palette.text_primary)),
            );
            ui.add_space(8.0);
            let names: Vec<(String, String)> = settings
                .recent
                .iter()
                .map(|p| {
                    (
                        p.file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                        parent_text(p),
                    )
                })
                .collect();
            let list: Vec<eng::RecentDocument> = names
                .iter()
                .map(|(name, folder)| eng::RecentDocument {
                    name,
                    folder,
                    when: "",
                    icon: "calculator",
                    thumbnail: None,
                })
                .collect();
            if let Some(i) = eng::recent_documents(
                ui,
                look,
                &list,
                tr("Documents you open or save appear here."),
            ) {
                if let Some(path) = settings.recent.get(i) {
                    action = Some(HomeAction::Recent(path.clone()));
                }
            }
        });
    action
}

fn parent_text(path: &Path) -> String {
    path.parent()
        .map_or_else(String::new, |p| p.display().to_string())
}

/// What a Home action means for the app.
pub fn pending_of(action: HomeAction) -> Pending {
    match action {
        HomeAction::New => Pending::NewBlank,
        HomeAction::Sample => Pending::Sample,
        HomeAction::Open => Pending::OpenDialog,
        HomeAction::Recent(path) => Pending::Open(path),
    }
}

/// Singular and plural forms are separate strings: Spanish has no template that fits both.
fn count_text(n: usize) -> String {
    if n == 1 {
        trf("{} cell", &["1"])
    } else {
        trf("{} cells", &[&n.to_string()])
    }
}

fn evaluated_text(n: usize, time: &str) -> String {
    if n == 1 {
        trf("Evaluated {} cell in {}", &["1", time])
    } else {
        trf("Evaluated {} cells in {}", &[&n.to_string(), time])
    }
}

/// Short names for the side panel's two tabs, so both fit in a narrow panel in any language.
fn tab_label(cmd: Cmd) -> &'static str {
    match cmd {
        Cmd::ShowDocumentSettings => tr("Document"),
        other => other.translated(),
    }
}
