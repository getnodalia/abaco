// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the script workspace shows and what the person did in it: the command window's history
//! and output, the editor's text, the variables and the figures. A reducer in the style of
//! [`crate::workbook`]: the window reads it, calls its methods, and sends the [`Job`]s they ask
//! for to the [`crate::script::engine::ScriptEngine`].

use std::path::PathBuf;

use abaco_script::plot::Figures;
use abaco_script::Event;

use super::engine::{Failure, Finished, Job, Reply, VarRow};

/// Most lines the command window keeps; older ones are dropped.
pub const MAX_LINES: usize = 5_000;
/// Most commands remembered for the up arrow.
pub const MAX_HISTORY: usize = 500;
/// Largest script file opened, in bytes.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// The extension of a script.
pub const EXTENSION: &str = "m";

/// What a line of the command window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A command the person typed.
    Input,
    /// What the interpreter printed.
    Output,
    Warning,
    Error,
    /// A note from the app (a run finished, was stopped).
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
}

/// What a run was started from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Command,
    Editor,
}

#[derive(Debug, Default)]
pub struct Session {
    /// The editor's text.
    pub editor: String,
    /// The file the editor was opened from or saved to.
    pub path: Option<PathBuf>,
    /// The editor has changes that are not on disk.
    pub dirty: bool,
    /// What the person is typing in the command window.
    pub input: String,
    pub lines: Vec<Line>,
    history: Vec<String>,
    /// Where the up arrow is in the history (`None`: not browsing).
    cursor: Option<usize>,
    /// The line being typed when the person started browsing the history.
    draft: String,
    pub vars: Vec<VarRow>,
    pub selected_var: Option<String>,
    /// The text of the selected variable.
    pub inspected: String,
    pub figures: Figures,
    /// The figure shown in the figure panel.
    pub figure: usize,
    /// A run is going.
    pub running: bool,
    next_id: u64,
    running_id: Option<u64>,
    running_from: Option<Source>,
    /// How long the last run took, in milliseconds.
    pub last_millis: Option<u64>,
    /// The editor line the last failure was on.
    pub error_line: Option<usize>,
    /// A new `figure` appeared: the window shows the figure panel once.
    pub figure_appeared: bool,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// The editor's file name, or `None` for a script that was never saved.
    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// Replaces the editor with a script that was opened.
    pub fn open(&mut self, path: PathBuf, text: String) {
        self.editor = text;
        self.path = Some(path);
        self.dirty = false;
        self.error_line = None;
    }

    /// A new, empty script.
    pub fn new_script(&mut self) {
        self.editor.clear();
        self.path = None;
        self.dirty = false;
        self.error_line = None;
    }

    /// The editor was saved to `path`.
    pub fn saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.dirty = false;
    }

    pub fn push(&mut self, kind: Kind, text: impl Into<String>) {
        self.lines.push(Line {
            kind,
            text: text.into(),
        });
        if self.lines.len() > MAX_LINES {
            let drop = self.lines.len() - MAX_LINES;
            self.lines.drain(..drop);
        }
    }

    pub fn clear_window(&mut self) {
        self.lines.clear();
    }

    // --- Running ------------------------------------------------------------------------------

    fn start(&mut self, source: String, from: Source) -> Option<Job> {
        if self.running || source.trim().is_empty() {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.running = true;
        self.running_id = Some(id);
        self.running_from = Some(from);
        self.error_line = None;
        Some(Job::Run { id, source })
    }

    /// Enter in the command window: run what was typed.
    pub fn submit(&mut self) -> Option<Job> {
        let line = std::mem::take(&mut self.input);
        self.cursor = None;
        self.draft.clear();
        let trimmed = line.trim();
        if trimmed.is_empty() || self.running {
            self.input = line;
            return None;
        }
        self.push(Kind::Input, format!(">> {trimmed}"));
        if self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_owned());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.start(trimmed.to_owned(), Source::Command)
    }

    /// Run the whole editor.
    pub fn run_editor(&mut self) -> Option<Job> {
        let source = self.editor.clone();
        let name = self.file_name().unwrap_or_else(|| "script".to_owned());
        let job = self.start(source, Source::Editor)?;
        self.push(Kind::Info, format!("▶ {name}"));
        Some(job)
    }

    /// Run `text` (a selection of the editor).
    pub fn run_text(&mut self, text: &str) -> Option<Job> {
        let job = self.start(text.to_owned(), Source::Editor)?;
        self.push(Kind::Info, "▶");
        Some(job)
    }

    /// The up arrow: the previous command.
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.cursor {
            None => {
                self.draft = std::mem::take(&mut self.input);
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.cursor = Some(next);
        self.input = self.history.get(next).cloned().unwrap_or_default();
    }

    /// The down arrow: the next command, then what was being typed.
    pub fn history_down(&mut self) {
        let Some(i) = self.cursor else { return };
        if i + 1 < self.history.len() {
            self.cursor = Some(i + 1);
            self.input = self.history.get(i + 1).cloned().unwrap_or_default();
        } else {
            self.cursor = None;
            self.input = std::mem::take(&mut self.draft);
        }
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    // --- Workspace ----------------------------------------------------------------------------

    pub fn select_var(&mut self, name: &str) -> Job {
        self.selected_var = Some(name.to_owned());
        Job::Inspect(name.to_owned())
    }

    // --- Replies ------------------------------------------------------------------------------

    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Finished(done) => self.finished(*done),
            Reply::Workspace(vars) => self.set_vars(vars),
            Reply::Inspected { name, text } => {
                if self.selected_var.as_deref() == Some(&name) {
                    self.inspected = text;
                }
            }
            Reply::Figures(figures) => self.set_figures(figures),
        }
    }

    fn set_vars(&mut self, vars: Vec<VarRow>) {
        if let Some(sel) = &self.selected_var {
            if !vars.iter().any(|v| &v.name == sel) {
                self.selected_var = None;
                self.inspected.clear();
            }
        }
        self.vars = vars;
    }

    fn set_figures(&mut self, figures: Figures) {
        if figures.list.len() > self.figures.list.len() {
            self.figure_appeared = true;
        }
        // Show the current figure when the one on show was closed.
        let on_show_closed = !figures.list.iter().any(|(n, _)| *n == self.figure);
        let new_current = figures.current != 0 && figures.current != self.figures.current;
        if on_show_closed || new_current {
            self.figure = figures.current;
        }
        self.figures = figures;
    }

    fn finished(&mut self, done: Finished) {
        if self.running_id != Some(done.id) {
            return;
        }
        self.running = false;
        self.running_id = None;
        let from = self.running_from.take();
        for event in done.events {
            match event {
                Event::Text(t) => {
                    let t = t.trim_end_matches('\n');
                    if !t.is_empty() {
                        self.push(Kind::Output, t);
                    }
                }
                Event::Warning(w) => self.push(Kind::Warning, format!("Warning: {w}")),
                Event::ClearScreen => self.lines.clear(),
                Event::Figure(_) => {}
            }
        }
        self.last_millis = Some(done.millis);
        if let Some(Failure { message, line }) = done.failure {
            if done.stopped {
                self.push(Kind::Info, "■");
            } else {
                self.push(Kind::Error, message);
                if from == Some(Source::Editor) {
                    self.error_line = line;
                }
            }
        }
        self.set_vars(done.vars);
        if let Some(figures) = done.figures {
            self.set_figures(figures);
        }
    }

    /// The jobs to refresh the text of the selected variable after a run.
    pub fn refresh_selection(&self) -> Option<Job> {
        self.selected_var.clone().map(Job::Inspect)
    }
}

// --- Files and selections ---------------------------------------------------------------------

/// Why a script could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Larger than [`MAX_FILE_BYTES`].
    TooLarge,
    /// Not UTF-8 text.
    NotText,
    /// The system said no.
    Io(String),
}

/// Reads a script, refusing files that are too large or not text.
pub fn read_script(path: &std::path::Path) -> Result<String, ReadError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| ReadError::Io(e.to_string()))?;
    let mut bytes = Vec::new();
    // One byte more than the limit tells a file that fits from one that does not, without
    // trusting its reported size.
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ReadError::Io(e.to_string()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ReadError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| ReadError::NotText)
}

/// Writes a script next to its destination first and renames it, so a failure half way leaves
/// the old file untouched.
pub fn write_script(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "script.m".to_owned());
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    let result = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `path` with the script extension when it has none.
pub fn with_script_extension(path: &std::path::Path) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension(EXTENSION)
    }
}

/// The text between two cursor positions (in characters, in any order), or the whole line the
/// cursor is on when nothing is selected. What Ctrl+Enter runs.
pub fn selection_or_line(text: &str, a: usize, b: usize) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    let chars: Vec<char> = text.chars().collect();
    let hi = hi.min(chars.len());
    let lo = lo.min(hi);
    if lo != hi {
        return chars[lo..hi].iter().collect();
    }
    let start = chars[..lo]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |i| i + 1);
    let end = chars[lo..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |i| lo + i);
    chars[start..end].iter().collect()
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn a_selection_wins_over_the_line_and_either_order_works() {
        let t = "a = 1\nb = 2\nc = 3";
        assert_eq!(selection_or_line(t, 6, 11), "b = 2");
        assert_eq!(selection_or_line(t, 11, 6), "b = 2");
        assert_eq!(
            selection_or_line(t, 8, 8),
            "b = 2",
            "no selection: the line"
        );
        assert_eq!(selection_or_line(t, 0, 0), "a = 1");
        assert_eq!(
            selection_or_line(t, 17, 17),
            "c = 3",
            "the last line, no newline"
        );
        assert_eq!(
            selection_or_line(t, 99, 99),
            "c = 3",
            "past the end is the end"
        );
        assert_eq!(selection_or_line("", 0, 0), "");
        assert_eq!(
            selection_or_line("é = 1\nñ", 7, 7),
            "ñ",
            "characters, not bytes"
        );
    }

    #[test]
    fn scripts_are_written_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("abaco-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.m");
        write_script(&path, "x = 1;\n").unwrap();
        assert_eq!(read_script(&path).unwrap(), "x = 1;\n");
        write_script(&path, "y = 2;\n").unwrap();
        assert_eq!(read_script(&path).unwrap(), "y = 2;\n");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "no temporary file is left behind");
        std::fs::write(dir.join("bin.m"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(read_script(&dir.join("bin.m")), Err(ReadError::NotText));
        assert!(matches!(
            read_script(&dir.join("missing.m")),
            Err(ReadError::Io(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_without_an_extension_becomes_a_script() {
        assert_eq!(
            with_script_extension(std::path::Path::new("/t/run")),
            PathBuf::from("/t/run.m")
        );
        assert_eq!(
            with_script_extension(std::path::Path::new("/t/run.txt")),
            PathBuf::from("/t/run.txt")
        );
    }
}
