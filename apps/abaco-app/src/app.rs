// SPDX-License-Identifier: MIT OR Apache-2.0
//! The application: its state, the commands, background work and the frame loop.
//!
//! Nothing here evaluates a formula or touches a file on the window's thread. Edits become
//! [`Request`]s for the engine's worker; the file dialogs run on threads of their own and
//! answer through a channel.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use abaco_app::blocks::engine::{BlocksEngine, Job as BlocksJob};
use abaco_app::blocks::session::{self as blocks, Session as BlocksSession};
use abaco_app::commands::{Category, Cmd};
use abaco_app::engine::{with_extension, Engine, Notice, Request};
use abaco_app::i18n::{self, tr, trf, Lang};
use abaco_app::script::engine::{Job, ScriptEngine};
use abaco_app::script::session::{self as script, ReadError, Session};
use abaco_app::settings::{Appearance, Settings, SidePanel, Workspace};
use abaco_app::workbook::{Msg, Workbook};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::{self, Appearance as NdlAppearance};
use nodalia_theme::{Accent, Mode};

use crate::blocks_ui::BlocksView;
use crate::figure::FigureView;
use crate::panels::UnitsState;
use crate::{chrome, ui};

/// What waits for the person to decide about unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    NewBlank,
    Sample,
    OpenDialog,
    Open(PathBuf),
    NewScript,
    OpenScriptDialog,
    NewBlocks,
    OpenBlocksDialog,
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
    /// A script was picked and read (`None`: the dialog was cancelled).
    ScriptOpened(Option<Result<(PathBuf, String), ReadError>>),
    ScriptSaveAs(Option<PathBuf>),
    /// A script was written; `text` is what went to disk.
    ScriptSaved {
        path: PathBuf,
        text: String,
        result: Result<(), String>,
    },
    /// A diagram was picked and read (`None`: the dialog was cancelled).
    BlocksOpened(Option<Result<(PathBuf, blocks::Loaded), String>>),
    BlocksExportAs(Option<PathBuf>),
    /// A Simulink file was written.
    BlocksExported {
        path: PathBuf,
        result: Result<(), String>,
    },
    BlocksSaveAs(Option<PathBuf>),
    /// A diagram was written; `text` is what went to disk.
    BlocksSaved {
        path: PathBuf,
        text: String,
        result: Result<(), String>,
    },
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
    /// The script workspace: editor, command window, variables, figures.
    pub script: Session,
    script_engine: Option<ScriptEngine>,
    /// The interpreter's thread is gone: scripts do not run any more.
    pub script_lost: bool,
    pub figure_view: FigureView,
    /// The block-diagram workspace: the diagram, its selection and its last run.
    pub blocks: BlocksSession,
    blocks_engine: Option<BlocksEngine>,
    /// The simulation thread is gone: diagrams do not run any more.
    pub blocks_lost: bool,
    pub blocks_view: BlocksView,
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
        let wake = ctx.clone();
        let script_engine = ScriptEngine::spawn(move || wake.request_repaint()).ok();
        let wake = ctx.clone();
        let blocks_engine = BlocksEngine::spawn(move || wake.request_repaint()).ok();
        let mut app = App {
            wb: Workbook::new(),
            engine_lost: engine.is_none(),
            engine,
            overlay: Overlay::None,
            home: false,
            toasts: Vec::new(),
            units: UnitsState::default(),
            script: Session::new(),
            script_lost: script_engine.is_none(),
            script_engine,
            figure_view: FigureView::default(),
            blocks: BlocksSession::new(),
            blocks_lost: blocks_engine.is_none(),
            blocks_engine,
            blocks_view: BlocksView::default(),
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
        let (scripts, documents): (Vec<PathBuf>, Vec<PathBuf>) = open.into_iter().partition(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(script::EXTENSION))
        });
        if let Some(path) = scripts.into_iter().find(|p| p.is_file()) {
            match script::read_script(&path) {
                Ok(text) => {
                    self.script.open(path, text);
                    self.set_workspace(Workspace::Script);
                }
                Err(e) => self.toast(read_error_text(&e)),
            }
        }
        let given = documents.into_iter().find(|p| p.is_file());
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
                // A missing icon draws as one transparent pixel rather than failing.
                let (w, h, rgba) = abaco_app::icon::rgba(abaco_app::icon::SMALL)
                    .unwrap_or_else(|| (1, 1, vec![0; 4]));
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
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
        self.poll_script();
        self.poll_blocks();
        while let Ok(done) = self.rx.try_recv() {
            self.dialog_open = false;
            match done {
                Dialog::Open(Some(path)) => {
                    self.home = false;
                    self.set_workspace(Workspace::Document);
                    self.msg(Msg::Open(path));
                }
                Dialog::SaveAs(Some(path)) => self.msg(Msg::Save(Some(with_extension(&path)))),
                Dialog::Open(None) | Dialog::SaveAs(None) | Dialog::ScriptSaveAs(None) => {
                    self.after_save = None;
                }
                Dialog::ScriptOpened(None) => {}
                Dialog::ScriptOpened(Some(Ok((path, text)))) => {
                    self.script.open(path, text);
                    self.home = false;
                    self.set_workspace(Workspace::Script);
                }
                Dialog::ScriptOpened(Some(Err(e))) => self.toast(read_error_text(&e)),
                Dialog::ScriptSaveAs(Some(path)) => {
                    self.write_script(script::with_script_extension(&path))
                }
                Dialog::ScriptSaved { path, text, result } => self.script_saved(path, text, result),
                Dialog::BlocksOpened(None) => {}
                Dialog::BlocksOpened(Some(Ok((path, loaded)))) => self.open_blocks(path, loaded),
                Dialog::BlocksExportAs(Some(path)) => {
                    self.export_simulink(blocks::with_simulink_extension(&path))
                }
                Dialog::BlocksExportAs(None) => {}
                Dialog::BlocksExported { path, result } => self.simulink_exported(path, result),
                Dialog::BlocksOpened(Some(Err(message))) => {
                    self.toast(trf("Could not open the diagram: {}", &[&message]));
                }
                Dialog::BlocksSaveAs(Some(path)) => {
                    self.write_blocks(blocks::with_extension(&path))
                }
                Dialog::BlocksSaveAs(None) => self.after_save = None,
                Dialog::BlocksSaved { path, text, result } => self.blocks_saved(path, text, result),
            }
        }
    }

    // --- Scripts ------------------------------------------------------------------------------

    fn poll_script(&mut self) {
        let replies = self
            .script_engine
            .as_mut()
            .map(ScriptEngine::poll)
            .unwrap_or_default();
        if replies.is_empty() {
            return;
        }
        for reply in replies {
            self.script.apply(reply);
        }
        if let Some(job) = self.script.refresh_selection() {
            self.script_send(job);
        }
        if std::mem::take(&mut self.script.figure_appeared) && !self.settings.show_figures_panel {
            self.settings.show_figures_panel = true;
            self.save_settings();
        }
    }

    /// Hands a job to the interpreter; when it is gone, says so and ends the run.
    pub fn script_send(&mut self, job: Job) {
        let sent = match self.script_engine.as_mut() {
            Some(engine) => engine.send(job).is_ok(),
            None => false,
        };
        if !sent {
            self.script.running = false;
            if !self.script_lost {
                self.script_lost = true;
                self.toast(tr(
                    "The script interpreter stopped. Save your work and restart Ábaco.",
                ));
            }
        }
    }

    fn script_saved(&mut self, path: PathBuf, text: String, result: Result<(), String>) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match result {
            Ok(()) => {
                // Typing went on while the file was written: what is on disk is older.
                self.script.dirty = self.script.editor != text;
                self.script.path = Some(path);
                self.toast(trf("Saved {}", &[&name]));
                if let Some(next) = self.after_save.take() {
                    self.guard(next);
                }
            }
            Err(message) => {
                self.after_save = None;
                self.toast(trf("Could not save {}: {}", &[&name, &message]));
            }
        }
    }

    fn write_script(&mut self, path: PathBuf) {
        let text = self.script.editor.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("save-script".into())
            .spawn(move || {
                let result = script::write_script(&path, &text).map_err(|e| e.to_string());
                let _ = tx.send(Dialog::ScriptSaved { path, text, result });
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.after_save = None;
            self.toast(tr("Could not save the script."));
        }
    }

    fn save_script(&mut self) {
        match self.script.path.clone() {
            Some(path) => self.write_script(path),
            None => self.pick_save_script(),
        }
    }

    fn pick_open_script(&mut self) {
        if self.dialog_open {
            return;
        }
        let title = tr("Open a script");
        let filter = tr("Scripts");
        self.dialog("open-script", move || {
            let picked = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .add_filter(filter, &[script::EXTENSION, "txt"])
                    .pick_file(),
            )
            .map(|f| f.path().to_path_buf());
            Dialog::ScriptOpened(picked.map(|path| script::read_script(&path).map(|t| (path, t))))
        });
    }

    fn pick_save_script(&mut self) {
        if self.dialog_open {
            return;
        }
        let title = tr("Save the script");
        let filter = tr("Scripts");
        let start = self
            .script
            .file_name()
            .unwrap_or_else(|| format!("{}.{}", tr("script"), script::EXTENSION));
        self.dialog("save-script-dialog", move || {
            let picked = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .set_file_name(start)
                    .add_filter(filter, &[script::EXTENSION])
                    .save_file(),
            )
            .map(|f| f.path().to_path_buf());
            Dialog::ScriptSaveAs(picked)
        });
    }

    // --- Block diagrams -----------------------------------------------------------------------

    fn poll_blocks(&mut self) {
        let replies = self
            .blocks_engine
            .as_mut()
            .map(BlocksEngine::poll)
            .unwrap_or_default();
        for reply in replies {
            self.blocks.apply(reply);
        }
    }

    /// Hands a job to the simulator; when it is gone, says so and ends the run.
    pub fn blocks_send(&mut self, job: BlocksJob) {
        let sent = match self.blocks_engine.as_mut() {
            Some(engine) => engine.send(job).is_ok(),
            None => false,
        };
        if !sent {
            self.blocks.running = false;
            if !self.blocks_lost {
                self.blocks_lost = true;
                self.toast(tr(
                    "The simulator stopped. Save your work and restart Ábaco.",
                ));
            }
        }
    }

    fn open_blocks(&mut self, path: PathBuf, loaded: blocks::Loaded) {
        // A converted Simulink model has no Ábaco file yet: saving must not overwrite the original.
        let file = if loaded.imported {
            None
        } else {
            Some(path.clone())
        };
        match self.blocks.open(file, &loaded.text) {
            Ok(()) => {
                if loaded.imported {
                    self.blocks.dirty = true;
                    let name = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    let message = match loaded.notes.first() {
                        None => trf("Imported {}", &[&name]),
                        Some(first) => trf(
                            "Imported {} with {} notes: {}",
                            &[&name, &loaded.notes.len().to_string(), first],
                        ),
                    };
                    self.toast(message);
                }
                self.blocks_view.fit_next = true;
                self.home = false;
                self.set_workspace(Workspace::Blocks);
            }
            Err(message) => self.toast(trf("Could not open the diagram: {}", &[&message])),
        }
    }

    fn blocks_saved(&mut self, path: PathBuf, text: String, result: Result<(), String>) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match result {
            Ok(()) => {
                // Editing went on while the file was written: what is on disk is older.
                self.blocks.dirty = self.blocks.text().map_or(true, |now| now != text);
                self.blocks.path = Some(path);
                self.toast(trf("Saved {}", &[&name]));
                if let Some(next) = self.after_save.take() {
                    self.guard(next);
                }
            }
            Err(message) => {
                self.after_save = None;
                self.toast(trf("Could not save {}: {}", &[&name, &message]));
            }
        }
    }

    fn write_blocks(&mut self, path: PathBuf) {
        let text = match self.blocks.text() {
            Ok(text) => text,
            Err(message) => {
                self.after_save = None;
                self.toast(trf("Could not save the diagram: {}", &[&message]));
                return;
            }
        };
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("save-diagram".into())
            .spawn(move || {
                let result = blocks::write_diagram(&path, &text).map_err(|e| e.to_string());
                let _ = tx.send(Dialog::BlocksSaved { path, text, result });
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.after_save = None;
            self.toast(tr("Could not save the diagram."));
        }
    }

    fn export_simulink(&mut self, path: PathBuf) {
        let text = match self.blocks.text() {
            Ok(text) => text,
            Err(message) => {
                self.toast(trf("Could not export the diagram: {}", &[&message]));
                return;
            }
        };
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("export-simulink".into())
            .spawn(move || {
                let result = blocks::simulink_bytes(&text, &path).and_then(|bytes| {
                    blocks::write_diagram(&path, bytes).map_err(|e| e.to_string())
                });
                let _ = tx.send(Dialog::BlocksExported { path, result });
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.toast(tr("Could not export the diagram."));
        }
    }

    fn simulink_exported(&mut self, path: PathBuf, result: Result<(), String>) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match result {
            Ok(()) => self.toast(trf("Exported {}", &[&name])),
            Err(message) => self.toast(trf("Could not export {}: {}", &[&name, &message])),
        }
    }

    fn pick_export_simulink(&mut self) {
        if self.dialog_open {
            return;
        }
        let title = tr("Export as a Simulink model");
        let filter = tr("Simulink models");
        let start = self.blocks.file_name().map_or_else(
            || format!("{}.slx", tr("diagram")),
            |n| {
                std::path::Path::new(&n)
                    .with_extension("slx")
                    .to_string_lossy()
                    .into_owned()
            },
        );
        self.dialog("export-simulink-dialog", move || {
            let picked = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .set_file_name(start)
                    .add_filter(filter, &["slx", "mdl"])
                    .save_file(),
            )
            .map(|f| f.path().to_path_buf());
            Dialog::BlocksExportAs(picked)
        });
    }

    fn save_blocks(&mut self) {
        match self.blocks.path.clone() {
            Some(path) => self.write_blocks(path),
            None => self.pick_save_blocks(),
        }
    }

    fn pick_open_blocks(&mut self) {
        if self.dialog_open {
            return;
        }
        let title = tr("Open a diagram");
        let filter = tr("Block diagrams");
        self.dialog("open-diagram", move || {
            let picked = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .add_filter(filter, &[blocks::EXTENSION, "slx", "mdl"])
                    .pick_file(),
            )
            .map(|f| f.path().to_path_buf());
            Dialog::BlocksOpened(picked.map(|path| blocks::read_diagram(&path).map(|t| (path, t))))
        });
    }

    fn pick_save_blocks(&mut self) {
        if self.dialog_open {
            return;
        }
        let title = tr("Save the diagram");
        let filter = tr("Block diagrams");
        let start = self
            .blocks
            .file_name()
            .unwrap_or_else(|| format!("{}.{}", tr("diagram"), blocks::EXTENSION));
        self.dialog("save-diagram-dialog", move || {
            let picked = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .set_file_name(start)
                    .add_filter(filter, &[blocks::EXTENSION])
                    .save_file(),
            )
            .map(|f| f.path().to_path_buf());
            Dialog::BlocksSaveAs(picked)
        });
    }

    pub fn blocks_mode(&self) -> bool {
        self.settings.workspace == Workspace::Blocks
    }

    /// Runs a dialog on a thread of its own and answers through the channel.
    fn dialog(&mut self, name: &str, job: impl FnOnce() -> Dialog + Send + 'static) {
        self.dialog_open = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = tx.send(job());
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.dialog_open = false;
            self.after_save = None;
            self.toast(tr("Could not open the file dialog."));
        }
    }

    /// Switches between the maths document and the script workspace.
    pub fn set_workspace(&mut self, workspace: Workspace) {
        if self.settings.workspace != workspace {
            self.settings.workspace = workspace;
            self.save_settings();
        }
    }

    pub fn script_mode(&self) -> bool {
        self.settings.workspace == Workspace::Script
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
                    self.guard(next);
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
        if self.unsaved(&pending) {
            self.overlay = Overlay::Confirm(pending);
        } else {
            self.proceed(pending);
        }
    }

    /// What `pending` would throw away is not saved.
    fn unsaved(&self, pending: &Pending) -> bool {
        match pending {
            Pending::NewBlank | Pending::Sample | Pending::OpenDialog | Pending::Open(_) => {
                self.wb.dirty
            }
            Pending::NewScript | Pending::OpenScriptDialog => self.script.dirty,
            Pending::NewBlocks | Pending::OpenBlocksDialog => self.blocks.dirty,
            Pending::Quit => self.wb.dirty || self.script.dirty || self.blocks.dirty,
        }
    }

    /// Whether the unsaved work `pending` would lose is the script's (the document's comes
    /// first when both are).
    fn script_is_unsaved_for(&self, pending: &Pending) -> bool {
        match pending {
            Pending::NewScript | Pending::OpenScriptDialog => true,
            Pending::Quit => !self.wb.dirty && self.script.dirty,
            _ => false,
        }
    }

    /// Whether the unsaved work `pending` would lose is the diagram's (the document's and the
    /// script's come first).
    fn blocks_is_unsaved_for(&self, pending: &Pending) -> bool {
        match pending {
            Pending::NewBlocks | Pending::OpenBlocksDialog => true,
            Pending::Quit => !self.wb.dirty && !self.script.dirty && self.blocks.dirty,
            _ => false,
        }
    }

    /// The name of what is unsaved, for the question about it.
    pub fn unsaved_name(&self, pending: &Pending) -> String {
        if self.blocks_is_unsaved_for(pending) {
            self.blocks
                .file_name()
                .unwrap_or_else(|| tr("Untitled diagram").to_owned())
        } else if self.script_is_unsaved_for(pending) {
            self.script
                .file_name()
                .unwrap_or_else(|| tr("Untitled script").to_owned())
        } else {
            self.dirty_title_of_document()
        }
    }

    /// Carries out what [`App::guard`] held back (or let through).
    pub fn proceed(&mut self, pending: Pending) {
        match pending {
            Pending::NewBlank => {
                self.home = false;
                self.set_workspace(Workspace::Document);
                self.msg(Msg::NewBlank);
            }
            Pending::Sample => {
                self.home = false;
                self.set_workspace(Workspace::Document);
                self.msg(Msg::NewSample);
            }
            Pending::OpenDialog => self.pick_open(),
            Pending::Open(path) => {
                self.home = false;
                self.set_workspace(Workspace::Document);
                self.msg(Msg::Open(path));
            }
            Pending::NewScript => {
                self.script.new_script();
                self.home = false;
                self.set_workspace(Workspace::Script);
            }
            Pending::OpenScriptDialog => self.pick_open_script(),
            Pending::NewBlocks => {
                self.blocks.new_diagram();
                self.blocks_view.fit_next = true;
                self.home = false;
                self.set_workspace(Workspace::Blocks);
            }
            Pending::OpenBlocksDialog => self.pick_open_blocks(),
            Pending::Quit => {
                self.quit_ok = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// "Save" in the unsaved-changes question: save, then carry on.
    pub fn save_then(&mut self, pending: Pending) {
        let script = self.script_is_unsaved_for(&pending);
        let diagram = self.blocks_is_unsaved_for(&pending);
        self.after_save = Some(pending);
        if diagram {
            self.save_blocks();
        } else if script {
            self.save_script();
        } else {
            self.save();
        }
    }

    // --- Commands -----------------------------------------------------------------------------

    pub fn run(&mut self, cmd: Cmd) {
        let selected = self.wb.selected;
        let scripting = self.script_mode() && !self.home;
        let diagrams = self.blocks_mode() && !self.home;
        match cmd {
            Cmd::NewBlank if diagrams => self.guard(Pending::NewBlocks),
            Cmd::Open if diagrams => self.guard(Pending::OpenBlocksDialog),
            Cmd::Save if diagrams => self.save_blocks(),
            Cmd::SaveAs if diagrams => self.pick_save_blocks(),
            Cmd::NewBlank if scripting => self.guard(Pending::NewScript),
            Cmd::Open if scripting => self.guard(Pending::OpenScriptDialog),
            Cmd::Save if scripting => self.save_script(),
            Cmd::SaveAs if scripting => self.pick_save_script(),
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
            Cmd::ShowDocument => {
                self.set_workspace(Workspace::Document);
                self.home = false;
            }
            Cmd::ShowScript => {
                self.set_workspace(Workspace::Script);
                self.home = false;
            }
            Cmd::ShowBlocks => {
                self.set_workspace(Workspace::Blocks);
                self.home = false;
            }
            Cmd::RunBlocks => {
                self.set_workspace(Workspace::Blocks);
                self.home = false;
                if let Some(job) = self.blocks.run() {
                    self.blocks_send(job);
                }
            }
            Cmd::StopBlocks => {
                if let Some(engine) = &self.blocks_engine {
                    engine.stop();
                }
            }
            Cmd::DeleteBlock => self.blocks.delete_selected(),
            Cmd::ExportSimulink => self.pick_export_simulink(),
            Cmd::BlocksExample => {
                self.blocks.open_example();
                self.blocks_view.fit_next = true;
                self.set_workspace(Workspace::Blocks);
                self.home = false;
            }
            Cmd::ToggleWorkspacePanel => {
                self.settings.show_workspace_panel = !self.settings.show_workspace_panel;
                self.save_settings();
            }
            Cmd::ToggleFiguresPanel => {
                self.settings.show_figures_panel = !self.settings.show_figures_panel;
                self.save_settings();
            }
            Cmd::RunScript => {
                self.set_workspace(Workspace::Script);
                self.home = false;
                if let Some(job) = self.script.run_editor() {
                    self.script_send(job);
                }
            }
            Cmd::StopScript => {
                if let Some(engine) = &self.script_engine {
                    engine.stop();
                }
            }
            Cmd::ClearWorkspace => self.script_send(Job::ClearVariables),
            Cmd::ClearCommandWindow => self.script.clear_window(),
            Cmd::CloseFigures => self.script_send(Job::CloseFigures),
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
            .filter(|c| !(self.script_mode() && !self.home && c.category() == Category::Cells))
            .filter(|c| !(self.blocks_mode() && !self.home && c.category() == Category::Cells))
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

    fn dirty_title_of_document(&self) -> String {
        self.wb
            .file_name()
            .unwrap_or_else(|| tr("Untitled").to_owned())
    }

    /// The name the title bar shows: the document's, or the script's in the script workspace.
    pub fn dirty_title(&self) -> String {
        if self.blocks_mode() {
            self.blocks
                .file_name()
                .unwrap_or_else(|| tr("Untitled diagram").to_owned())
        } else if self.script_mode() {
            self.script
                .file_name()
                .unwrap_or_else(|| tr("Untitled script").to_owned())
        } else {
            self.dirty_title_of_document()
        }
    }

    /// The open document or script has changes that are not on disk.
    pub fn title_modified(&self) -> bool {
        if self.blocks_mode() {
            self.blocks.dirty
        } else if self.script_mode() {
            self.script.dirty
        } else {
            self.wb.dirty
        }
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
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_ok
            && (self.wb.dirty || self.script.dirty || self.blocks.dirty)
        {
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

/// What went wrong reading a script, in words.
fn read_error_text(error: &ReadError) -> String {
    match error {
        ReadError::TooLarge => tr("The file is too large to open as a script.").to_owned(),
        ReadError::NotText => tr("The file is not text.").to_owned(),
        ReadError::Io(message) => trf("Could not open the script: {}", &[message]),
    }
}
