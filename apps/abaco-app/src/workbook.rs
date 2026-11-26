// SPDX-License-Identifier: MIT OR Apache-2.0
//! The view-model: what the window reads and the reducer that changes it.
//!
//! The window draws a [`Workbook`] and reports what the person did as [`Msg`]s. [`Workbook::update`]
//! turns a message into the [`Request`]s the engine should get (and any change that is the
//! window's own, such as which cell is selected); [`Workbook::apply`] folds an engine [`Reply`]
//! back in. Nothing here touches egui or a thread.
//!
//! The text of a cell lives here, in its `buffer`, because the person edits it between replies:
//! a reply updates results and the cell list, never the text being typed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use abaco_doc::CellId;

use crate::engine::{Body, CellSpec, Notice, Outcome, Reply, Request, Snapshot};

/// What a cell is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Math,
    Text,
}

/// One cell as the window holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct CellState {
    pub id: CellId,
    pub kind: Kind,
    /// The source or the markdown, as edited.
    pub buffer: String,
    /// The latest result of a math cell.
    pub outcome: Outcome,
}

impl CellState {
    /// A short name for the navigator: the names a math cell defines, else its first line.
    pub fn title(&self) -> String {
        let first = |s: &str| {
            s.lines()
                .map(|l| l.trim().trim_start_matches('#').trim())
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .chars()
                .take(48)
                .collect::<String>()
        };
        match (&self.kind, &self.outcome) {
            (Kind::Math, Outcome::Ok { defined, .. }) if !defined.is_empty() => defined.join(", "),
            (Kind::Math, _) => first(&self.buffer.replace('#', " ")),
            (Kind::Text, _) => first(&self.buffer),
        }
    }

    pub fn is_blank(&self) -> bool {
        self.buffer.trim().is_empty()
    }
}

/// What the person did.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// The text of a cell was edited (its buffer already holds the new text).
    Edited(CellId),
    Select(CellId),
    /// A new cell after `after`, or at the end.
    AddMath {
        after: Option<CellId>,
    },
    AddText {
        after: Option<CellId>,
    },
    /// Ctrl+Enter: the cell is evaluated as you type, so this moves on to a fresh cell below.
    EvaluateAndAdd,
    Remove(CellId),
    MoveUp(CellId),
    MoveDown(CellId),
    Duplicate(CellId),
    /// Appends text (a unit, a constant) to the selected math cell.
    InsertIntoSelected(String),
    SetDigits(usize),
    SetStrictOrder(bool),
    /// A blank document with one empty cell.
    NewBlank,
    /// The sample document.
    NewSample,
    Open(PathBuf),
    /// Save to this path, or to the document's own when `None`.
    Save(Option<PathBuf>),
}

/// The document as the window shows it.
#[derive(Clone, Debug)]
pub struct Workbook {
    epoch: u64,
    pub cells: Vec<CellState>,
    pub selected: Option<CellId>,
    /// A cell whose editor should take the keyboard on the next frame.
    pub focus: Option<CellId>,
    /// A cell to scroll into view on the next frame.
    pub reveal: Option<CellId>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub digits: usize,
    pub strict_order: bool,
    pub errors: usize,
    pub evaluated: usize,
    pub elapsed: Duration,
    /// Requests are waiting for the engine.
    pub busy: bool,
    /// Messages for the person, oldest first; the window drains them.
    pub notices: Vec<Notice>,
    /// What to focus when the engine reports the cell the last add created.
    want_focus_on_created: bool,
}

impl Default for Workbook {
    fn default() -> Workbook {
        Workbook::new()
    }
}

impl Workbook {
    pub fn new() -> Workbook {
        Workbook::from_snapshot(&Snapshot::empty())
    }

    fn from_snapshot(s: &Snapshot) -> Workbook {
        let mut wb = Workbook {
            epoch: s.epoch,
            cells: Vec::new(),
            selected: None,
            focus: None,
            reveal: None,
            path: None,
            dirty: false,
            digits: s.digits,
            strict_order: false,
            errors: 0,
            evaluated: 0,
            elapsed: Duration::ZERO,
            busy: false,
            notices: Vec::new(),
            want_focus_on_created: false,
        };
        wb.take_scalars(s);
        wb.cells = s.cells.iter().map(fresh).collect();
        wb.selected = wb.cells.first().map(|c| c.id);
        wb
    }

    fn take_scalars(&mut self, s: &Snapshot) {
        self.path.clone_from(&s.path);
        self.dirty = s.dirty;
        self.digits = s.digits;
        self.strict_order = s.strict_order;
        self.errors = s.errors;
        self.evaluated = s.evaluated;
        self.elapsed = s.elapsed;
    }

    pub fn cell(&self, id: CellId) -> Option<&CellState> {
        self.cells.iter().find(|c| c.id == id)
    }

    pub fn cell_mut(&mut self, id: CellId) -> Option<&mut CellState> {
        self.cells.iter_mut().find(|c| c.id == id)
    }

    pub fn position(&self, id: CellId) -> Option<usize> {
        self.cells.iter().position(|c| c.id == id)
    }

    /// The cell a new one goes after: the given one, else the selected one.
    fn anchor(&self, after: Option<CellId>) -> Option<usize> {
        after.or(self.selected).and_then(|id| self.position(id))
    }

    /// The file's name for the title bar, or `None` for a document that has no file yet.
    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// How many math cells and text cells there are.
    pub fn counts(&self) -> (usize, usize) {
        let math = self.cells.iter().filter(|c| c.kind == Kind::Math).count();
        (math, self.cells.len() - math)
    }

    /// Applies what the person did and returns what the engine must be asked.
    pub fn update(&mut self, msg: Msg) -> Vec<Request> {
        match msg {
            Msg::Edited(id) => match self.cell(id) {
                Some(c) if c.kind == Kind::Math => vec![Request::SetSource {
                    id,
                    source: c.buffer.clone(),
                }],
                Some(c) => vec![Request::SetText {
                    id,
                    markdown: c.buffer.clone(),
                }],
                None => Vec::new(),
            },
            Msg::Select(id) => {
                if self.position(id).is_some() {
                    self.selected = Some(id);
                }
                Vec::new()
            }
            Msg::AddMath { after } => self.add(after, true),
            Msg::AddText { after } => self.add(after, false),
            Msg::EvaluateAndAdd => {
                let blank_math = self
                    .selected
                    .and_then(|id| self.cell(id))
                    .is_some_and(|c| c.kind == Kind::Math && c.is_blank());
                if blank_math {
                    self.focus = self.selected;
                    return Vec::new();
                }
                self.add(None, true)
            }
            Msg::Remove(id) => {
                let Some(pos) = self.position(id) else {
                    return Vec::new();
                };
                // The neighbour takes the selection, so the keyboard has somewhere to go.
                if self.selected == Some(id) {
                    let next = self
                        .cells
                        .get(pos + 1)
                        .or_else(|| pos.checked_sub(1).and_then(|p| self.cells.get(p)));
                    self.selected = next.map(|c| c.id);
                }
                self.cells.remove(pos);
                vec![Request::Remove { id }]
            }
            Msg::MoveUp(id) => self.shift(id, -1),
            Msg::MoveDown(id) => self.shift(id, 1),
            Msg::Duplicate(id) => {
                if self.position(id).is_none() {
                    return Vec::new();
                }
                self.want_focus_on_created = true;
                vec![Request::Duplicate { id }]
            }
            Msg::InsertIntoSelected(text) => {
                let Some(id) = self.selected else {
                    return Vec::new();
                };
                let Some(cell) = self.cell_mut(id).filter(|c| c.kind == Kind::Math) else {
                    return Vec::new();
                };
                let sep = if cell.buffer.is_empty() || cell.buffer.ends_with([' ', '(', '\n']) {
                    ""
                } else {
                    " "
                };
                cell.buffer = format!("{}{sep}{text}", cell.buffer);
                self.focus = Some(id);
                vec![Request::SetSource {
                    id,
                    source: self.cell(id).map(|c| c.buffer.clone()).unwrap_or_default(),
                }]
            }
            Msg::SetDigits(d) => vec![Request::SetDigits(d.clamp(1, 17))],
            Msg::SetStrictOrder(on) => vec![Request::SetStrictOrder(on)],
            Msg::NewBlank => {
                self.want_focus_on_created = false;
                vec![Request::New {
                    cells: vec![CellSpec::Math(String::new())],
                    digits: self.digits,
                }]
            }
            Msg::NewSample => vec![Request::New {
                cells: crate::sample::cells(),
                digits: self.digits,
            }],
            Msg::Open(path) => vec![Request::Open { path }],
            Msg::Save(path) => vec![Request::Save { path }],
        }
    }

    fn add(&mut self, after: Option<CellId>, math: bool) -> Vec<Request> {
        let index = self.anchor(after).map_or(self.cells.len(), |p| p + 1);
        self.want_focus_on_created = true;
        vec![if math {
            Request::InsertMath {
                index,
                source: String::new(),
            }
        } else {
            Request::InsertText {
                index,
                markdown: String::new(),
            }
        }]
    }

    fn shift(&mut self, id: CellId, by: isize) -> Vec<Request> {
        let Some(pos) = self.position(id) else {
            return Vec::new();
        };
        let Some(target) = pos.checked_add_signed(by).filter(|t| *t < self.cells.len()) else {
            return Vec::new();
        };
        // Show the move now; the reply confirms it.
        self.cells.swap(pos, target);
        self.reveal = Some(id);
        vec![Request::Move { id, index: target }]
    }

    /// Folds an engine reply in. Typed text is kept; results, the cell list and the status
    /// come from the snapshot.
    pub fn apply(&mut self, reply: Reply) {
        let Reply {
            snapshot,
            created,
            notices,
            ..
        } = reply;
        if snapshot.epoch != self.epoch {
            let keep_notices = std::mem::take(&mut self.notices);
            *self = Workbook::from_snapshot(&snapshot);
            self.notices = keep_notices;
        } else {
            let mut old: HashMap<CellId, CellState> =
                self.cells.drain(..).map(|c| (c.id, c)).collect();
            self.cells = snapshot
                .cells
                .iter()
                .map(|view| match old.remove(&view.id) {
                    Some(mut cell) => {
                        if let Body::Math { outcome, .. } = &view.body {
                            cell.outcome = outcome.clone();
                        }
                        cell
                    }
                    None => fresh(view),
                })
                .collect();
            self.take_scalars(&snapshot);
            if self.selected.is_none_or(|id| self.position(id).is_none()) {
                self.selected = self.cells.first().map(|c| c.id);
            }
        }
        if self.want_focus_on_created {
            if let Some(&id) = created.last() {
                self.want_focus_on_created = false;
                self.selected = Some(id);
                self.focus = Some(id);
                self.reveal = Some(id);
            }
        }
        self.notices.extend(notices);
    }
}

fn fresh(view: &crate::engine::CellView) -> CellState {
    match &view.body {
        Body::Math { source, outcome } => CellState {
            id: view.id,
            kind: Kind::Math,
            buffer: source.clone(),
            outcome: outcome.clone(),
        },
        Body::Text { markdown } => CellState {
            id: view.id,
            kind: Kind::Text,
            buffer: markdown.clone(),
            outcome: Outcome::Pending,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Backend;

    /// The window and the engine wired together without a thread.
    struct Rig {
        wb: Workbook,
        backend: Backend,
    }

    impl Rig {
        fn new() -> Rig {
            let backend = Backend::new();
            let mut rig = Rig {
                wb: Workbook::new(),
                backend,
            };
            rig.send(Vec::new());
            rig
        }

        fn send(&mut self, requests: Vec<Request>) {
            let reply = self.backend.handle(requests);
            self.wb.apply(reply);
        }

        fn msg(&mut self, msg: Msg) {
            let requests = self.wb.update(msg);
            self.send(requests);
        }

        fn type_in(&mut self, id: CellId, text: &str) {
            self.wb.cell_mut(id).unwrap().buffer = text.to_owned();
            self.msg(Msg::Edited(id));
        }
    }

    #[test]
    fn adding_a_cell_selects_and_focuses_it() {
        let mut rig = Rig::new();
        rig.msg(Msg::AddMath { after: None });
        let id = rig.wb.cells[0].id;
        assert_eq!(rig.wb.selected, Some(id));
        assert_eq!(rig.wb.focus, Some(id));
        rig.type_in(id, "x = 2 m");
        rig.msg(Msg::AddMath { after: None });
        assert_eq!(rig.wb.cells.len(), 2);
        assert_eq!(rig.wb.focus, Some(rig.wb.cells[1].id));
    }

    #[test]
    fn typing_keeps_the_buffer_and_shows_the_result() {
        let mut rig = Rig::new();
        rig.msg(Msg::AddMath { after: None });
        let id = rig.wb.cells[0].id;
        rig.type_in(id, "10 kg * 9.81 m/s^2 -> N");
        let cell = rig.wb.cell(id).unwrap();
        assert_eq!(cell.buffer, "10 kg * 9.81 m/s^2 -> N");
        assert!(matches!(&cell.outcome, Outcome::Ok { value: Some(v), .. } if v.text == "98.1 N"));
        assert!(rig.wb.dirty);
    }

    #[test]
    fn a_reply_does_not_overwrite_text_typed_since() {
        let mut rig = Rig::new();
        rig.msg(Msg::AddMath { after: None });
        let id = rig.wb.cells[0].id;
        // The person types "ab" but only "a" has been answered when the reply lands.
        rig.wb.cell_mut(id).unwrap().buffer = "a".into();
        let first = rig.wb.update(Msg::Edited(id));
        rig.wb.cell_mut(id).unwrap().buffer = "ab".into();
        let second = rig.wb.update(Msg::Edited(id));
        let reply = rig.backend.handle(first);
        rig.wb.apply(reply);
        assert_eq!(rig.wb.cell(id).unwrap().buffer, "ab");
        let reply = rig.backend.handle(second);
        rig.wb.apply(reply);
        assert_eq!(rig.wb.cell(id).unwrap().buffer, "ab");
    }

    #[test]
    fn ctrl_enter_adds_below_unless_the_cell_is_blank() {
        let mut rig = Rig::new();
        rig.msg(Msg::NewBlank);
        let first = rig.wb.cells[0].id;
        rig.msg(Msg::EvaluateAndAdd);
        assert_eq!(
            rig.wb.cells.len(),
            1,
            "a blank cell is not followed by another"
        );
        assert_eq!(rig.wb.focus, Some(first));
        rig.type_in(first, "1 + 1");
        rig.msg(Msg::EvaluateAndAdd);
        assert_eq!(rig.wb.cells.len(), 2);
        assert_eq!(rig.wb.selected, Some(rig.wb.cells[1].id));
        // Adding after the first inserts between cells.
        rig.msg(Msg::Select(first));
        rig.msg(Msg::EvaluateAndAdd);
        assert_eq!(rig.wb.cells.len(), 3);
        assert_eq!(rig.wb.position(rig.wb.selected.unwrap()), Some(1));
    }

    #[test]
    fn remove_moves_the_selection_to_a_neighbour() {
        let mut rig = Rig::new();
        rig.send(vec![Request::New {
            cells: vec![
                CellSpec::Math("a = 1".into()),
                CellSpec::Math("b = 2".into()),
                CellSpec::Math("c = 3".into()),
            ],
            digits: 6,
        }]);
        let ids: Vec<CellId> = rig.wb.cells.iter().map(|c| c.id).collect();
        rig.msg(Msg::Select(ids[1]));
        rig.msg(Msg::Remove(ids[1]));
        assert_eq!(rig.wb.selected, Some(ids[2]));
        rig.msg(Msg::Remove(ids[2]));
        assert_eq!(rig.wb.selected, Some(ids[0]));
        rig.msg(Msg::Remove(ids[0]));
        assert!(rig.wb.cells.is_empty());
        assert_eq!(rig.wb.selected, None);
        rig.msg(Msg::Remove(ids[0]));
    }

    #[test]
    fn moving_swaps_at_once_and_stops_at_the_ends() {
        let mut rig = Rig::new();
        rig.send(vec![Request::New {
            cells: vec![
                CellSpec::Math("a = 1".into()),
                CellSpec::Math("b = a".into()),
            ],
            digits: 6,
        }]);
        let (a, b) = (rig.wb.cells[0].id, rig.wb.cells[1].id);
        rig.msg(Msg::MoveUp(a));
        rig.msg(Msg::MoveDown(b));
        assert_eq!(rig.wb.cells[0].id, a);
        rig.msg(Msg::MoveDown(a));
        assert_eq!(rig.wb.cells[0].id, b);
        assert_eq!(rig.wb.reveal, Some(a));
        // The engine agrees with what the window showed.
        assert_eq!(rig.backend.snapshot().cells[0].id, b);
    }

    #[test]
    fn duplicate_focuses_the_copy() {
        let mut rig = Rig::new();
        rig.send(vec![Request::New {
            cells: vec![CellSpec::Math("2 + 2".into())],
            digits: 6,
        }]);
        let a = rig.wb.cells[0].id;
        rig.msg(Msg::Duplicate(a));
        assert_eq!(rig.wb.cells.len(), 2);
        assert_eq!(rig.wb.focus, Some(rig.wb.cells[1].id));
        assert_eq!(rig.wb.cells[1].buffer, "2 + 2");
    }

    #[test]
    fn inserting_a_unit_appends_to_the_selected_math_cell() {
        let mut rig = Rig::new();
        rig.msg(Msg::NewBlank);
        let id = rig.wb.cells[0].id;
        rig.msg(Msg::InsertIntoSelected("c".into()));
        assert_eq!(rig.wb.cell(id).unwrap().buffer, "c");
        rig.msg(Msg::InsertIntoSelected("* 2 s".into()));
        assert_eq!(rig.wb.cell(id).unwrap().buffer, "c * 2 s");
        assert!(rig.wb.cell(id).unwrap().outcome != Outcome::Pending);
        // A text cell takes nothing.
        rig.msg(Msg::AddText { after: None });
        let text = rig.wb.selected.unwrap();
        rig.msg(Msg::InsertIntoSelected("m".into()));
        assert_eq!(rig.wb.cell(text).unwrap().buffer, "");
    }

    #[test]
    fn new_and_open_replace_everything_and_drop_stale_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.abaco");
        let mut rig = Rig::new();
        rig.msg(Msg::NewBlank);
        let id = rig.wb.cells[0].id;
        rig.type_in(id, "x = 3 m");
        rig.msg(Msg::Save(Some(path.clone())));
        assert!(!rig.wb.dirty);
        assert_eq!(rig.wb.file_name().as_deref(), Some("a.abaco"));
        rig.type_in(id, "x = 4 m");
        assert!(rig.wb.dirty);
        rig.msg(Msg::Open(path.clone()));
        assert_eq!(
            rig.wb.cells[0].buffer, "x = 3 m",
            "the saved text comes back"
        );
        assert!(!rig.wb.dirty);
        assert!(rig.wb.notices.contains(&Notice::Opened(path.clone())));
        rig.msg(Msg::NewSample);
        assert!(rig.wb.cells.len() > 10);
        assert_eq!(rig.wb.path, None);
        assert_eq!(rig.wb.errors, 0);
    }

    #[test]
    fn titles_name_the_cell() {
        let mut rig = Rig::new();
        rig.send(vec![Request::New {
            cells: vec![
                CellSpec::Text("# Heading\nbody".into()),
                CellSpec::Math("a = 1\nb = 2".into()),
                CellSpec::Math("# only a comment\n1 + ".into()),
            ],
            digits: 6,
        }]);
        let titles: Vec<String> = rig.wb.cells.iter().map(CellState::title).collect();
        assert_eq!(titles[0], "Heading");
        assert_eq!(titles[1], "a, b");
        assert_eq!(titles[2], "only a comment");
    }

    #[test]
    fn counts_and_status_follow_the_snapshot() {
        let mut rig = Rig::new();
        rig.msg(Msg::NewSample);
        let (math, text) = rig.wb.counts();
        assert_eq!(math + text, rig.wb.cells.len());
        assert!(text >= 4 && math >= 10);
        assert!(rig.wb.evaluated >= 1);
        rig.msg(Msg::SetDigits(3));
        assert_eq!(rig.wb.digits, 3);
        rig.msg(Msg::SetDigits(500));
        assert_eq!(rig.wb.digits, 17);
        rig.msg(Msg::SetStrictOrder(true));
        assert!(rig.wb.strict_order);
    }

    #[test]
    fn stale_messages_do_nothing() {
        let mut wb = Workbook::new();
        for msg in [
            Msg::Edited(CellId(9)),
            Msg::Select(CellId(9)),
            Msg::Remove(CellId(9)),
            Msg::MoveUp(CellId(9)),
            Msg::Duplicate(CellId(9)),
            Msg::InsertIntoSelected("x".into()),
        ] {
            assert!(wb.update(msg).is_empty());
        }
    }
}
