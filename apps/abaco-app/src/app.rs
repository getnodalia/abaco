// SPDX-License-Identifier: MIT OR Apache-2.0
//! The application: its state, the commands, background work and the frame loop.
//!
//! Nothing here evaluates a formula or touches a file on the window's thread. Edits become
//! [`Request`]s for the engine's worker; the file dialogs run on threads of their own and
//! answer through a channel.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use abaco_app::commands::Cmd;
use abaco_app::engine::{with_extension, Engine, Notice, Request};
use abaco_app::i18n::{self, tr, trf, Lang};
use abaco_app::settings::{Appearance, Settings, SidePanel};
use abaco_app::workbook::{Msg, Workbook};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::{self, Appearance as NdlAppearance};
use nodalia_theme::{Accent, Mode};

use crate::panels::UnitsState;
use crate::{chrome, ui};

/// What waits for the person to decide about unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    NewBlank,
    Sample,
    OpenDialog,
    Open(PathBuf),
    Quit,
}

/// What covers the document.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    None,
    Search(String),
    Preferences,
    Shortcuts(String),
    About,
    Confirm(Pending),
}

pub struct Toast {
    pub text: String,
    pub until: f64,
}

/// A file dialog's answer.
enum Dialog {
    Open(Option<PathBuf>),
    SaveAs(Option<PathBuf>),
}

pub struct App {
    pub settings: Settings,
    pub wb: Workbook,
    engine: Option<Engine>,
    /// The worker thread is gone: edits are not evaluated any more.
    pub engine_lost: bool,
    pub overlay: Overlay,
    /// Home (start cards and recent documents) covers the document.
    pub home: bool,
    pub toasts: Vec<Toast>,
    pub units: UnitsState,
    tx: Sender<Dialog>,
    rx: Receiver<Dialog>,
    dialog_open: bool,
    /// What to do once the save that was asked for finishes.
    after_save: Option<Pending>,
    quit_ok: bool,
    applied: Option<(Appearance, bool, bool)>,
    decorated: Option<bool>,
    #[cfg(target_os = "linux")]
    menu: Option<crate::linux_menu::GlobalMenu>,
    ctx: egui::Context,
    icon: Option<egui::TextureHandle>,
}

pub fn mode_for(dark: bool, high_contrast: bool) -> Mode {
    match (dark, high_contrast) {
        (false, false) => Mode::Light,
        (true, false) => Mode::Dark,
        (false, true) => Mode::LightHc,
        (true, true) => Mode::DarkHc,
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings, open: Vec<PathBuf>) -> App {
        i18n::set(Lang::from_setting(&settings.language));
        let ctx = cc.egui_ctx.clone();
        eng::install_fonts(&ctx);
        let (tx, rx) = channel();
        let wake = ctx.clone();
        let engine = Engine::spawn(move || wake.request_repaint()).ok();
        let mut app = App {
            wb: Workbook::new(),
            engine_lost: engine.is_none(),
            engine,
            overlay: Overlay::None,
            home: false,
            toasts: Vec::new(),
            units: UnitsState::default(),
            tx,
            rx,
            dialog_open: false,
            after_save: None,
            quit_ok: false,
            applied: None,
            decorated: None,
            #[cfg(target_os = "linux")]
            menu: crate::linux_menu::install(
                &ctx,
                std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()),
            ),
            ctx,
            icon: None,
            settings,
        };
        let ctx = app.ctx.clone();
        app.apply_appearance(&ctx);
        app.wb.digits = app.settings.significant_digits;
        app.start(open);
        app
    }

    /// The first document: a file named on the command line, the one left open last time, the
    /// sample on the very first launch, else a blank page.
    fn start(&mut self, open: Vec<PathBuf>) {
        let given = open.into_iter().find(|p| p.is_file());
        let last = self.settings.last_file.clone().filter(|p| p.is_file());
        if let Some(path) = given.or(last) {
            self.msg(Msg::Open(path));
        } else if !self.settings.sample_shown {
            self.settings.sample_shown = true;
            self.save_settings();
            self.msg(Msg::NewSample);
        } else {
            self.msg(Msg::NewBlank);
        }
    }

    pub fn ndl_controls(&self) -> bool {
        chrome::NDL_CONTROLS_POSSIBLE && !self.settings.native_window_controls
    }

    pub fn dark(&self, ctx: &egui::Context) -> bool {
        match self.settings.appearance {
            Appearance::System => ctx.theme() == egui::Theme::Dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// The colours to draw with, following the system theme when asked to.
    pub fn look(&self, ctx: &egui::Context) -> (Look, Mode) {
        let mode = mode_for(self.dark(ctx), self.settings.high_contrast);
        (Look::new(mode.palette_with(Accent::Abaco)), mode)
    }

    /// The icon as a texture, made once.
    pub fn icon(&mut self, ctx: &egui::Context) -> egui::TextureId {
        self.icon
            .get_or_insert_with(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [128, 128],
                    &abaco_app::icon::rgba(128),
                );
                ctx.load_texture("abaco-icon", image, egui::TextureOptions::LINEAR)
            })
            .id()
    }

    fn apply_appearance(&mut self, ctx: &egui::Context) {
        let key = (
            self.settings.appearance,
            self.settings.high_contrast,
            self.settings.reduced_motion,
        );
        if self.applied == Some(key) {
            return;
        }
        self.applied = Some(key);
        let appearance = |dark: bool| NdlAppearance {
            mode: mode_for(dark, self.settings.high_contrast),
            accent: Accent::Abaco,
            reduced_motion: self.settings.reduced_motion,
            ..NdlAppearance::default()
        };
        match self.settings.appearance {
            Appearance::System => for_egui::apply_system(ctx, &appearance(false)),
            Appearance::Light => for_egui::apply(ctx, &appearance(false)),
            Appearance::Dark => for_egui::apply(ctx, &appearance(true)),
        }
        eng::apply_text_styles(ctx);
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let now = self.ctx.input(|i| i.time);
        self.toasts.push(Toast {
            text: text.into(),
            until: now + 4.0,
        });
        self.ctx.request_repaint();
    }

    pub fn save_settings(&mut self) {
        if self.settings.save().is_err() {
            self.toast(tr("Could not save the preferences."));
        }
    }

    // --- The engine ---------------------------------------------------------------------------

    fn send(&mut self, requests: Vec<Request>) {
        for request in requests {
            let sent = match self.engine.as_mut() {
                Some(engine) => engine.send(request).is_ok(),
                None => false,
            };
            if !sent && !self.engine_lost {
                self.engine_lost = true;
                self.toast(tr(
                    "The calculation engine stopped. Save your work and restart Ábaco.",
                ));
            }
        }
    }

    /// Applies what the person did and asks the engine for what follows.
    pub fn msg(&mut self, msg: Msg) {
        let requests = self.wb.update(msg);
        self.send(requests);
    }

    fn poll(&mut self) {
        let replies = self.engine.as_mut().map(Engine::poll).unwrap_or_default();
        for reply in replies {
            self.wb.apply(reply);
        }
        for notice in std::mem::take(&mut self.wb.notices) {
            self.notice(notice);
        }
        while let Ok(done) = self.rx.try_recv() {
            self.dialog_open = false;
            match done {
                Dialog::Open(Some(path)) => {
                    self.home = false;
                    self.msg(Msg::Open(path));
                }
                Dialog::SaveAs(Some(path)) => self.msg(Msg::Save(Some(with_extension(&path)))),
                Dialog::Open(None) | Dialog::SaveAs(None) => self.after_save = None,
            }
        }
    }

    fn notice(&mut self, notice: Notice) {
        let name = |p: &Path| {
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        };
        match notice {
            Notice::Opened(path) => {
                self.settings.remember(&path);
                self.save_settings();
                self.home = false;
            }
            Notice::Saved(path) => {
                self.settings.remember(&path);
                self.save_settings();
                self.toast(trf("Saved {}", &[&name(&path)]));
                if let Some(next) = self.after_save.take() {
                    self.proceed(next);
                }
            }
            Notice::OpenFailed { path, message } => {
                self.settings.forget(&path);
                self.save_settings();
                self.toast(trf("Could not open {}: {}", &[&name(&path), &message]));
            }
            Notice::SaveFailed { path, message } => {
                self.after_save = None;
                self.toast(trf("Could not save {}: {}", &[&name(&path), &message]));
            }
            Notice::NeedsPath => self.pick_save(),
            Notice::Rejected(message) => self.toast(message),
            Notice::Internal => {
                self.toast(tr(
                    "Something went wrong inside Ábaco. Your document was kept.",
                ));
            }
        }
    }

    // --- Files --------------------------------------------------------------------------------

    fn pick_open(&mut self) {
        if self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let title = tr("Open a document");
        let filter = tr("Ábaco documents");
        let spawned = std::thread::Builder::new()
            .name("open-dialog".into())
            .spawn(move || {
                let picked = pollster::block_on(
                    rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .add_filter(filter, &[abaco_app::EXTENSION])
                        .pick_file(),
                )
                .map(|f| f.path().to_path_buf());
                let _ = tx.send(Dialog::Open(picked));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.dialog_open = false;
            self.toast(tr("Could not open the file dialog."));
        }
    }

    fn pick_save(&mut self) {
        if self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let title = tr("Save the document");
        let filter = tr("Ábaco documents");
        let start = self
            .wb
            .file_name()
            .unwrap_or_else(|| format!("{}.{}", tr("document"), abaco_app::EXTENSION));
        let spawned = std::thread::Builder::new()
            .name("save-dialog".into())
            .spawn(move || {
                let picked = pollster::block_on(
                    rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .set_file_name(start)
                        .add_filter(filter, &[abaco_app::EXTENSION])
                        .save_file(),
                )
                .map(|f| f.path().to_path_buf());
                let _ = tx.send(Dialog::SaveAs(picked));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.dialog_open = false;
            self.after_save = None;
            self.toast(tr("Could not open the file dialog."));
        }
    }

    fn save(&mut self) {
        if self.wb.path.is_some() {
            self.msg(Msg::Save(None));
        } else {
            self.pick_save();
        }
    }

    /// Asks about unsaved changes before `pending` replaces the document.
    pub fn guard(&mut self, pending: Pending) {
        if self.wb.dirty {
            self.overlay = Overlay::Confirm(pending);
        } else {
            self.proceed(pending);
        }
    }

    /// Carries out what [`App::guard`] held back (or let through).
    pub fn proceed(&mut self, pending: Pending) {
        match pending {
            Pending::NewBlank => {
                self.home = false;
                self.msg(Msg::NewBlank);
            }
            Pending::Sample => {
                self.home = false;
                self.msg(Msg::NewSample);
            }
            Pending::OpenDialog => self.pick_open(),
            Pending::Open(path) => {
                self.home = false;
                self.msg(Msg::Open(path));
            }
            Pending::Quit => {
                self.quit_ok = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// "Save" in the unsaved-changes question: save, then carry on.
    pub fn save_then(&mut self, pending: Pending) {
        self.after_save = Some(pending);
        self.save();
    }

    // --- Commands -----------------------------------------------------------------------------

    pub fn run(&mut self, cmd: Cmd) {
        let selected = self.wb.selected;
        match cmd {
            Cmd::NewBlank => self.guard(Pending::NewBlank),
            Cmd::OpenSample => self.guard(Pending::Sample),
            Cmd::Open => self.guard(Pending::OpenDialog),
            Cmd::Save => self.save(),
            Cmd::SaveAs => self.pick_save(),
            Cmd::Quit => self.guard(Pending::Quit),
            Cmd::AddMath => {
                self.home = false;
                self.msg(Msg::AddMath { after: selected });
            }
            Cmd::AddText => {
                self.home = false;
                self.msg(Msg::AddText { after: selected });
            }
            Cmd::EvaluateAndAdd => {
                self.home = false;
                self.msg(Msg::EvaluateAndAdd);
            }
            Cmd::Duplicate => {
                if let Some(id) = selected {
                    self.msg(Msg::Duplicate(id));
                }
            }
            Cmd::DeleteCell => {
                if let Some(id) = selected {
                    self.msg(Msg::Remove(id));
                }
            }
            Cmd::MoveUp => {
                if let Some(id) = selected {
                    self.msg(Msg::MoveUp(id));
                }
            }
            Cmd::MoveDown => {
                if let Some(id) = selected {
                    self.msg(Msg::MoveDown(id));
                }
            }
            Cmd::ToggleNavigator => {
                self.settings.show_navigator = !self.settings.show_navigator;
                self.save_settings();
            }
            Cmd::ToggleSidePanel => {
                self.settings.show_side_panel = !self.settings.show_side_panel;
                self.save_settings();
            }
            Cmd::ShowUnits => self.show_panel(SidePanel::Units),
            Cmd::ShowDocumentSettings => self.show_panel(SidePanel::Document),
            Cmd::CommandSearch => self.overlay = Overlay::Search(String::new()),
            Cmd::Preferences => self.overlay = Overlay::Preferences,
            Cmd::KeyboardShortcuts => self.overlay = Overlay::Shortcuts(String::new()),
            Cmd::About => self.overlay = Overlay::About,
        }
    }

    fn show_panel(&mut self, panel: SidePanel) {
        self.settings.side_panel = panel;
        self.settings.show_side_panel = true;
        self.save_settings();
    }

    /// Runs the commands whose shortcut was pressed this frame.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.overlay != Overlay::None {
            return;
        }
        // Most specific first: egui matches a shortcut even when Shift or Alt are also held.
        let mut table: Vec<(Cmd, egui::KeyboardShortcut)> = Cmd::ALL
            .iter()
            .filter_map(|c| c.shortcut().map(|s| (*c, s)))
            .collect();
        table.sort_by_key(|(_, s)| {
            std::cmp::Reverse(u8::from(s.modifiers.shift) + u8::from(s.modifiers.alt))
        });
        let mut commands = Vec::new();
        ctx.input_mut(|i| {
            for (cmd, s) in &table {
                if i.consume_shortcut(s) {
                    commands.push(*cmd);
                }
            }
        });
        for cmd in commands {
            self.run(cmd);
        }
    }

    pub fn dirty_title(&self) -> String {
        self.wb
            .file_name()
            .unwrap_or_else(|| tr("Untitled").to_owned())
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_appearance(&ctx);
        self.poll();

        let decorated = !self.ndl_controls();
        if self.decorated != Some(decorated) {
            if self.decorated.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(decorated));
            }
            self.decorated = Some(decorated);
        }

        // Closing the window asks about unsaved work first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok && self.wb.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.overlay = Overlay::Confirm(Pending::Quit);
        }

        self.shortcuts(&ctx);
        ui::show(ui, self);

        #[cfg(target_os = "linux")]
        if let Some(menu) = &self.menu {
            for id in menu.take() {
                if let Some(cmd) = Cmd::from_id(id) {
                    self.run(cmd);
                }
            }
        }

        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| t.until > now);
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.window_fill.to_normalized_gamma_f32()
    }
}
