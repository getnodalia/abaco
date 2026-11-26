// SPDX-License-Identifier: MIT OR Apache-2.0
//! The window: title bar, ribbon, outline, document, side panel and status bar. Everything here
//! only draws and collects what the person did; the app carries it out after the frame, so no
//! widget holds a borrow of the state it changes.

use abaco_app::commands::Cmd;
use abaco_app::i18n::tr;
use abaco_app::workbook::Msg;
use egui::{pos2, vec2, Id, Rect, Sense, Ui, UiBuilder};
use nodalia_theme::engineering::{
    self as eng, layout, menu, Document, Look, TitleAction, TitleBar, TitleLabels,
};
use nodalia_theme::for_egui::color;
use nodalia_theme::Mode;

use crate::app::App;
use crate::{cells, chrome, overlays, panels};

const RIBBON_H: f32 = 84.0;
const MIN_CENTER: f32 = 420.0;

fn file_menu_id() -> Id {
    Id::unique("abaco-file-menu")
}

fn app_menu_id() -> Id {
    Id::unique("abaco-app-menu")
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, color(look.palette.surface_window));

    let mut cmds: Vec<Cmd> = Vec::new();
    let mut msgs: Vec<Msg> = Vec::new();
    let gap = layout::GAP;

    let title = Rect::from_min_size(full.min, vec2(full.width(), layout::TITLE_BAR));
    title_bar(ui, title, app, &look, mode, ndl, &mut cmds);

    let ribbon = Rect::from_min_max(
        pos2(full.left() + gap, title.bottom()),
        pos2(full.right() - gap, title.bottom() + RIBBON_H),
    );
    island(ui, &look, ribbon, "ribbon", |ui| {
        ribbon_tools(ui, &look, app, &mut cmds)
    });

    let status = Rect::from_min_max(
        pos2(full.left() + gap, full.bottom() - layout::STATUS_BAR),
        pos2(full.right() - gap, full.bottom()),
    );
    let body = Rect::from_min_max(
        pos2(full.left() + gap, ribbon.bottom() + gap),
        pos2(full.right() - gap, status.top()),
    );

    // Side panels give way before the document does.
    let mut left = app.settings.show_navigator && !app.home;
    let mut right = app.settings.show_side_panel && !app.home;
    let room = |l: bool, r: bool| {
        body.width()
            - if l { layout::BROWSER_WIDTH + gap } else { 0.0 }
            - if r {
                layout::PROPERTIES_WIDTH + gap
            } else {
                0.0
            }
    };
    if room(left, right) < MIN_CENTER {
        right = false;
    }
    if room(left, right) < MIN_CENTER {
        left = false;
    }
    let mut x0 = body.left();
    let mut x1 = body.right();
    if left {
        let r = Rect::from_min_max(
            body.min,
            pos2(body.left() + layout::BROWSER_WIDTH, body.bottom()),
        );
        island(ui, &look, r, "outline", |ui| {
            panels::outline(ui, &look, &mut app.wb, &mut msgs)
        });
        x0 = r.right() + gap;
    }
    if right {
        let r = Rect::from_min_max(
            pos2(body.right() - layout::PROPERTIES_WIDTH, body.top()),
            body.max,
        );
        let mut changed = false;
        island(ui, &look, r, "side", |ui| {
            let App {
                settings,
                units,
                wb,
                ..
            } = app;
            changed = panels::side_panel(ui, &look, settings, units, wb, &mut msgs);
        });
        if changed {
            app.save_settings();
        }
        x1 = r.left() - gap;
    }
    let center = Rect::from_min_max(pos2(x0, body.top()), pos2(x1, body.bottom()));
    island(ui, &look, center, "document", |ui| {
        if app.home {
            let icon = app.icon(&ctx);
            if let Some(action) = panels::home(ui, &look, icon, &app.settings) {
                app.home = false;
                app.guard(panels::pending_of(action));
            }
        } else {
            egui::ScrollArea::vertical()
                .id_salt("document-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    cells::show(ui, &mut app.wb, &look, &mut msgs);
                });
        }
    });

    ui.scope_builder(UiBuilder::new().max_rect(status), |ui| {
        panels::status(ui, &look, &app.wb, &app.settings, app.engine_lost);
    });

    overlays::show(&ctx, app, &look);
    toasts(ui, app, &look);
    if ndl {
        chrome::resize_edges(ui, full);
    }
    for msg in msgs {
        app.msg(msg);
    }
    for cmd in cmds {
        app.run(cmd);
    }
}

/// A rounded panel surface filling `rect`.
fn island(ui: &mut Ui, look: &Look, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        eng::island(look).show(ui, |ui| {
            let inner = vec2(rect.width() - 16.0, rect.height() - 12.0);
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            add(ui);
        });
    });
}

fn ribbon_tools(ui: &mut Ui, look: &Look, app: &App, cmds: &mut Vec<Cmd>) {
    let selected = app.wb.selected.is_some();
    let tool = |ui: &mut Ui, cmd: Cmd, enabled: bool, cmds: &mut Vec<Cmd>, active: bool| {
        let name = cmd.icon().unwrap_or("more");
        let text = short_label(cmd);
        let response = ui
            .add_enabled_ui(enabled, |ui| eng::ribbon_tool(ui, look, name, text, active))
            .inner;
        let hint = match cmd.shortcut_label(cfg!(target_os = "macos")) {
            Some(s) => format!("{}  {s}", cmd.translated()),
            None => cmd.translated().to_owned(),
        };
        if response.on_hover_text(hint).clicked() && enabled {
            cmds.push(cmd);
        }
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        eng::ribbon_group(ui, look, tr("Document"), |ui| {
            for cmd in [Cmd::NewBlank, Cmd::Open, Cmd::Save, Cmd::OpenSample] {
                tool(ui, cmd, true, cmds, false);
            }
        });
        eng::ribbon_group(ui, look, tr("Cells"), |ui| {
            tool(ui, Cmd::AddMath, true, cmds, false);
            tool(ui, Cmd::AddText, true, cmds, false);
            tool(ui, Cmd::Duplicate, selected, cmds, false);
            tool(ui, Cmd::DeleteCell, selected, cmds, false);
            tool(ui, Cmd::MoveUp, selected, cmds, false);
            tool(ui, Cmd::MoveDown, selected, cmds, false);
        });
        eng::ribbon_group(ui, look, tr("View"), |ui| {
            tool(
                ui,
                Cmd::ToggleNavigator,
                true,
                cmds,
                app.settings.show_navigator,
            );
            tool(
                ui,
                Cmd::ShowUnits,
                true,
                cmds,
                app.settings.show_side_panel
                    && app.settings.side_panel == abaco_app::settings::SidePanel::Units,
            );
            tool(
                ui,
                Cmd::ShowDocumentSettings,
                true,
                cmds,
                app.settings.show_side_panel
                    && app.settings.side_panel == abaco_app::settings::SidePanel::Document,
            );
        });
    });
}

/// A command's label as it fits under a ribbon icon.
fn short_label(cmd: Cmd) -> &'static str {
    match cmd {
        Cmd::NewBlank => tr("New"),
        Cmd::Open => tr("Open"),
        Cmd::Save => tr("Save"),
        Cmd::OpenSample => tr("Example"),
        Cmd::AddMath => tr("Maths"),
        Cmd::AddText => tr("Text"),
        Cmd::Duplicate => tr("Duplicate"),
        Cmd::DeleteCell => tr("Delete"),
        Cmd::MoveUp => tr("Up"),
        Cmd::MoveDown => tr("Down"),
        Cmd::ToggleNavigator => tr("Outline"),
        Cmd::ShowUnits => tr("Units"),
        Cmd::ShowDocumentSettings => tr("Settings"),
        other => other.translated(),
    }
}

fn title_bar(
    ui: &mut Ui,
    bar: Rect,
    app: &mut App,
    look: &Look,
    mode: Mode,
    ndl: bool,
    cmds: &mut Vec<Cmd>,
) {
    let ctx = ui.ctx().clone();
    let controls_left = ndl && chrome::side() == nodalia_theme::titlebar::Side::Left;
    let controls_width = if ndl && !controls_left {
        chrome::CONTROLS_WIDTH
    } else {
        0.0
    };
    let leading_width = if controls_left {
        chrome::CONTROLS_WIDTH
    } else {
        chrome::leading_inset()
    };
    let name = app.dirty_title();
    let documents = [Document {
        name: &name,
        modified: app.wb.dirty,
    }];
    let icon = app.icon(&ctx);
    let title = TitleBar {
        app: abaco_app::NAME,
        icon: Some(icon),
        documents: &documents,
        active: if app.home { None } else { Some(0) },
        labels: TitleLabels {
            home: tr("Home"),
            file: tr("File"),
            save: tr("Save"),
            undo: tr("Undo"),
            redo: tr("Redo"),
            new_document: tr("New document"),
            close_document: tr("Close"),
            search: tr("Search commands"),
            menu: tr("Main menu"),
        },
        quick_access: &[TitleAction::Save],
        controls_width,
        leading_width,
    };
    let out = ui
        .scope_builder(UiBuilder::new().max_rect(bar), |ui| {
            eng::title_bar_with(ui, look, &title, |ui, rect| {
                if chrome::drags_window(ndl) {
                    chrome::drag_area(ui, rect.with_max_x(rect.right() - controls_width))
                } else {
                    ui.interact(rect, ui.scope_id().with("abaco_title"), Sense::hover())
                }
            })
        })
        .inner;
    if ndl {
        chrome::controls(
            ui,
            if controls_left { bar } else { out.controls },
            &look.palette,
            mode,
        );
    }
    let anchor = |x: f32| pos2(x, bar.bottom() - 4.0);
    let at = |ui: &Ui, fallback: f32| ui.ctx().pointer_interact_pos().map_or(fallback, |p| p.x);
    match out.action {
        Some(TitleAction::Home) => app.home = true,
        Some(TitleAction::File) => {
            let x = at(ui, bar.left()) - 20.0;
            ui.data_mut(|d| d.insert_temp(file_menu_id().with("at"), anchor(x)));
            egui::Popup::toggle_id(&ctx, file_menu_id());
        }
        Some(TitleAction::Menu) => {
            let x = at(ui, bar.right());
            ui.data_mut(|d| d.insert_temp(app_menu_id().with("at"), anchor(x)));
            egui::Popup::toggle_id(&ctx, app_menu_id());
        }
        Some(TitleAction::Save) => cmds.push(Cmd::Save),
        Some(TitleAction::NewDocument) | Some(TitleAction::CloseDocument(_)) => {
            cmds.push(Cmd::NewBlank);
        }
        Some(TitleAction::Document(_)) => app.home = false,
        Some(TitleAction::Search) => cmds.push(Cmd::CommandSearch),
        Some(TitleAction::Undo) | Some(TitleAction::Redo) | None => {}
    }
    title_menus(ui, look, cmds);
}

/// The File menu and the hamburger menu, opened from the title bar.
fn title_menus(ui: &mut Ui, look: &Look, cmds: &mut Vec<Cmd>) {
    let ctx = ui.ctx().clone();
    let at = |id: Id| {
        ctx.data(|d| d.get_temp::<egui::Pos2>(id.with("at")))
            .unwrap_or_default()
    };
    let popup = |id: Id| {
        menu::popup(
            egui::Popup::new(id, ctx.clone(), at(id), ui.layer_id())
                .kind(egui::PopupKind::Menu)
                .open_memory(None),
            look,
        )
    };
    let mac = cfg!(target_os = "macos");
    let item = |ui: &mut Ui, cmd: Cmd, cmds: &mut Vec<Cmd>| {
        let shortcut = cmd.shortcut_label(mac);
        let mut row = menu::Item::new(cmd.translated()).gutter();
        if let Some(icon) = cmd.icon() {
            row = row.icon(icon);
        }
        if let Some(s) = &shortcut {
            row = row.shortcut(s);
        }
        if menu::item(ui, look, &row).clicked() {
            cmds.push(cmd);
            ui.close();
        }
    };
    popup(file_menu_id()).show(|ui| {
        ui.set_min_width(menu::metric::MIN_WIDTH + 40.0);
        for cmd in [Cmd::NewBlank, Cmd::OpenSample, Cmd::Open] {
            item(ui, cmd, cmds);
        }
        menu::separator(ui, look);
        for cmd in [Cmd::Save, Cmd::SaveAs] {
            item(ui, cmd, cmds);
        }
        menu::separator(ui, look);
        item(ui, Cmd::Quit, cmds);
    });
    popup(app_menu_id()).show(|ui| {
        ui.set_min_width(menu::metric::MIN_WIDTH);
        for cmd in [Cmd::Preferences, Cmd::KeyboardShortcuts, Cmd::About] {
            item(ui, cmd, cmds);
        }
    });
}

fn toasts(ui: &mut Ui, app: &App, look: &Look) {
    let p = &look.palette;
    let full = ui.max_rect();
    let mut y = full.bottom() - layout::STATUS_BAR - 12.0;
    for toast in app.toasts.iter().rev() {
        let galley = ui.painter().layout(
            toast.text.clone(),
            eng::body(),
            color(p.text_primary),
            380.0,
        );
        let size = galley.size() + vec2(28.0, 18.0);
        let rect = Rect::from_min_size(pos2(full.right() - size.x - 16.0, y - size.y), size);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::unique(("abaco-toast", toast.text.len())),
        ));
        painter.rect_filled(rect, 12.0, color(p.surface_overlay));
        painter.rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(1.0, color(p.border_default)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + vec2(14.0, 9.0), galley, color(p.text_primary));
        y = rect.top() - 8.0;
    }
}
