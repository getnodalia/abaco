// SPDX-License-Identifier: MIT OR Apache-2.0
//! The maths document on a worker thread.
//!
//! The window never evaluates anything. It sends [`Request`]s (an edit, an open, a save) and
//! reads the [`Reply`] each batch of them produces: a [`Snapshot`] holding every cell's text and
//! result, already formatted, so drawing is a read. The worker takes every request that is
//! waiting, applies them in order and evaluates once, so typing faster than the engine answers
//! costs one evaluation, not one per key.
//!
//! [`Backend`] is the whole behaviour without the thread, so tests drive it directly.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use abaco_doc::{CellError, CellId, CellKind, DocError, Document, Settings};
use abaco_expr::{line_col, Value};
use nodalia_units::format_sig;

/// Longest text of a result shown on one line, in characters.
pub const MAX_TEXT_CHARS: usize = 600;
/// Most rows and columns of a matrix result that are drawn as a grid.
pub const MAX_GRID_ROWS: usize = 24;
pub const MAX_GRID_COLS: usize = 10;
/// Longest source line quoted under an error, in characters.
const MAX_QUOTE_CHARS: usize = 120;

/// What a new cell holds when a document is built from scratch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellSpec {
    Math(String),
    Text(String),
}

/// Something the window asks the engine to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    InsertMath {
        index: usize,
        source: String,
    },
    InsertText {
        index: usize,
        markdown: String,
    },
    SetSource {
        id: CellId,
        source: String,
    },
    SetText {
        id: CellId,
        markdown: String,
    },
    Remove {
        id: CellId,
    },
    /// Moves the cell so that it ends up at `index` among the cells.
    Move {
        id: CellId,
        index: usize,
    },
    /// A copy right after the cell.
    Duplicate {
        id: CellId,
    },
    SetDigits(usize),
    SetStrictOrder(bool),
    /// Replaces the document with these cells (a blank page, or the sample).
    New {
        cells: Vec<CellSpec>,
        digits: usize,
    },
    Open {
        path: PathBuf,
    },
    /// Saves to `path`, or to the document's own path when `None`.
    Save {
        path: Option<PathBuf>,
    },
}

/// Something the person should be told about.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Opened(PathBuf),
    Saved(PathBuf),
    OpenFailed {
        path: PathBuf,
        message: String,
    },
    SaveFailed {
        path: PathBuf,
        message: String,
    },
    /// A save was asked for a document that has no file yet.
    NeedsPath,
    /// An edit the document refused (too many cells, a cell too long...).
    Rejected(String),
    /// A bug: the worker caught a panic and kept the document.
    Internal,
}

/// A value, formatted for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    /// The whole value on one line (`98.1 N`, `[1, 2; 3, 4]`), cut when huge.
    pub text: String,
    /// A quantity as its number and its unit, so the window can set them apart.
    pub parts: Option<(String, String)>,
    /// A vector (one column) or matrix, as cell texts, when it is small enough to draw as a grid.
    pub grid: Option<Grid>,
    /// What the value measures (`force`), for a quantity.
    pub dimension: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// Rows and columns of the whole value.
    pub rows: usize,
    pub cols: usize,
    /// Row-major texts of the rows and columns that are shown (`shown_rows * shown_cols`).
    pub cells: Vec<String>,
    pub shown_rows: usize,
    pub shown_cols: usize,
}

/// Why a cell has no value, with where in its source.
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorView {
    /// `syntax`, `name`, `dimension`, `cycle`...
    pub code: &'static str,
    pub message: String,
    /// 1-based line and column of the problem, when it has a position.
    pub position: Option<(usize, usize)>,
    /// The byte range of the problem in the source that was evaluated.
    pub span: Option<(usize, usize)>,
    /// The source line with the problem, a caret column (0-based, in characters) and its width.
    pub quote: Option<Quote>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    pub line: String,
    pub caret: usize,
    pub width: usize,
}

/// What evaluating a math cell gave.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Not evaluated yet.
    Pending,
    Ok {
        value: Option<Shown>,
        /// Names the cell defines.
        defined: Vec<String>,
        warnings: Vec<String>,
    },
    Err {
        error: ErrorView,
        warnings: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Math { source: String, outcome: Outcome },
    Text { markdown: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellView {
    pub id: CellId,
    pub body: Body,
}

/// The document as the window reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Changes whenever the whole document is replaced (new, open), so the window drops what it
    /// was holding for the old one.
    pub epoch: u64,
    pub cells: Vec<CellView>,
    pub path: Option<PathBuf>,
    /// Edits since the last save or open.
    pub dirty: bool,
    pub digits: usize,
    pub strict_order: bool,
    pub errors: usize,
    /// Cells the last evaluation computed, and how long it took.
    pub evaluated: usize,
    pub elapsed: Duration,
}

impl Snapshot {
    pub fn empty() -> Snapshot {
        Snapshot {
            epoch: 0,
            cells: Vec::new(),
            path: None,
            dirty: false,
            digits: abaco_doc::DEFAULT_SIGNIFICANT_DIGITS,
            strict_order: false,
            errors: 0,
            evaluated: 0,
            elapsed: Duration::ZERO,
        }
    }
}

/// The answer to a batch of requests.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub snapshot: Snapshot,
    /// Cells the batch created, in order.
    pub created: Vec<CellId>,
    pub notices: Vec<Notice>,
    /// How many requests the batch held.
    pub handled: usize,
}

/// The document and what is known about its file, without a thread.
pub struct Backend {
    doc: Document,
    epoch: u64,
    revision: u64,
    saved: u64,
    path: Option<PathBuf>,
    evaluated: usize,
    elapsed: Duration,
}

impl Default for Backend {
    fn default() -> Backend {
        Backend::new()
    }
}

impl Backend {
    pub fn new() -> Backend {
        Backend {
            doc: Document::new(),
            epoch: 1,
            revision: 0,
            saved: 0,
            path: None,
            evaluated: 0,
            elapsed: Duration::ZERO,
        }
    }

    /// Applies `requests` in order, evaluates what changed once, and describes the result.
    pub fn handle(&mut self, requests: Vec<Request>) -> Reply {
        let handled = requests.len();
        let mut created = Vec::new();
        let mut notices = Vec::new();
        for request in requests {
            self.apply(request, &mut created, &mut notices);
        }
        self.settle();
        Reply {
            snapshot: self.snapshot(),
            created,
            notices,
            handled,
        }
    }

    fn settle(&mut self) {
        let started = Instant::now();
        let done = self.doc.evaluate();
        if !done.is_empty() {
            self.evaluated = done.len();
            self.elapsed = started.elapsed();
        }
    }

    fn touch(&mut self) {
        self.revision += 1;
    }

    fn replace(&mut self, doc: Document, path: Option<PathBuf>) {
        self.doc = doc;
        self.path = path;
        self.epoch += 1;
        self.revision += 1;
        self.saved = self.revision;
    }

    fn apply(&mut self, request: Request, created: &mut Vec<CellId>, notices: &mut Vec<Notice>) {
        let outcome: Result<(), DocError> = match request {
            Request::InsertMath { index, source } => {
                let at = index.min(self.doc.cells().len());
                self.doc
                    .insert(at, CellKind::Math { source })
                    .map(|id| created.push(id))
            }
            Request::InsertText { index, markdown } => {
                let at = index.min(self.doc.cells().len());
                self.doc
                    .insert(at, CellKind::Text { markdown })
                    .map(|id| created.push(id))
            }
            Request::SetSource { id, source } => {
                let same = matches!(
                    self.doc.cell(id).map(|c| &c.kind),
                    Some(CellKind::Math { source: old }) if *old == source
                );
                if same {
                    return;
                }
                self.doc.set_source(id, source)
            }
            Request::SetText { id, markdown } => {
                let same = matches!(
                    self.doc.cell(id).map(|c| &c.kind),
                    Some(CellKind::Text { markdown: old }) if *old == markdown
                );
                if same {
                    return;
                }
                self.doc.set_text(id, markdown)
            }
            Request::Remove { id } => self.doc.remove(id).map(|_| ()),
            Request::Move { id, index } => {
                let last = self.doc.cells().len().saturating_sub(1);
                self.doc.move_cell(id, index.min(last))
            }
            Request::Duplicate { id } => match (self.doc.cell(id), self.doc.position(id)) {
                (Some(cell), Some(pos)) => {
                    let kind = cell.kind.clone();
                    self.doc.insert(pos + 1, kind).map(|new| created.push(new))
                }
                _ => Err(DocError::UnknownCell(id)),
            },
            Request::SetDigits(digits) => {
                let s = Settings {
                    significant_digits: digits,
                    ..self.doc.settings()
                };
                self.doc.set_settings(s)
            }
            Request::SetStrictOrder(on) => {
                let s = Settings {
                    strict_order: on,
                    ..self.doc.settings()
                };
                self.doc.set_settings(s)
            }
            Request::New { cells, digits } => {
                self.new_document(cells, digits, notices);
                return;
            }
            Request::Open { path } => {
                match Document::open(&path) {
                    Ok(doc) => {
                        self.replace(doc, Some(path.clone()));
                        notices.push(Notice::Opened(path));
                    }
                    Err(e) => notices.push(Notice::OpenFailed {
                        path,
                        message: e.to_string(),
                    }),
                }
                return;
            }
            Request::Save { path } => {
                self.save(path, notices);
                return;
            }
        };
        match outcome {
            Ok(()) => self.touch(),
            // A request about a cell that is already gone (typing in a cell that was deleted a
            // moment ago) is not worth telling anyone.
            Err(DocError::UnknownCell(_)) => {}
            Err(e) => notices.push(Notice::Rejected(e.to_string())),
        }
    }

    fn new_document(&mut self, cells: Vec<CellSpec>, digits: usize, notices: &mut Vec<Notice>) {
        let mut doc = Document::new();
        let settings = Settings {
            significant_digits: digits,
            ..Settings::default()
        };
        if let Err(e) = doc.set_settings(settings) {
            notices.push(Notice::Rejected(e.to_string()));
        }
        for cell in cells {
            let added = match cell {
                CellSpec::Math(source) => doc.push_math(source),
                CellSpec::Text(markdown) => doc.push_text(markdown),
            };
            if let Err(e) = added {
                notices.push(Notice::Rejected(e.to_string()));
                break;
            }
        }
        self.replace(doc, None);
    }

    fn save(&mut self, path: Option<PathBuf>, notices: &mut Vec<Notice>) {
        let Some(target) = path.or_else(|| self.path.clone()) else {
            notices.push(Notice::NeedsPath);
            return;
        };
        let target = with_extension(&target);
        match self.doc.save(&target) {
            Ok(()) => {
                self.path = Some(target.clone());
                self.saved = self.revision;
                notices.push(Notice::Saved(target));
            }
            Err(e) => notices.push(Notice::SaveFailed {
                path: target,
                message: e.to_string(),
            }),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let settings = self.doc.settings();
        let digits = settings.significant_digits;
        let cells = self
            .doc
            .cells()
            .iter()
            .map(|cell| CellView {
                id: cell.id,
                body: match &cell.kind {
                    CellKind::Text { markdown } => Body::Text {
                        markdown: markdown.clone(),
                    },
                    CellKind::Math { source } => Body::Math {
                        source: source.clone(),
                        outcome: self.outcome(cell.id, source, digits),
                    },
                },
            })
            .collect();
        Snapshot {
            epoch: self.epoch,
            cells,
            path: self.path.clone(),
            dirty: self.revision != self.saved,
            digits,
            strict_order: settings.strict_order,
            errors: self.doc.error_count(),
            evaluated: self.evaluated,
            elapsed: self.elapsed,
        }
    }

    fn outcome(&self, id: CellId, source: &str, digits: usize) -> Outcome {
        let Some(result) = self.doc.result(id) else {
            return Outcome::Pending;
        };
        let warnings = result.warnings.clone();
        match &result.outcome {
            Ok(out) => Outcome::Ok {
                value: out.value.as_ref().map(|v| shown(v, digits)),
                defined: out.defined.clone(),
                warnings,
            },
            Err(e) => Outcome::Err {
                error: error_view(e, source),
                warnings,
            },
        }
    }
}

/// `path` with the `.abaco` extension when it has none.
pub fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension(crate::EXTENSION)
    }
}

fn truncate(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

/// A value as the window draws it.
pub fn shown(value: &Value, digits: usize) -> Shown {
    let text = value.display(digits);
    let mut parts = None;
    let mut dimension = None;
    if let Value::Quantity(q) = value {
        if let Some(unit) = q.display_unit() {
            let number = format_sig(q.display_value(), digits);
            let unit = unit.to_string();
            // Only split what the engine's own rendering is made of.
            if text == format!("{number} {unit}") {
                parts = Some((number, unit));
            }
        }
        dimension = Some(q.dimension().describe());
    }
    let grid = match value {
        Value::Vector(items) => grid_of(items.len(), 1, |r, _| items.get(r), digits),
        Value::Matrix(m) => grid_of(m.rows, m.cols, |r, c| m.get(r, c), digits),
        _ => None,
    };
    Shown {
        text: truncate(text, MAX_TEXT_CHARS),
        parts,
        grid,
        dimension,
    }
}

fn grid_of<'a>(
    rows: usize,
    cols: usize,
    at: impl Fn(usize, usize) -> Option<&'a Value>,
    digits: usize,
) -> Option<Grid> {
    if rows == 0 || cols == 0 {
        return None;
    }
    let shown_rows = rows.min(MAX_GRID_ROWS);
    let shown_cols = cols.min(MAX_GRID_COLS);
    let mut cells = Vec::with_capacity(shown_rows * shown_cols);
    for r in 0..shown_rows {
        for c in 0..shown_cols {
            let text = at(r, c).map_or_else(String::new, |v| v.display(digits));
            cells.push(truncate(text, 40));
        }
    }
    Some(Grid {
        rows,
        cols,
        cells,
        shown_rows,
        shown_cols,
    })
}

/// An error against the cell's source: its message, its position and the line it is on.
pub fn error_view(e: &CellError, source: &str) -> ErrorView {
    let mut view = ErrorView {
        code: e.code(),
        message: e.message().to_owned(),
        position: None,
        span: None,
        quote: None,
    };
    let Some(span) = e.span() else {
        return view;
    };
    view.span = Some((span.start, span.end.max(span.start)));
    let (line, column) = line_col(source, span.start);
    view.position = Some((line, column));
    let Some(text) = source.lines().nth(line.saturating_sub(1)) else {
        return view;
    };
    let width = source
        .get(span.start..span.end.min(source.len()))
        .map_or(1, |s| s.lines().next().unwrap_or("").chars().count())
        .max(1);
    // Quote a window of a long line, keeping the problem inside it.
    let caret = column.saturating_sub(1);
    let skip = caret.saturating_sub(MAX_QUOTE_CHARS / 2);
    let quoted: String = text.chars().skip(skip).take(MAX_QUOTE_CHARS).collect();
    view.quote = Some(Quote {
        line: quoted,
        caret: caret - skip,
        width: width.min(MAX_QUOTE_CHARS),
    });
    view
}

// --- The worker -----------------------------------------------------------------------------

/// The worker is gone (it never should be); the window says so instead of freezing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineGone;

/// A handle on the worker thread.
pub struct Engine {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    sent: u64,
    handled: u64,
}

impl Engine {
    /// Starts the worker. `wake` is called after every reply, from the worker's thread, so the
    /// window repaints and reads it.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> std::io::Result<Engine> {
        let (tx, requests) = channel::<Request>();
        let (replies, rx) = channel::<Reply>();
        // A deep expression recurses; give the worker room beyond the default thread's.
        std::thread::Builder::new()
            .name("abaco-engine".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let mut backend = Backend::new();
                while let Ok(first) = requests.recv() {
                    let mut batch = vec![first];
                    while let Ok(more) = requests.try_recv() {
                        batch.push(more);
                    }
                    let handled = batch.len();
                    let reply = catch_unwind(AssertUnwindSafe(|| backend.handle(batch)))
                        .unwrap_or_else(|_| internal_error(&backend, handled));
                    if replies.send(reply).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        let mut engine = Engine {
            tx,
            rx,
            sent: 0,
            handled: 0,
        };
        // The first reply describes the empty document.
        engine.sent = 0;
        Ok(engine)
    }

    pub fn send(&mut self, request: Request) -> Result<(), EngineGone> {
        self.tx.send(request).map_err(|_| EngineGone)?;
        self.sent += 1;
        Ok(())
    }

    /// The replies that have arrived, oldest first.
    pub fn poll(&mut self) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Ok(reply) = self.rx.try_recv() {
            self.handled += reply.handled as u64;
            out.push(reply);
        }
        out
    }

    /// Requests sent that no reply has covered yet.
    pub fn in_flight(&self) -> u64 {
        self.sent.saturating_sub(self.handled)
    }
}

/// What a batch that panicked answers: the document as it is, and a notice that it is a bug.
fn internal_error(backend: &Backend, handled: usize) -> Reply {
    let snapshot =
        catch_unwind(AssertUnwindSafe(|| backend.snapshot())).unwrap_or_else(|_| Snapshot::empty());
    Reply {
        snapshot,
        created: Vec::new(),
        notices: vec![Notice::Internal],
        handled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn math(index: usize, source: &str) -> Request {
        Request::InsertMath {
            index,
            source: source.to_owned(),
        }
    }

    fn outcome(snapshot: &Snapshot, index: usize) -> &Outcome {
        match &snapshot.cells[index].body {
            Body::Math { outcome, .. } => outcome,
            Body::Text { .. } => panic!("a text cell"),
        }
    }

    fn text_of(snapshot: &Snapshot, index: usize) -> String {
        match outcome(snapshot, index) {
            Outcome::Ok { value: Some(v), .. } => v.text.clone(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn a_batch_is_applied_in_order_and_evaluated_once() {
        let mut b = Backend::new();
        let reply = b.handle(vec![
            math(0, "m = 10 kg"),
            math(1, "F = m * g0 -> N"),
            Request::SetSource {
                id: CellId(1),
                source: "m = 20 kg".into(),
            },
        ]);
        assert_eq!(reply.handled, 3);
        assert_eq!(reply.created, vec![CellId(1), CellId(2)]);
        assert_eq!(text_of(&reply.snapshot, 1), "196.133 N");
        assert_eq!(reply.snapshot.evaluated, 2);
        assert!(reply.snapshot.dirty);
    }

    #[test]
    fn a_quantity_comes_apart_into_number_and_unit() {
        let mut b = Backend::new();
        let reply = b.handle(vec![math(0, "10 kg * 9.81 m/s^2 -> N")]);
        let Outcome::Ok { value: Some(v), .. } = outcome(&reply.snapshot, 0) else {
            panic!("a value");
        };
        assert_eq!(v.text, "98.1 N");
        assert_eq!(v.parts, Some(("98.1".to_owned(), "N".to_owned())));
        assert_eq!(v.dimension.as_deref(), Some("force"));
    }

    #[test]
    fn a_matrix_becomes_a_grid_and_a_vector_a_column() {
        let mut b = Backend::new();
        let reply = b.handle(vec![math(0, "[1, 2; 3, 4]"), math(1, "[5, 6, 7]")]);
        let grid = |i| match outcome(&reply.snapshot, i) {
            Outcome::Ok {
                value: Some(Shown { grid: Some(g), .. }),
                ..
            } => g.clone(),
            other => panic!("{other:?}"),
        };
        let m = grid(0);
        assert_eq!((m.rows, m.cols), (2, 2));
        assert_eq!(m.cells, ["1", "2", "3", "4"]);
        let v = grid(1);
        assert_eq!((v.rows, v.cols), (3, 1));
    }

    #[test]
    fn a_huge_vector_is_cut_not_drawn_whole() {
        let mut b = Backend::new();
        let reply = b.handle(vec![math(0, "linspace(0, 1, 5000)")]);
        let Outcome::Ok { value: Some(v), .. } = outcome(&reply.snapshot, 0) else {
            panic!("a value");
        };
        assert!(v.text.chars().count() <= MAX_TEXT_CHARS + 1);
        let g = v.grid.as_ref().unwrap();
        assert_eq!(g.rows, 5000);
        assert_eq!(g.shown_rows, MAX_GRID_ROWS);
    }

    #[test]
    fn errors_carry_line_column_and_the_quoted_line() {
        let mut b = Backend::new();
        let reply = b.handle(vec![math(0, "x = 1\ny = 5 m + 3 s")]);
        let Outcome::Err { error, .. } = outcome(&reply.snapshot, 0) else {
            panic!("an error");
        };
        assert_eq!(error.code, "dimension");
        let (line, column) = error.position.unwrap();
        assert_eq!(line, 2);
        assert!(column >= 1);
        let quote = error.quote.as_ref().unwrap();
        assert_eq!(quote.line, "y = 5 m + 3 s");
        assert!(quote.width >= 1);
        assert_eq!(reply.snapshot.errors, 1);
    }

    #[test]
    fn a_long_line_is_quoted_around_the_problem() {
        let source = format!("{} + $", "1 + ".repeat(400));
        let mut b = Backend::new();
        let reply = b.handle(vec![math(0, &source)]);
        let Outcome::Err { error, .. } = outcome(&reply.snapshot, 0) else {
            panic!("an error");
        };
        let q = error.quote.as_ref().unwrap();
        assert!(q.line.chars().count() <= MAX_QUOTE_CHARS);
        assert!(q.caret < MAX_QUOTE_CHARS);
    }

    #[test]
    fn edits_of_a_cell_that_is_gone_are_ignored() {
        let mut b = Backend::new();
        b.handle(vec![math(0, "1")]);
        let reply = b.handle(vec![
            Request::Remove { id: CellId(1) },
            Request::SetSource {
                id: CellId(1),
                source: "2".into(),
            },
        ]);
        assert!(reply.notices.is_empty());
        assert!(reply.snapshot.cells.is_empty());
    }

    #[test]
    fn move_remove_and_duplicate_change_the_page() {
        let mut b = Backend::new();
        b.handle(vec![math(0, "a = 1"), math(1, "b = 2"), math(2, "c = 3")]);
        let r = b.handle(vec![Request::Move {
            id: CellId(3),
            index: 0,
        }]);
        let ids: Vec<u64> = r.snapshot.cells.iter().map(|c| c.id.0).collect();
        assert_eq!(ids, [3, 1, 2]);
        let r = b.handle(vec![Request::Duplicate { id: CellId(1) }]);
        assert_eq!(r.created.len(), 1);
        assert_eq!(r.snapshot.cells.len(), 4);
        // The copy sits right after the original and defines the same name: the engine flags it.
        assert_eq!(r.snapshot.cells[2].id, r.created[0]);
        assert!(r.snapshot.errors >= 1);
        let r = b.handle(vec![Request::Remove { id: r.created[0] }]);
        assert_eq!(r.snapshot.cells.len(), 3);
        assert_eq!(r.snapshot.errors, 0);
    }

    #[test]
    fn dirty_follows_saves_and_ignores_no_op_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book");
        let mut b = Backend::new();
        let r = b.handle(vec![math(0, "x = 1")]);
        assert!(r.snapshot.dirty);
        let r = b.handle(vec![Request::Save {
            path: Some(path.clone()),
        }]);
        assert!(!r.snapshot.dirty);
        let saved = dir.path().join("book.abaco");
        assert_eq!(r.snapshot.path.as_deref(), Some(saved.as_path()));
        assert_eq!(r.notices, vec![Notice::Saved(saved.clone())]);
        let r = b.handle(vec![Request::SetSource {
            id: CellId(1),
            source: "x = 1".into(),
        }]);
        assert!(!r.snapshot.dirty, "the same text is not an edit");
        let r = b.handle(vec![Request::SetSource {
            id: CellId(1),
            source: "x = 2".into(),
        }]);
        assert!(r.snapshot.dirty);
        // Save again to the remembered path.
        let r = b.handle(vec![Request::Save { path: None }]);
        assert!(!r.snapshot.dirty);
    }

    #[test]
    fn saving_without_a_path_asks_for_one() {
        let mut b = Backend::new();
        let r = b.handle(vec![Request::Save { path: None }]);
        assert_eq!(r.notices, vec![Notice::NeedsPath]);
    }

    #[test]
    fn open_replaces_the_document_and_bumps_the_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.abaco");
        let mut b = Backend::new();
        b.handle(vec![
            math(0, "x = 3 m"),
            Request::Save {
                path: Some(path.clone()),
            },
        ]);
        let mut other = Backend::new();
        let before = other.snapshot().epoch;
        let r = other.handle(vec![Request::Open { path: path.clone() }]);
        assert!(r.snapshot.epoch > before);
        assert_eq!(text_of(&r.snapshot, 0), "3 m");
        assert!(!r.snapshot.dirty);
        assert_eq!(r.notices, vec![Notice::Opened(path)]);
    }

    #[test]
    fn a_bad_file_is_reported_and_the_document_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.abaco");
        std::fs::write(&bad, "{ nope").unwrap();
        let mut b = Backend::new();
        b.handle(vec![math(0, "x = 1")]);
        let r = b.handle(vec![
            Request::Open { path: bad.clone() },
            Request::Open {
                path: dir.path().join("missing.abaco"),
            },
        ]);
        assert_eq!(r.snapshot.cells.len(), 1);
        assert_eq!(r.notices.len(), 2);
        assert!(r
            .notices
            .iter()
            .all(|n| matches!(n, Notice::OpenFailed { .. })));
    }

    #[test]
    fn save_failure_is_a_notice() {
        let mut b = Backend::new();
        let r = b.handle(vec![Request::Save {
            path: Some("/nonexistent-dir-abaco/x.abaco".into()),
        }]);
        assert!(matches!(r.notices[0], Notice::SaveFailed { .. }));
    }

    #[test]
    fn new_replaces_the_document_with_the_given_cells() {
        let mut b = Backend::new();
        b.handle(vec![math(0, "x = 1")]);
        let r = b.handle(vec![Request::New {
            cells: vec![
                CellSpec::Text("# Title".into()),
                CellSpec::Math("2 + 2".into()),
            ],
            digits: 9,
        }]);
        assert_eq!(r.snapshot.cells.len(), 2);
        assert_eq!(r.snapshot.digits, 9);
        assert!(!r.snapshot.dirty && r.snapshot.path.is_none());
        assert_eq!(text_of(&r.snapshot, 1), "4");
    }

    #[test]
    fn digits_and_strict_order_change_the_document() {
        let mut b = Backend::new();
        b.handle(vec![math(0, "1/3")]);
        let r = b.handle(vec![Request::SetDigits(3)]);
        assert_eq!(text_of(&r.snapshot, 0), "0.333");
        let r = b.handle(vec![Request::SetDigits(99)]);
        assert!(matches!(r.notices[0], Notice::Rejected(_)));
        assert_eq!(r.snapshot.digits, 3);
        let r = b.handle(vec![Request::SetStrictOrder(true)]);
        assert!(r.snapshot.strict_order);
    }

    #[test]
    fn the_cell_limit_is_a_notice_not_a_crash() {
        let mut b = Backend::new();
        let big = vec![CellSpec::Text(String::new()); abaco_doc::MAX_CELLS + 5];
        let r = b.handle(vec![Request::New {
            cells: big,
            digits: 6,
        }]);
        assert!(matches!(r.notices[0], Notice::Rejected(_)));
        assert!(r.snapshot.cells.len() <= abaco_doc::MAX_CELLS);
    }

    #[test]
    fn hostile_requests_never_panic() {
        let mut b = Backend::new();
        let huge = "9".repeat(abaco_doc::MAX_SOURCE_BYTES + 1);
        let r = b.handle(vec![
            math(usize::MAX, &huge),
            math(
                usize::MAX,
                "x = ((((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))))",
            ),
            Request::Move {
                id: CellId(99),
                index: usize::MAX,
            },
            Request::Duplicate { id: CellId(99) },
            Request::Remove { id: CellId(99) },
            Request::SetText {
                id: CellId(1),
                markdown: "x".into(),
            },
            Request::SetDigits(0),
        ]);
        assert_eq!(r.handled, 7);
        assert!(!r.notices.is_empty());
    }

    #[test]
    fn the_worker_answers_and_coalesces() {
        let (woken, seen) = channel::<()>();
        let mut engine = Engine::spawn(move || {
            let _ = woken.send(());
        })
        .unwrap();
        for i in 0..50 {
            engine
                .send(Request::InsertMath {
                    index: i,
                    source: format!("v{i} = {i}"),
                })
                .unwrap();
        }
        assert!(engine.in_flight() > 0);
        let mut last = None;
        while engine.in_flight() > 0 {
            seen.recv_timeout(Duration::from_secs(10)).unwrap();
            for reply in engine.poll() {
                last = Some(reply);
            }
        }
        let reply = last.unwrap();
        assert_eq!(reply.snapshot.cells.len(), 50);
        assert_eq!(engine.in_flight(), 0);
    }

    #[test]
    fn a_runaway_cell_stops_at_the_step_budget_and_the_worker_lives() {
        let mut b = Backend::new();
        let r = b.handle(vec![
            math(0, "f(n) = f(n + 1)"),
            math(1, "f(1)"),
            math(2, "2 + 2"),
        ]);
        assert!(matches!(outcome(&r.snapshot, 1), Outcome::Err { .. }));
        assert_eq!(text_of(&r.snapshot, 2), "4");
    }
}
