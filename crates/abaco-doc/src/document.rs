// SPDX-License-Identifier: MIT OR Apache-2.0
//! The document: ordered cells and their incremental, dependency-ordered evaluation.

use crate::graph;
use crate::model::{
    Cell, CellError, CellId, CellKind, CellOutput, CellResult, DocError, Settings, MAX_CELLS,
    MAX_SOURCE_BYTES, MAX_TEXT_BYTES,
};
use abaco_expr::{eval_program, parse_with, Env, ExprError, Limits, Program, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What parsing a math cell gave, kept until the cell's text changes.
#[derive(Clone, Debug)]
struct Parsed {
    program: Result<Program, ExprError>,
    defined: Vec<String>,
    used: BTreeSet<String>,
}

/// What the graph says about a cell; when it changes the cell is evaluated again.
#[derive(Clone, PartialEq, Debug, Default)]
struct Signature {
    resolved: BTreeMap<String, CellId>,
    duplicate: Option<(String, CellId)>,
    cycle: Option<Vec<CellId>>,
    order_warnings: Vec<String>,
}

#[derive(Default)]
struct Runtime {
    parsed: Option<Parsed>,
    result: Option<CellResult>,
    /// Only the names this cell defines.
    exports: Env,
    signature: Signature,
}

/// A maths document: cells in page order, evaluated in dependency order.
///
/// Editing marks work to do; [`Document::evaluate`] does it, re-evaluating only the cells
/// that are new, edited, or downstream of something whose outcome changed.
pub struct Document {
    cells: Vec<Cell>,
    next_id: u64,
    settings: Settings,
    limits: Limits,
    rt: HashMap<CellId, Runtime>,
    dirty: BTreeSet<CellId>,
    all_dirty: bool,
    evaluations: u64,
    last_evaluated: Vec<CellId>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("cells", &self.cells)
            .field("settings", &self.settings)
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl Default for Document {
    fn default() -> Document {
        Document::new()
    }
}

impl Document {
    /// An empty document with default settings.
    pub fn new() -> Document {
        Document {
            cells: Vec::new(),
            next_id: 1,
            settings: Settings::default(),
            limits: Limits::default(),
            rt: HashMap::new(),
            dirty: BTreeSet::new(),
            all_dirty: true,
            evaluations: 0,
            last_evaluated: Vec::new(),
        }
    }

    /// Rebuilds a document from stored cells. The caller has validated them.
    pub(crate) fn from_parts(cells: Vec<Cell>, next_id: u64, settings: Settings) -> Document {
        let mut d = Document::new();
        d.cells = cells;
        d.next_id = next_id;
        d.settings = settings;
        d
    }

    // ---------- reading ----------

    /// The cells in page order.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The cell with this id.
    pub fn cell(&self, id: CellId) -> Option<&Cell> {
        self.cells.iter().find(|c| c.id == id)
    }

    /// The 0-based position of a cell on the page.
    pub fn position(&self, id: CellId) -> Option<usize> {
        self.cells.iter().position(|c| c.id == id)
    }

    /// The next id that will be handed out.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// The document settings.
    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// The evaluation limits applied to every cell.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The result of a math cell as of the last [`Document::evaluate`].
    pub fn result(&self, id: CellId) -> Option<&CellResult> {
        self.rt.get(&id).and_then(|r| r.result.as_ref())
    }

    /// How many cell evaluations have run in this document's lifetime.
    pub fn evaluations(&self) -> u64 {
        self.evaluations
    }

    /// The cells the last [`Document::evaluate`] evaluated, in the order it ran them.
    pub fn last_evaluated(&self) -> &[CellId] {
        &self.last_evaluated
    }

    /// `true` when edits are waiting for [`Document::evaluate`].
    pub fn needs_evaluation(&self) -> bool {
        self.all_dirty || !self.dirty.is_empty()
    }

    /// Number of math cells whose result is an error.
    pub fn error_count(&self) -> usize {
        self.cells
            .iter()
            .filter_map(|c| self.result(c.id))
            .filter(|r| r.outcome.is_err())
            .count()
    }

    /// Formats a value with the document's significant digits.
    pub fn display(&self, value: &Value) -> String {
        value.display(self.settings.significant_digits)
    }

    // ---------- editing ----------

    fn check_new(&self, kind: &CellKind) -> Result<(), DocError> {
        if self.cells.len() >= MAX_CELLS {
            return Err(DocError::TooLarge(format!(
                "a document holds at most {MAX_CELLS} cells"
            )));
        }
        check_kind_size(kind)
    }

    /// Inserts a cell at `index` (0 is first; `len` appends) and returns its new id.
    pub fn insert(&mut self, index: usize, kind: CellKind) -> Result<CellId, DocError> {
        if index > self.cells.len() {
            return Err(DocError::BadIndex {
                index,
                len: self.cells.len(),
            });
        }
        self.check_new(&kind)?;
        let id = CellId(self.next_id);
        self.next_id += 1;
        self.cells.insert(index, Cell { id, kind });
        self.dirty.insert(id);
        Ok(id)
    }

    /// Appends a math cell.
    pub fn push_math(&mut self, source: impl Into<String>) -> Result<CellId, DocError> {
        self.insert(
            self.cells.len(),
            CellKind::Math {
                source: source.into(),
            },
        )
    }

    /// Appends a text cell.
    pub fn push_text(&mut self, markdown: impl Into<String>) -> Result<CellId, DocError> {
        self.insert(
            self.cells.len(),
            CellKind::Text {
                markdown: markdown.into(),
            },
        )
    }

    /// Replaces the source of a math cell.
    pub fn set_source(&mut self, id: CellId, source: impl Into<String>) -> Result<(), DocError> {
        let source = source.into();
        let cell = self
            .cells
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(DocError::UnknownCell(id))?;
        let CellKind::Math { source: slot } = &mut cell.kind else {
            return Err(DocError::WrongKind(id, "math"));
        };
        check_kind_size(&CellKind::Math {
            source: source.clone(),
        })?;
        if *slot != source {
            *slot = source;
            if let Some(rt) = self.rt.get_mut(&id) {
                rt.parsed = None;
            }
            self.dirty.insert(id);
        }
        Ok(())
    }

    /// Replaces the markdown of a text cell.
    pub fn set_text(&mut self, id: CellId, markdown: impl Into<String>) -> Result<(), DocError> {
        let markdown = markdown.into();
        let cell = self
            .cells
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(DocError::UnknownCell(id))?;
        let CellKind::Text { markdown: slot } = &mut cell.kind else {
            return Err(DocError::WrongKind(id, "text"));
        };
        check_kind_size(&CellKind::Text {
            markdown: markdown.clone(),
        })?;
        *slot = markdown;
        Ok(())
    }

    /// Removes a cell and returns it.
    pub fn remove(&mut self, id: CellId) -> Result<Cell, DocError> {
        let pos = self.position(id).ok_or(DocError::UnknownCell(id))?;
        self.rt.remove(&id);
        self.dirty.remove(&id);
        // Dependents see their names resolve differently on the next evaluation.
        Ok(self.cells.remove(pos))
    }

    /// Moves a cell so that it ends up at `index` (0-based, among the remaining cells).
    pub fn move_cell(&mut self, id: CellId, index: usize) -> Result<(), DocError> {
        let pos = self.position(id).ok_or(DocError::UnknownCell(id))?;
        if index >= self.cells.len() {
            return Err(DocError::BadIndex {
                index,
                len: self.cells.len(),
            });
        }
        let cell = self.cells.remove(pos);
        self.cells.insert(index, cell);
        Ok(())
    }

    /// Changes the settings. Turning `strict_order` on or off re-evaluates the page.
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), DocError> {
        settings.validate()?;
        if settings.strict_order != self.settings.strict_order {
            self.all_dirty = true;
        }
        self.settings = settings;
        Ok(())
    }

    /// Changes the evaluation limits; every cell is evaluated again.
    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
        for rt in self.rt.values_mut() {
            rt.parsed = None;
        }
        self.all_dirty = true;
    }

    // ---------- evaluation ----------

    /// Brings every result up to date and returns the cells it evaluated.
    ///
    /// Order follows the dependency graph, not the page. A cell is evaluated when it is new,
    /// edited, resolves a name differently than before, or uses a cell whose outcome just
    /// changed; a cell whose outcome comes out the same does not wake its dependents.
    pub fn evaluate(&mut self) -> Vec<CellId> {
        let mut rt = std::mem::take(&mut self.rt);
        let order = self.run(&mut rt);
        self.rt = rt;
        self.dirty.clear();
        self.all_dirty = false;
        self.evaluations += order.len() as u64;
        self.last_evaluated = order.clone();
        order
    }

    fn run(&self, rt: &mut HashMap<CellId, Runtime>) -> Vec<CellId> {
        let live: HashSet<CellId> = self.cells.iter().map(|c| c.id).collect();
        rt.retain(|id, _| live.contains(id));

        // 1. Parse what changed.
        let mut math: Vec<(CellId, &str)> = Vec::new();
        for cell in &self.cells {
            if let CellKind::Math { source } = &cell.kind {
                math.push((cell.id, source.as_str()));
                let slot = rt.entry(cell.id).or_default();
                if slot.parsed.is_none() {
                    slot.parsed = Some(parse_cell(source, &self.limits));
                }
            }
        }

        // 2. Who defines which name. The first cell on the page wins; later ones are errors.
        let mut definers: HashMap<&str, CellId> = HashMap::new();
        let mut duplicates: HashMap<CellId, (String, CellId)> = HashMap::new();
        let mut nodes: Vec<CellId> = Vec::new();
        for (id, _) in &math {
            let Some(parsed) = rt.get(id).and_then(|r| r.parsed.as_ref()) else {
                continue;
            };
            if parsed.program.is_err() {
                continue;
            }
            nodes.push(*id);
            for name in &parsed.defined {
                match definers.get(name.as_str()) {
                    None => {
                        definers.insert(name.as_str(), *id);
                    }
                    Some(&first) if first != *id => {
                        duplicates
                            .entry(*id)
                            .or_insert_with(|| (name.clone(), first));
                    }
                    Some(_) => {}
                }
            }
        }

        // 3. Edges: a cell depends on the cells that define the names it uses.
        let node_of: HashMap<CellId, usize> =
            nodes.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let position: HashMap<CellId, usize> = self
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i))
            .collect();
        let mut resolved: Vec<BTreeMap<String, CellId>> = Vec::with_capacity(nodes.len());
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(nodes.len());
        for id in &nodes {
            let mut map = BTreeMap::new();
            if let Some(parsed) = rt.get(id).and_then(|r| r.parsed.as_ref()) {
                for name in &parsed.used {
                    if let Some(&d) = definers.get(name.as_str()) {
                        map.insert(name.clone(), d);
                    }
                }
            }
            let mut edges: Vec<usize> = map
                .values()
                .filter_map(|d| node_of.get(d).copied())
                .collect();
            edges.sort_unstable();
            edges.dedup();
            deps.push(edges);
            resolved.push(map);
        }

        // 4. Cycles and evaluation order.
        let comps = graph::components(&deps);
        let mut cycle_of: HashMap<usize, Vec<CellId>> = HashMap::new();
        let mut cycle_message: HashMap<usize, String> = HashMap::new();
        for comp in &comps {
            let self_loop = comp.len() == 1
                && comp
                    .first()
                    .is_some_and(|n| deps.get(*n).is_some_and(|e| e.contains(n)));
            if comp.len() < 2 && !self_loop {
                continue;
            }
            // The member that is first on the page starts the reported path.
            let Some(start) = comp
                .iter()
                .copied()
                .min_by_key(|n| nodes.get(*n).and_then(|id| position.get(id)).copied())
            else {
                continue;
            };
            let path = graph::cycle_through(&deps, comp, start);
            let ids: Vec<CellId> = path.iter().filter_map(|n| nodes.get(*n).copied()).collect();
            let message = cycle_text(&path, &nodes, &resolved);
            for n in comp {
                cycle_of.insert(*n, ids.clone());
                cycle_message.insert(*n, message.clone());
            }
        }

        // 5. Evaluate what is stale.
        let mut ran: Vec<CellId> = Vec::new();
        let mut changed: HashSet<usize> = HashSet::new();

        // Cells that do not parse are not part of the graph.
        for (id, _) in &math {
            if node_of.contains_key(id) {
                continue;
            }
            let Some(slot) = rt.get_mut(id) else { continue };
            let err = match slot.parsed.as_ref().map(|p| &p.program) {
                Some(Err(e)) => e.clone(),
                _ => continue,
            };
            let result = CellResult {
                outcome: Err(CellError::Expr(err)),
                warnings: Vec::new(),
            };
            if slot.result.as_ref() != Some(&result) || self.all_dirty || self.dirty.contains(id) {
                slot.result = Some(result);
                slot.exports = Env::new();
                slot.signature = Signature::default();
                ran.push(*id);
            }
        }

        for comp in &comps {
            for &n in comp {
                let Some(&id) = nodes.get(n) else { continue };
                let sig = Signature {
                    resolved: resolved.get(n).cloned().unwrap_or_default(),
                    duplicate: duplicates.get(&id).cloned(),
                    cycle: cycle_of.get(&n).cloned(),
                    order_warnings: self.order_warnings(id, resolved.get(n), &position),
                };
                let stale = {
                    let slot = rt.get(&id);
                    self.all_dirty
                        || self.dirty.contains(&id)
                        || slot.is_none_or(|s| s.result.is_none() || s.signature != sig)
                        || deps
                            .get(n)
                            .is_some_and(|e| e.iter().any(|d| changed.contains(d)))
                };
                if !stale {
                    continue;
                }
                let (result, exports) =
                    self.evaluate_cell(id, &sig, cycle_message.get(&n), rt, &position);
                let slot = rt.entry(id).or_default();
                let differs = slot.result.as_ref() != Some(&result) || slot.exports != exports;
                slot.result = Some(result);
                slot.exports = exports;
                slot.signature = sig;
                if differs {
                    changed.insert(n);
                }
                ran.push(id);
            }
        }
        ran
    }

    fn order_warnings(
        &self,
        id: CellId,
        resolved: Option<&BTreeMap<String, CellId>>,
        position: &HashMap<CellId, usize>,
    ) -> Vec<String> {
        if !self.settings.strict_order {
            return Vec::new();
        }
        let Some(mine) = position.get(&id) else {
            return Vec::new();
        };
        resolved
            .into_iter()
            .flatten()
            .filter(|(_, def)| **def != id && position.get(def).is_some_and(|p| p > mine))
            .map(|(name, _)| {
                format!("`{name}` is used above the cell that defines it; move that cell up")
            })
            .collect()
    }

    fn evaluate_cell(
        &self,
        id: CellId,
        sig: &Signature,
        cycle_message: Option<&String>,
        rt: &HashMap<CellId, Runtime>,
        _position: &HashMap<CellId, usize>,
    ) -> (CellResult, Env) {
        let fail = |e: CellError| {
            (
                CellResult {
                    outcome: Err(e),
                    warnings: sig.order_warnings.clone(),
                },
                Env::new(),
            )
        };
        if let Some((name, first)) = &sig.duplicate {
            return fail(CellError::Duplicate {
                name: name.clone(),
                first: *first,
                message: format!(
                    "`{name}` is already defined by an earlier cell; a name can be defined once per document"
                ),
            });
        }
        if let (Some(path), Some(message)) = (&sig.cycle, cycle_message) {
            return fail(CellError::Cycle {
                path: path.clone(),
                message: message.clone(),
            });
        }
        let Some(program) = rt
            .get(&id)
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.program.as_ref().ok())
        else {
            return fail(CellError::Expr(ExprError::new(
                abaco_expr::ErrorKind::Syntax,
                "the cell could not be read",
            )));
        };

        // The environment holds exactly the names this cell uses from other cells.
        let mut env = Env::new();
        for (name, def) in &sig.resolved {
            let Some(other) = rt.get(def) else { continue };
            match other.result.as_ref().map(|r| &r.outcome) {
                Some(Ok(_)) => {}
                _ => {
                    return fail(CellError::Dependency {
                        cell: *def,
                        name: name.clone(),
                        message: format!(
                            "`{name}` cannot be computed because the cell that defines it has an error"
                        ),
                    });
                }
            }
            if let Some(v) = other.exports.var(name) {
                env.set_var(name.clone(), v.clone());
            }
            if let Some(f) = other.exports.func(name) {
                env.set_fn(f.clone());
            }
        }

        match eval_program(program, &mut env, &self.limits) {
            Ok(ev) => {
                let mut exports = Env::new();
                for name in &ev.defined {
                    if let Some(v) = env.var(name) {
                        exports.set_var(name.clone(), v.clone());
                    }
                    if let Some(f) = env.func(name) {
                        exports.set_fn(f.clone());
                    }
                }
                let mut warnings = sig.order_warnings.clone();
                warnings.extend(ev.warnings);
                (
                    CellResult {
                        outcome: Ok(CellOutput {
                            value: ev.value,
                            defined: ev.defined,
                        }),
                        warnings,
                    },
                    exports,
                )
            }
            Err(e) => fail(CellError::Expr(e)),
        }
    }
}

fn check_kind_size(kind: &CellKind) -> Result<(), DocError> {
    match kind {
        CellKind::Math { source } if source.len() > MAX_SOURCE_BYTES => {
            Err(DocError::TooLarge(format!(
                "a math cell holds at most {MAX_SOURCE_BYTES} bytes; this one has {}",
                source.len()
            )))
        }
        CellKind::Text { markdown } if markdown.len() > MAX_TEXT_BYTES => {
            Err(DocError::TooLarge(format!(
                "a text cell holds at most {MAX_TEXT_BYTES} bytes; this one has {}",
                markdown.len()
            )))
        }
        _ => Ok(()),
    }
}

fn parse_cell(source: &str, limits: &Limits) -> Parsed {
    match parse_with(source, limits) {
        Ok(program) => {
            let defined = program.defined_names();
            let used = program.used_names();
            Parsed {
                program: Ok(program),
                defined,
                used,
            }
        }
        Err(e) => Parsed {
            program: Err(e),
            defined: Vec::new(),
            used: BTreeSet::new(),
        },
    }
}

/// ``circular reference: `a` -> `b` -> `a` `` where each name is defined by the cell that
/// uses the next one.
fn cycle_text(path: &[usize], nodes: &[CellId], resolved: &[BTreeMap<String, CellId>]) -> String {
    let mut labels: Vec<String> = Vec::new();
    for (i, n) in path.iter().enumerate() {
        // The name through which the previous cell in the cycle refers to this one.
        let prev = if i == 0 {
            path.last().copied()
        } else {
            path.get(i - 1).copied()
        };
        let this_id = nodes.get(*n).copied();
        let label = prev
            .and_then(|p| resolved.get(p))
            .and_then(|m| {
                m.iter()
                    .find(|(_, d)| Some(**d) == this_id)
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_else(|| "?".to_string());
        labels.push(format!("`{label}`"));
    }
    if let Some(first) = labels.first().cloned() {
        labels.push(first);
    }
    format!("circular reference: {}", labels.join(" -> "))
}
