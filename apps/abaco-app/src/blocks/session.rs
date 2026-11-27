// SPDX-License-Identifier: MIT OR Apache-2.0
//! The diagram being edited and everything the block workspace does to it that is not drawing:
//! adding, moving, wiring and deleting blocks, editing a block's parameters, running, and turning
//! the recorded signals into a figure. The window reads this and calls its methods.

use std::path::{Path, PathBuf};

use abaco_blocks::{Block, BlockKind, Line, Model, PaletteEntry, SimResult};
use abaco_script::plot::{Axes, Figure, Kind, Series};
use serde_json::{Map, Value};

use super::engine::{Finished, Job, Reply};

/// The extension of a diagram file.
pub const EXTENSION: &str = "json";
/// Most blocks the editor lets a person add (the engine's own limit is larger).
pub const MAX_BLOCKS: usize = 2_000;
/// Grid the blocks snap to, in world units.
pub const GRID: f64 = 10.0;
/// The worked example a first visit opens.
pub const EXAMPLE: &str = include_str!("../../../../examples/servo.json");

/// What a run last said.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Nothing has run yet.
    Idle,
    /// Done: the time it took and the solver's counters.
    Done {
        millis: u64,
        steps: u64,
        samples: usize,
        stopped_by: Option<String>,
    },
    /// The person stopped the run.
    Stopped,
    /// The diagram or the run failed; the message names the block.
    Failed(String),
}

/// The diagram, its selection and its last run.
pub struct Session {
    pub model: Model,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// The selected block's name.
    pub selected: Option<String>,
    /// The selected line, as an index into `model.lines`.
    pub selected_line: Option<usize>,
    pub running: bool,
    pub status: Status,
    /// The last run's recorded signals drawn as plots (one axes per signal).
    pub figure: Figure,
    pub has_result: bool,
    run_id: u64,
}

impl Default for Session {
    fn default() -> Session {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Session {
        Session {
            model: Model::new(""),
            path: None,
            dirty: false,
            selected: None,
            selected_line: None,
            running: false,
            status: Status::Idle,
            figure: Figure::default(),
            has_result: false,
            run_id: 0,
        }
    }

    /// The name the title bar shows, `None` for a diagram that has no file yet.
    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    /// Replaces the diagram with an empty one.
    pub fn new_diagram(&mut self) {
        *self = Session::new();
    }

    /// Replaces the diagram with the one in `text`.
    pub fn open(&mut self, path: Option<PathBuf>, text: &str) -> Result<(), String> {
        let mut model = Model::from_json(text).map_err(|e| e.to_string())?;
        auto_layout(&mut model);
        *self = Session::new();
        self.model = model;
        self.path = path;
        Ok(())
    }

    /// The worked example.
    pub fn open_example(&mut self) {
        // The example is part of the program; a failure here is a build problem caught by a test.
        if self.open(None, EXAMPLE).is_err() {
            self.new_diagram();
        }
    }

    /// The diagram as the text of a file.
    pub fn text(&self) -> Result<String, String> {
        self.model.to_json().map_err(|e| e.to_string())
    }

    fn touch(&mut self) {
        self.dirty = true;
        // What was plotted belongs to the diagram that made it; keep it on screen but say so.
    }

    // --- Editing ------------------------------------------------------------------------------

    fn unique_name(&self, base: &str) -> String {
        if self.model.block(base).is_none() {
            return base.to_owned();
        }
        (1..)
            .map(|n| format!("{base}{n}"))
            .find(|n| self.model.block(n).is_none())
            .unwrap_or_else(|| base.to_owned())
    }

    /// Adds a block from the palette at `at` (world units); returns its name.
    pub fn add_block(&mut self, entry: &PaletteEntry, at: (f64, f64)) -> Result<String, String> {
        if self.model.blocks.len() >= MAX_BLOCKS {
            return Err(format!("a diagram can have at most {MAX_BLOCKS} blocks"));
        }
        let name = self.unique_name(entry.name);
        self.model.blocks.push(Block {
            name: name.clone(),
            kind: entry.kind.clone(),
            position: Some(snap(at)),
        });
        self.selected = Some(name.clone());
        self.selected_line = None;
        self.touch();
        Ok(name)
    }

    /// Moves a block to `at` (world units, snapped to the grid).
    pub fn move_block(&mut self, name: &str, at: (f64, f64)) {
        if let Some(b) = self.model.blocks.iter_mut().find(|b| b.name == name) {
            let p = snap(at);
            if b.position != Some(p) {
                b.position = Some(p);
                self.dirty = true;
            }
        }
    }

    /// Deletes a block and the lines on it.
    pub fn delete_block(&mut self, name: &str) {
        let before = self.model.blocks.len();
        self.model.blocks.retain(|b| b.name != name);
        if self.model.blocks.len() != before {
            self.model
                .lines
                .retain(|l| l.from.0 != name && l.to.0 != name);
            if self.selected.as_deref() == Some(name) {
                self.selected = None;
            }
            self.selected_line = None;
            self.touch();
        }
    }

    /// Deletes whatever is selected, a block or a line.
    pub fn delete_selected(&mut self) {
        if let Some(name) = self.selected.clone() {
            self.delete_block(&name);
        } else if let Some(i) = self.selected_line.take() {
            if i < self.model.lines.len() {
                self.model.lines.remove(i);
                self.touch();
            }
        }
    }

    /// Duplicates a block a little to the lower right of the original.
    pub fn duplicate_block(&mut self, name: &str) -> Option<String> {
        let b = self.model.block(name)?.clone();
        if self.model.blocks.len() >= MAX_BLOCKS {
            return None;
        }
        let new = self.unique_name(name.trim_end_matches(|c: char| c.is_ascii_digit()));
        let (x, y) = b.position.unwrap_or((0.0, 0.0));
        self.model.blocks.push(Block {
            name: new.clone(),
            kind: b.kind,
            position: Some(snap((x + 40.0, y + 40.0))),
        });
        self.selected = Some(new.clone());
        self.selected_line = None;
        self.touch();
        Some(new)
    }

    /// Wires an output port to an input port; the input's previous line is replaced.
    pub fn connect(&mut self, from: (&str, usize), to: (&str, usize)) -> Result<(), String> {
        let ports = |n: &str| self.model.block(n).and_then(|b| b.kind.ports());
        let (_, outs) = ports(from.0).ok_or_else(|| format!("there is no block \"{}\"", from.0))?;
        let (ins, _) = ports(to.0).ok_or_else(|| format!("there is no block \"{}\"", to.0))?;
        if from.1 >= outs {
            return Err(format!("\"{}\" has no output {}", from.0, from.1 + 1));
        }
        if to.1 >= ins {
            return Err(format!("\"{}\" has no input {}", to.0, to.1 + 1));
        }
        self.model
            .lines
            .retain(|l| !(l.to.0 == to.0 && l.to.1 == to.1));
        self.model.lines.push(Line {
            from: (from.0.to_owned(), from.1),
            to: (to.0.to_owned(), to.1),
        });
        self.selected = None;
        self.selected_line = Some(self.model.lines.len() - 1);
        self.touch();
        Ok(())
    }

    /// Renames a block, keeping its lines.
    pub fn rename_block(&mut self, old: &str, new: &str) -> Result<(), String> {
        let new = new.trim();
        if new.is_empty() || new.contains('/') {
            return Err("a block name cannot be empty or contain \"/\"".into());
        }
        if new == old {
            return Ok(());
        }
        if self.model.block(new).is_some() {
            return Err(format!("there is already a block named \"{new}\""));
        }
        let b = self
            .model
            .blocks
            .iter_mut()
            .find(|b| b.name == old)
            .ok_or_else(|| format!("there is no block \"{old}\""))?;
        b.name = new.to_owned();
        for l in &mut self.model.lines {
            if l.from.0 == old {
                l.from.0 = new.to_owned();
            }
            if l.to.0 == old {
                l.to.0 = new.to_owned();
            }
        }
        if self.selected.as_deref() == Some(old) {
            self.selected = Some(new.to_owned());
        }
        self.touch();
        Ok(())
    }

    /// The block's parameters as a JSON object (without its `type`).
    pub fn params(&self, name: &str) -> Option<Map<String, Value>> {
        let b = self.model.block(name)?;
        let Value::Object(mut m) = serde_json::to_value(&b.kind).ok()? else {
            return None;
        };
        m.remove("type");
        Some(m)
    }

    /// Replaces a block's parameters. Lines to ports the new parameters no longer have are
    /// removed (their number is returned).
    pub fn set_params(&mut self, name: &str, params: Map<String, Value>) -> Result<usize, String> {
        let current = self
            .model
            .block(name)
            .ok_or_else(|| format!("there is no block \"{name}\""))?
            .kind
            .type_name();
        let mut object = params;
        object.insert("type".into(), Value::String(current));
        let kind: BlockKind =
            serde_json::from_value(Value::Object(object)).map_err(|e| e.to_string())?;
        let (ins, outs) = kind
            .ports()
            .ok_or_else(|| "these parameters are not valid for the block".to_string())?;
        let before = self.model.lines.len();
        self.model.lines.retain(|l| {
            !((l.to.0 == name && l.to.1 >= ins) || (l.from.0 == name && l.from.1 >= outs))
        });
        let removed = before - self.model.lines.len();
        if let Some(b) = self.model.blocks.iter_mut().find(|b| b.name == name) {
            b.kind = kind;
        }
        self.selected_line = None;
        self.touch();
        Ok(removed)
    }

    // --- Running ------------------------------------------------------------------------------

    /// Starts a run of the whole diagram; `None` when one is already going.
    pub fn run(&mut self) -> Option<Job> {
        if self.running {
            return None;
        }
        self.running = true;
        self.run_id += 1;
        Some(Job::Run {
            id: self.run_id,
            model: Box::new(self.model.clone()),
        })
    }

    /// Folds in what the worker answered.
    pub fn apply(&mut self, reply: Reply) {
        let Reply::Finished(f) = reply;
        let Finished {
            id,
            outcome,
            millis,
            stopped,
        } = *f;
        if id != self.run_id {
            return;
        }
        self.running = false;
        match outcome {
            Ok(result) => {
                self.figure = figure_from(&result);
                self.has_result = true;
                self.status = Status::Done {
                    millis,
                    steps: result.stats.steps,
                    samples: result.time.len(),
                    stopped_by: result.stopped_by.clone(),
                };
            }
            Err(_) if stopped => self.status = Status::Stopped,
            Err(message) => self.status = Status::Failed(message),
        }
    }
}

fn snap(at: (f64, f64)) -> (f64, f64) {
    let s = |v: f64| {
        if v.is_finite() {
            (v / GRID).round() * GRID
        } else {
            0.0
        }
    };
    (s(at.0), s(at.1))
}

/// Gives every block without a position one, in columns by distance from the sources.
pub fn auto_layout(model: &mut Model) {
    if model.blocks.iter().all(|b| b.position.is_some()) {
        return;
    }
    let index = |name: &str| model.blocks.iter().position(|b| b.name == name);
    let n = model.blocks.len();
    // Columns by longest path from a source (Kahn's order). A feedback loop leaves nothing with
    // no pending input; the first block still waiting is let go, which cuts the loop there.
    let mut incoming = vec![0usize; n];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in &model.lines {
        if let (Some(a), Some(b)) = (index(&l.from.0), index(&l.to.0)) {
            if a != b {
                incoming[b] += 1;
                successors[a].push(b);
            }
        }
    }
    let mut depth = vec![0usize; n];
    let mut done = vec![false; n];
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|i| incoming[*i] == 0).collect();
    let mut finished = 0;
    while finished < n {
        let Some(u) = queue.pop_front().or_else(|| (0..n).find(|i| !done[*i])) else {
            break;
        };
        if done[u] {
            continue;
        }
        done[u] = true;
        finished += 1;
        for &v in &successors[u] {
            if done[v] {
                continue;
            }
            depth[v] = depth[v].max(depth[u] + 1);
            incoming[v] = incoming[v].saturating_sub(1);
            if incoming[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    let mut rows: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let mut positions = Vec::with_capacity(n);
    for (i, b) in model.blocks.iter().enumerate() {
        let row = rows.entry(depth[i]).or_insert(0);
        let at = b
            .position
            .unwrap_or((40.0 + depth[i] as f64 * 150.0, 40.0 + *row as f64 * 90.0));
        *row += 1;
        positions.push(at);
    }
    for (b, p) in model.blocks.iter_mut().zip(positions) {
        b.position = Some(p);
    }
}

/// A figure with one axes per recorded signal (several series for a vector signal).
pub fn figure_from(result: &SimResult) -> Figure {
    let axes: Vec<Axes> = result
        .signals
        .iter()
        .map(|s| {
            let series = (0..s.width.max(1))
                .map(|k| Series {
                    kind: Kind::Line,
                    x: result.time.clone(),
                    y: s.column(k),
                    z: Vec::new(),
                    spec: String::new(),
                    label: if s.width > 1 {
                        format!("{}[{}]", s.name, k + 1)
                    } else {
                        String::new()
                    },
                })
                .collect();
            Axes {
                series,
                title: s.name.clone(),
                xlabel: "t".into(),
                grid: true,
                legend: s.width > 1,
                ..Axes::default()
            }
        })
        .collect();
    let rows = axes.len().max(1);
    Figure {
        rows,
        cols: 1,
        axes: if axes.is_empty() {
            vec![Axes::default()]
        } else {
            axes
        },
        current: 0,
        hold: false,
    }
}

/// The path with the diagram's extension.
pub fn with_extension(path: &Path) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
    {
        path.to_path_buf()
    } else {
        path.with_extension(EXTENSION)
    }
}

/// Most bytes of a Simulink file the editor reads.
const MAX_SIMULINK_BYTES: u64 = 64 << 20;

/// A diagram read from disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    /// The diagram as the text of an Ábaco file.
    pub text: String,
    /// It came from a Simulink file, so it has no Ábaco file of its own yet.
    pub imported: bool,
    /// What the conversion approximated or dropped.
    pub notes: Vec<String>,
}

/// Whether `path` names a Simulink model (`.mdl` or `.slx`).
pub fn is_simulink(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mdl") || e.eq_ignore_ascii_case("slx"))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    // One byte more than the limit tells a file that fits from one that does not.
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("the file is too large to open as a diagram".into());
    }
    Ok(bytes)
}

/// Reads a diagram file, refusing one that is larger than the engine reads. A Simulink model is
/// converted; its variables come from a MAT-file of the same name beside it, when there is one.
pub fn read_diagram(path: &Path) -> Result<Loaded, String> {
    if is_simulink(path) {
        let bytes = read_bounded(path, MAX_SIMULINK_BYTES)?;
        let workspace = std::fs::read(path.with_extension("mat"))
            .ok()
            .and_then(|b| abaco_formats::mat::read(&b).ok())
            .map(|f| abaco_blocks::simulink::workspace_from_mat(&f.vars))
            .unwrap_or_default();
        let import =
            abaco_blocks::simulink::read(&bytes, &workspace, false).map_err(|e| e.to_string())?;
        let text = import.model.to_json().map_err(|e| e.to_string())?;
        return Ok(Loaded {
            text,
            imported: true,
            notes: import.warnings,
        });
    }
    let bytes = read_bounded(path, abaco_blocks::model::MAX_FILE_BYTES as u64)?;
    let text = String::from_utf8(bytes).map_err(|_| "the file is not text".to_owned())?;
    Ok(Loaded {
        text,
        imported: false,
        notes: Vec::new(),
    })
}

/// The diagram in `text` as the bytes of a Simulink file: `.mdl` text, or an `.slx` archive for
/// any other extension.
pub fn simulink_bytes(text: &str, path: &Path) -> Result<Vec<u8>, String> {
    let model = Model::from_json(text).map_err(|e| e.to_string())?;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mdl"))
    {
        abaco_blocks::simulink::write_mdl(&model)
            .map(String::into_bytes)
            .map_err(|e| e.to_string())
    } else {
        abaco_blocks::simulink::write_slx(&model).map_err(|e| e.to_string())
    }
}

/// `path` with an `.slx` extension unless it already says `.mdl` or `.slx`.
pub fn with_simulink_extension(path: &Path) -> PathBuf {
    if is_simulink(path) {
        path.to_path_buf()
    } else {
        path.with_extension("slx")
    }
}

/// Writes a diagram next to its destination first and renames it, so a failure half way leaves
/// the old file untouched.
pub fn write_diagram(path: &Path, text: impl AsRef<[u8]>) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "diagram.json".to_owned());
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    let result = std::fs::write(&tmp, text.as_ref()).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
