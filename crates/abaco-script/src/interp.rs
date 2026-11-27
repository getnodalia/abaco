// SPDX-License-Identifier: MIT OR Apache-2.0
//! The interpreter: workspaces, calls, control flow, assignment and printing.

use crate::ast::{BinOp, Expr, ExprKind, FuncDef, Program, Stmt, StmtKind, UnOp};
use crate::builtins;
use crate::display;
use crate::error::{err, Error, Result, Span};
use crate::index::{self, Sel};
use crate::ops;
use crate::parser::parse;
use crate::value::{Arr, FnValue, StructV, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Resource limits for one interpreter.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most evaluation steps (statements, loop iterations, expression nodes).
    pub max_steps: u64,
    /// Deepest nesting of function calls.
    pub max_call_depth: usize,
    /// Most characters of printed output kept per run.
    pub max_output: usize,
    /// Most bytes of native stack a run may use for nested expressions and calls. The
    /// default suits a 2 MB thread in a debug build; a host that runs scripts on a thread
    /// with a bigger stack raises it to about three quarters of that stack.
    pub max_stack: usize,
    /// Whether the script may read and write files (`readmatrix`, `writematrix`, `save`,
    /// `load`, `fileread`). Off by default: a host that runs scripts it did not write leaves
    /// it off, a host that runs the person's own scripts turns it on.
    pub files: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_steps: 200_000_000,
            max_call_depth: 256,
            max_output: 4_000_000,
            max_stack: 1_000_000,
            files: false,
        }
    }
}

/// Something a script did that the host should show.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Printed text.
    Text(String),
    /// A warning message.
    Warning(String),
    /// The figure state changed; the host redraws from [`Interp::figures`].
    Figure(usize),
    /// `clc`.
    ClearScreen,
}

struct Frame {
    vars: HashMap<String, Value>,
    nargin: usize,
    nargout: usize,
    globals: HashSet<String>,
}

impl Frame {
    fn new() -> Frame {
        Frame {
            vars: HashMap::new(),
            nargin: 0,
            nargout: 0,
            globals: HashSet::new(),
        }
    }
}

enum Flow {
    Normal,
    Break,
    Continue,
    Return,
}

/// A small, fast, seedable generator (xoshiro256++), so results are reproducible.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// A generator from a seed.
    pub fn new(seed: u64) -> Rng {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng {
            s: [next(), next(), next(), next()],
        }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let r = self.s[0]
            .wrapping_add(self.s[3])
            .rotate_left(23)
            .wrapping_add(self.s[0]);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    /// Uniform in the open interval (0, 1).
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let (u1, u2) = (self.uniform(), self.uniform());
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// The interpreter.
pub struct Interp {
    frames: Vec<Frame>,
    globals: HashMap<String, Value>,
    funcs: HashMap<String, Arc<FuncDef>>,
    events: Vec<Event>,
    limits: Limits,
    steps: u64,
    out_chars: usize,
    /// Set from another thread to stop a run.
    pub cancel: Arc<AtomicBool>,
    end_stack: Vec<(usize, usize, usize)>,
    /// Random number generator.
    pub rng: Rng,
    /// The start time of the last `tic`.
    pub tic: Option<Instant>,
    /// `format long`.
    pub format_long: bool,
    /// The last error caught or raised (for `lasterr`).
    pub last_error: Option<Error>,
    /// Figures drawn by plotting commands.
    pub figures: crate::plot::Figures,
    stack_base: usize,
    search_path: Vec<PathBuf>,
    file_funcs: HashSet<String>,
}

impl Default for Interp {
    fn default() -> Interp {
        Interp::new()
    }
}

/// Registry lookup shared with builtins.
pub fn is_builtin(name: &str) -> bool {
    builtins::lookup(name).is_some()
}

impl Interp {
    /// A new interpreter with an empty workspace.
    pub fn new() -> Interp {
        Interp::with_limits(Limits::default())
    }

    /// A new interpreter with the given limits.
    pub fn with_limits(limits: Limits) -> Interp {
        Interp {
            frames: vec![Frame::new()],
            globals: HashMap::new(),
            funcs: HashMap::new(),
            events: Vec::new(),
            limits,
            steps: 0,
            out_chars: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            end_stack: Vec::new(),
            rng: Rng::new(0x5EED),
            tic: None,
            format_long: false,
            last_error: None,
            figures: crate::plot::Figures::default(),
            stack_base: 0,
            search_path: Vec::new(),
            file_funcs: HashSet::new(),
        }
    }

    // ---------- host API ----------

    /// Runs a script in the base workspace. Printed output accumulates as events.
    pub fn run(&mut self, src: &str) -> Result<()> {
        self.steps = 0;
        self.out_chars = 0;
        self.cancel.store(false, Ordering::Relaxed);
        let program = parse(src)?;
        self.exec_program(&program)
    }

    /// Runs source text in the current frame, keeping the step budget (for `eval`).
    pub fn eval_code(&mut self, src: &str) -> Result<()> {
        let program = parse(src)?;
        self.exec_program(&program)
    }

    /// Runs a command-syntax statement such as `format long` or `close all`.
    pub fn run_command(&mut self, name: &str, args: &[String]) -> Result<()> {
        self.command(name, args)
    }

    /// Evaluates one expression in the current workspace.
    pub fn eval_str(&mut self, src: &str) -> Result<Value> {
        let program = parse(src)?;
        let [stmt] = program.stmts.as_slice() else {
            return err("Abaco:syntax", "expected a single expression");
        };
        match &stmt.kind {
            StmtKind::Expr(e) => self.eval(e),
            _ => err("Abaco:syntax", "expected an expression"),
        }
    }

    /// Takes the events produced since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The text printed since the last call (events other than text are dropped).
    pub fn take_output(&mut self) -> String {
        self.take_events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// A variable of the current workspace.
    pub fn get(&self, name: &str) -> Option<&Value> {
        let f = self.frames.last()?;
        if f.globals.contains(name) {
            self.globals.get(name)
        } else {
            f.vars.get(name)
        }
    }

    /// Sets a variable of the current workspace.
    pub fn set(&mut self, name: &str, v: Value) {
        let global = self.frames.last().is_some_and(|f| f.globals.contains(name));
        if global {
            self.globals.insert(name.to_string(), v);
        } else if let Some(f) = self.frames.last_mut() {
            f.vars.insert(name.to_string(), v);
        }
    }

    /// Removes a variable.
    pub fn remove(&mut self, name: &str) {
        if let Some(f) = self.frames.last_mut() {
            f.vars.remove(name);
        }
    }

    /// Removes every variable of the base workspace.
    pub fn clear_all(&mut self) {
        if let Some(f) = self.frames.last_mut() {
            f.vars.clear();
        }
        self.globals.clear();
        self.forget_file_funcs();
    }

    /// Functions loaded from `.m` files are read again the next time they are called.
    fn forget_file_funcs(&mut self) {
        for name in std::mem::take(&mut self.file_funcs) {
            self.funcs.remove(&name);
        }
    }

    /// The variables of the current workspace, sorted by name.
    pub fn variables(&self) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        if let Some(f) = self.frames.last() {
            for (k, v) in &f.vars {
                out.push((k.clone(), v.clone()));
            }
            for g in &f.globals {
                if let Some(v) = self.globals.get(g) {
                    out.push((g.clone(), v.clone()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// `true` when the host lets scripts touch files.
    pub fn files_allowed(&self) -> bool {
        self.limits.files
    }

    /// Names of the functions defined by scripts.
    pub fn user_functions(&self) -> Vec<String> {
        let mut v: Vec<String> = self.funcs.keys().cloned().collect();
        v.sort();
        v
    }

    /// Defines a function from source text such as `function y = f(x) ... end`.
    pub fn define_functions(&mut self, src: &str) -> Result<()> {
        let program = parse(src)?;
        self.hoist(&program.stmts);
        Ok(())
    }

    /// Writes text as output.
    pub fn print(&mut self, s: &str) {
        if self.out_chars > self.limits.max_output {
            return;
        }
        self.out_chars += s.len();
        if self.out_chars > self.limits.max_output {
            self.events
                .push(Event::Text("\n[output truncated]\n".into()));
            return;
        }
        match self.events.last_mut() {
            Some(Event::Text(t)) => t.push_str(s),
            _ => self.events.push(Event::Text(s.to_string())),
        }
    }

    /// Emits a warning.
    pub fn warn(&mut self, s: impl Into<String>) {
        self.events.push(Event::Warning(s.into()));
    }

    /// Pushes a raw event.
    pub fn emit(&mut self, e: Event) {
        self.events.push(e);
    }

    /// Fails when the native stack in use passes the limit, so deep expressions and runaway
    /// recursion end in an error instead of a crash.
    fn guard_stack(&mut self) -> Result<()> {
        let marker = 0u8;
        let here = std::hint::black_box(&marker) as *const u8 as usize;
        if self.stack_base == 0 || here > self.stack_base || self.stack_base - here > (1 << 30) {
            self.stack_base = here;
        }
        if self.stack_base - here > self.limits.max_stack {
            return err("Abaco:limit", "the expression or the chain of calls is nested too deeply; simplify it or split it up");
        }
        Ok(())
    }

    /// Counts `n` steps against the limit and checks for cancellation.
    pub fn charge(&mut self, n: u64) -> Result<()> {
        self.steps = self.steps.saturating_add(n);
        if self.steps > self.limits.max_steps {
            return err(
                "Abaco:limit",
                format!(
                    "the script used more than {} steps; stop it or raise the limit",
                    self.limits.max_steps
                ),
            );
        }
        if self.steps % 256 < n.max(1) && self.cancel.load(Ordering::Relaxed) {
            return err("Abaco:cancelled", "stopped");
        }
        Ok(())
    }

    /// Checks the cancel flag now.
    pub fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            err("Abaco:cancelled", "stopped")
        } else {
            Ok(())
        }
    }

    /// `nargin` of the running function.
    pub fn nargin(&self) -> usize {
        self.frames.last().map_or(0, |f| f.nargin)
    }

    /// `nargout` of the running function.
    pub fn nargout(&self) -> usize {
        self.frames.last().map_or(0, |f| f.nargout)
    }

    /// `true` when a user function of that name exists.
    pub fn has_function(&self, name: &str) -> bool {
        self.funcs.contains_key(name)
    }

    // ---------- statements ----------

    fn hoist(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            if let StmtKind::Func(f) = &s.kind {
                self.funcs.insert(f.name.clone(), f.clone());
            }
        }
    }

    fn exec_program(&mut self, p: &Program) -> Result<()> {
        self.hoist(&p.stmts);
        match self.exec_block(&p.stmts)? {
            Flow::Normal | Flow::Return | Flow::Break | Flow::Continue => Ok(()),
        }
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Flow> {
        for s in stmts {
            match self.exec(s)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec(&mut self, s: &Stmt) -> Result<Flow> {
        self.charge(1)?;
        self.guard_stack()?;
        let flow = self.exec_inner(s).map_err(|e| e.or_span(s.span));
        for rcond in crate::linalg::take_conditioning() {
            self.warn(format!(
                "Matrix is close to singular or badly scaled. Results may be inaccurate. RCOND = {rcond:.6e}."
            ));
        }
        flow
    }

    fn exec_inner(&mut self, s: &Stmt) -> Result<Flow> {
        match &s.kind {
            StmtKind::Expr(e) => {
                let is_var = matches!(&e.kind, ExprKind::Ident(n) if self.get(n).is_some());
                let vals = self.eval_multi(e, 0)?;
                if let Some(v) = vals.into_iter().next() {
                    if is_var {
                        if s.print {
                            if let ExprKind::Ident(n) = &e.kind {
                                let text = display::show(n, &v, self.format_long);
                                self.print(&text);
                            }
                        }
                    } else {
                        if s.print {
                            let text = display::show("ans", &v, self.format_long);
                            self.print(&text);
                        }
                        self.set("ans", v);
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Assign(targets, value) => {
                self.exec_assign(targets, value, s.print)?;
                Ok(Flow::Normal)
            }
            StmtKind::If(branches, otherwise) => {
                for (cond, body) in branches {
                    let c = self.eval(cond)?;
                    if self.truthy(&c, cond.span)? {
                        return self.exec_block(body);
                    }
                }
                self.exec_block(otherwise)
            }
            StmtKind::While(cond, body) => {
                loop {
                    self.charge(1)?;
                    let c = self.eval(cond)?;
                    if !self.truthy(&c, cond.span)? {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Normal | Flow::Continue => {}
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::For(var, iter, body) => self.exec_for(var, iter, body),
            StmtKind::Switch(subject, cases, otherwise) => {
                let v = self.eval(subject)?;
                for (case, body) in cases {
                    let c = self.eval(case)?;
                    if switch_matches(&v, &c) {
                        return self.exec_block(body);
                    }
                }
                self.exec_block(otherwise)
            }
            StmtKind::Try(body, name, handler) => {
                let depth = self.frames.len();
                let end_len = self.end_stack.len();
                match self.exec_block(body) {
                    Ok(f) => Ok(f),
                    Err(e) if e.id == "Abaco:cancelled" || e.id == "Abaco:limit" => Err(e),
                    Err(e) => {
                        self.frames.truncate(depth);
                        self.end_stack.truncate(end_len);
                        self.last_error = Some(e.clone());
                        if let Some(n) = name {
                            self.set(n, error_struct(&e));
                        }
                        self.exec_block(handler)
                    }
                }
            }
            StmtKind::Break => Ok(Flow::Break),
            StmtKind::Continue => Ok(Flow::Continue),
            StmtKind::Return => Ok(Flow::Return),
            StmtKind::Func(f) => {
                self.funcs.insert(f.name.clone(), f.clone());
                Ok(Flow::Normal)
            }
            StmtKind::Command(name, args) => {
                self.command(name, args)?;
                Ok(Flow::Normal)
            }
            StmtKind::Global(names) => {
                for n in names {
                    if let Some(f) = self.frames.last_mut() {
                        f.globals.insert(n.clone());
                        f.vars.remove(n);
                    }
                    self.globals.entry(n.clone()).or_insert_with(Value::empty);
                }
                Ok(Flow::Normal)
            }
        }
    }

    fn command(&mut self, name: &str, args: &[String]) -> Result<()> {
        match name {
            "format" => {
                self.format_long = args.first().is_some_and(|a| a == "long");
            }
            "clc" => self.events.push(Event::ClearScreen),
            "clear" | "clearvars" => {
                if args.is_empty() || args.first().is_some_and(|a| a == "all") {
                    self.clear_all();
                    if args.first().is_some_and(|a| a == "all") {
                        self.funcs.clear();
                    }
                } else {
                    for a in args {
                        self.remove(a);
                    }
                }
            }
            "close" | "figure" | "hold" | "grid" | "axis" => {
                let argv: Vec<Value> = args.iter().map(|a| Value::text(a.clone())).collect();
                builtins::call(self, name, &argv, 0)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn truthy(&self, v: &Value, span: Span) -> Result<bool> {
        v.is_true().map_err(|e| e.or_span(span))
    }

    fn exec_for(&mut self, var: &Expr, iter: &Expr, body: &[Stmt]) -> Result<Flow> {
        // Ranges iterate lazily so `for i = 1:1e7` needs no array.
        if let ExprKind::Range(a, step, b) = &iter.kind {
            let start = self.eval(a)?.scalar_f64("the start of a range")?;
            let st = match step {
                Some(s) => self.eval(s)?.scalar_f64("the step of a range")?,
                None => 1.0,
            };
            let stop = self.eval(b)?.scalar_f64("the end of a range")?;
            let n = range_count(start, st, stop)?;
            for k in 0..n {
                self.charge(1)?;
                self.assign_to(var, Value::num(start + k as f64 * st))?;
                match self.exec_block(body)? {
                    Flow::Break => break,
                    Flow::Return => return Ok(Flow::Return),
                    Flow::Normal | Flow::Continue => {}
                }
            }
            return Ok(Flow::Normal);
        }
        let v = self.eval(iter)?;
        let (r, c) = v.dims();
        if r == 0 || c == 0 {
            return Ok(Flow::Normal);
        }
        for j in 0..c {
            self.charge(1)?;
            let item = match &v {
                Value::Num(a) => Value::Num(Arr::col(a.column(j))),
                Value::Bool(a) => Value::Bool(Arr::col(a.column(j))),
                Value::Cplx(a) => Value::cplx(Arr::col(a.column(j))),
                Value::Cell(a) => Value::Cell(Arr::col(a.column(j))),
                Value::Str(s) => Value::Str(s.chars().nth(j).map(String::from).unwrap_or_default()),
                other => other.clone(),
            };
            self.assign_to(var, item)?;
            match self.exec_block(body)? {
                Flow::Break => break,
                Flow::Return => return Ok(Flow::Return),
                Flow::Normal | Flow::Continue => {}
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_assign(&mut self, targets: &[Option<Expr>], value: &Expr, print: bool) -> Result<()> {
        let n = targets.len();
        let vals = if n == 1 {
            // `x = f(...)` asks for one output; a cs-list on the right must give exactly one.
            let mut v = self.eval_multi(value, 1)?;
            if v.is_empty() {
                return err(
                    "Abaco:value",
                    "the expression on the right of `=` produced no value",
                );
            }
            v.truncate(1);
            v
        } else {
            let v = self.eval_multi(value, n)?;
            if v.len() < targets.iter().filter(|t| t.is_some()).count().max(1) && v.len() < n {
                // allow fewer only when the missing ones are `~`
                let needed = targets
                    .iter()
                    .rposition(Option::is_some)
                    .map_or(0, |p| p + 1);
                if v.len() < needed {
                    return err(
                        "Abaco:value",
                        format!(
                            "the right side produced {} value{}, but {needed} {} needed",
                            v.len(),
                            if v.len() == 1 { "" } else { "s" },
                            if needed == 1 { "is" } else { "are" }
                        ),
                    );
                }
            }
            v
        };
        for (t, v) in targets.iter().zip(vals) {
            if let Some(t) = t {
                self.assign_to(t, v)?;
                if print {
                    let root = root_name(t);
                    if let Some(name) = root {
                        if let Some(val) = self.get(&name).cloned() {
                            let text = display::show(&name, &val, self.format_long);
                            self.print(&text);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // ---------- assignment ----------

    /// Assigns `rhs` to an lvalue expression.
    pub fn assign_to(&mut self, target: &Expr, rhs: Value) -> Result<()> {
        // flatten the chain
        let mut accs: Vec<&ExprKind> = Vec::new();
        let mut cur = target;
        loop {
            match &cur.kind {
                ExprKind::Ident(_) => break,
                ExprKind::Index(b, _)
                | ExprKind::CellIndex(b, _)
                | ExprKind::Field(b, _)
                | ExprKind::DynField(b, _) => {
                    accs.push(&cur.kind);
                    cur = b;
                }
                _ => {
                    return err("Abaco:assign", "invalid assignment target")
                        .map_err(|e| e.or_span(target.span))
                }
            }
        }
        accs.reverse();
        let ExprKind::Ident(name) = &cur.kind else {
            return err("Abaco:assign", "invalid assignment target");
        };
        if accs.is_empty() {
            if self.funcs.contains_key(name) && false {
                return err("Abaco:name", format!("`{name}` is a function"));
            }
            self.set(name, rhs);
            return Ok(());
        }
        // resolve index arguments with `end` bound to the current shape
        let mut resolved: Vec<RAcc> = Vec::with_capacity(accs.len());
        let mut cur_val: Option<Value> = self.get(name).cloned_dims_only();
        for a in &accs {
            match a {
                ExprKind::Index(_, args) => {
                    let dims = cur_val.as_ref().map_or((0, 0), Value::dims);
                    let vals = self.eval_args_with_end(args, dims)?;
                    cur_val = None;
                    resolved.push(RAcc::Paren(vals));
                }
                ExprKind::CellIndex(_, args) => {
                    let dims = cur_val.as_ref().map_or((0, 0), Value::dims);
                    let vals = self.eval_args_with_end(args, dims)?;
                    cur_val = None;
                    resolved.push(RAcc::Brace(vals));
                }
                ExprKind::Field(_, f) => {
                    cur_val = None;
                    resolved.push(RAcc::Field(f.clone()));
                }
                ExprKind::DynField(_, e) => {
                    let n = self.eval(e)?;
                    let n = n.as_str("a dynamic field name")?.to_string();
                    cur_val = None;
                    resolved.push(RAcc::Field(n));
                }
                _ => {}
            }
        }
        // take the variable out, update it in place, put it back
        let global = self.frames.last().is_some_and(|f| f.globals.contains(name));
        let mut v = if global {
            self.globals.remove(name)
        } else {
            self.frames.last_mut().and_then(|f| f.vars.remove(name))
        }
        .unwrap_or_else(Value::empty);
        let r = set_path(&mut v, &resolved, rhs, true);
        if global {
            self.globals.insert(name.clone(), v);
        } else if let Some(f) = self.frames.last_mut() {
            f.vars.insert(name.clone(), v);
        }
        r
    }

    // ---------- expressions ----------

    /// Evaluates an expression to exactly one value.
    pub fn eval(&mut self, e: &Expr) -> Result<Value> {
        self.charge(1)?;
        let mut v = self.eval_multi_inner(e, 1)?;
        match v.len() {
            1 => Ok(v.remove(0)),
            0 => err("Abaco:value", "this expression produced no value")
                .map_err(|x| x.or_span(e.span)),
            n => {
                // a cs-list used where one value is needed takes the first, as MATLAB does for `x = c{:}`
                v.truncate(1);
                let _ = n;
                Ok(v.remove(0))
            }
        }
    }

    /// Evaluates an expression that may produce several values (calls, `c{:}`).
    pub fn eval_multi(&mut self, e: &Expr, nargout: usize) -> Result<Vec<Value>> {
        self.charge(1)?;
        self.eval_multi_inner(e, nargout)
    }

    fn eval_multi_inner(&mut self, e: &Expr, nargout: usize) -> Result<Vec<Value>> {
        self.guard_stack().map_err(|x| x.or_span(e.span))?;
        let r = self.eval_kind(e, nargout);
        r.map_err(|x| x.or_span(e.span))
    }

    fn eval_kind(&mut self, e: &Expr, nargout: usize) -> Result<Vec<Value>> {
        match &e.kind {
            ExprKind::Num(x) => Ok(vec![Value::num(*x)]),
            ExprKind::Imag(x) => Ok(vec![Value::cscalar(num_complex::Complex64::new(0.0, *x))]),
            ExprKind::Str(s) => Ok(vec![Value::Str(s.clone())]),
            ExprKind::Ident(name) => self.ident(name, nargout),
            ExprKind::Colon => Ok(vec![Value::text(":")]),
            ExprKind::End => match self.end_stack.last() {
                Some(&(rows, cols, pos_nargs)) => {
                    let (pos, n) = (pos_nargs / 1000, pos_nargs % 1000);
                    let v = if n <= 1 {
                        rows * cols
                    } else if pos == 0 {
                        rows
                    } else if pos == 1 {
                        cols
                    } else {
                        1
                    };
                    Ok(vec![Value::num(v as f64)])
                }
                None => err("Abaco:syntax", "`end` is only valid inside an index"),
            },
            ExprKind::Tilde => err(
                "Abaco:syntax",
                "`~` is only valid in the outputs of `[~, k] = f(x)`",
            ),
            ExprKind::Unary(op, a) => {
                let v = self.eval(a)?;
                Ok(vec![ops::unary(*op, &v)?])
            }
            ExprKind::Binary(op, a, b) => {
                let x = self.eval(a)?;
                let y = self.eval(b)?;
                Ok(vec![ops::binary(*op, &x, &y)?])
            }
            ExprKind::ShortAnd(a, b) => {
                let x = self.eval(a)?;
                if !self.truthy(&x, a.span)? {
                    return Ok(vec![Value::boolean(false)]);
                }
                let y = self.eval(b)?;
                Ok(vec![Value::boolean(self.truthy(&y, b.span)?)])
            }
            ExprKind::ShortOr(a, b) => {
                let x = self.eval(a)?;
                if self.truthy(&x, a.span)? {
                    return Ok(vec![Value::boolean(true)]);
                }
                let y = self.eval(b)?;
                Ok(vec![Value::boolean(self.truthy(&y, b.span)?)])
            }
            ExprKind::Transpose(a, conj) => {
                let v = self.eval(a)?;
                Ok(vec![ops::transpose(&v, *conj)?])
            }
            ExprKind::Range(a, step, b) => {
                let start = self.eval(a)?.scalar_f64("the start of a range")?;
                let st = match step {
                    Some(s) => self.eval(s)?.scalar_f64("the step of a range")?,
                    None => 1.0,
                };
                let stop = self.eval(b)?.scalar_f64("the end of a range")?;
                Ok(vec![make_range(start, st, stop)?])
            }
            ExprKind::Matrix(rows) => Ok(vec![self.matrix(rows)?]),
            ExprKind::CellLit(rows) => Ok(vec![self.cell_literal(rows)?]),
            ExprKind::Anon(params, body, src) => {
                let mut names = BTreeMap::new();
                collect_idents(body, &mut names);
                let mut captured = BTreeMap::new();
                for n in names.keys() {
                    if params.contains(n) {
                        continue;
                    }
                    if let Some(v) = self.get(n) {
                        captured.insert(n.clone(), v.clone());
                    }
                }
                Ok(vec![Value::Fn(Arc::new(FnValue::Anon {
                    params: params.clone(),
                    body: Arc::new((**body).clone()),
                    captured,
                    src: src.clone(),
                }))])
            }
            ExprKind::Handle(name) => Ok(vec![Value::Fn(Arc::new(FnValue::Named(name.clone())))]),
            ExprKind::Field(base, name) => {
                let v = self.eval(base)?;
                Ok(vec![field_get(&v, name)?])
            }
            ExprKind::DynField(base, ne) => {
                let v = self.eval(base)?;
                let n = self.eval(ne)?;
                Ok(vec![field_get(&v, n.as_str("a dynamic field name")?)?])
            }
            ExprKind::Index(base, args) => self.index_or_call(base, args, nargout, e.span),
            ExprKind::CellIndex(base, args) => {
                let v = self.eval(base)?;
                let Value::Cell(c) = &v else {
                    return err(
                        "Abaco:type",
                        format!("`{{}}` indexing needs a cell array, got {}", v.describe()),
                    );
                };
                let vals = self.eval_args_with_end(args, (c.r, c.c))?;
                let sels = self.sels(&vals, (c.r, c.c))?;
                let picked = match sels.as_slice() {
                    [s] => index::get1(c, s)?,
                    [r, cc] => index::get2(c, r, cc)?,
                    _ => return err("Abaco:index", "cell indexing takes one or two indices"),
                };
                Ok(picked.d)
            }
        }
    }

    fn ident(&mut self, name: &str, nargout: usize) -> Result<Vec<Value>> {
        if let Some(v) = self.get(name) {
            return Ok(vec![v.clone()]);
        }
        match name {
            "nargin" if self.frames.len() > 1 => return Ok(vec![Value::num(self.nargin() as f64)]),
            "nargout" if self.frames.len() > 1 => {
                return Ok(vec![Value::num(self.nargout() as f64)])
            }
            _ => {}
        }
        self.call_function(name, Vec::new(), nargout).map_err(|e| {
            if e.id == "Abaco:undefined" {
                self.undefined_message(name)
            } else {
                e
            }
        })
    }

    fn undefined_message(&self, name: &str) -> Error {
        let mut names: Vec<String> = self.funcs.keys().cloned().collect();
        if let Some(f) = self.frames.last() {
            names.extend(f.vars.keys().cloned());
        }
        names.extend(builtins::names().into_iter().map(String::from));
        let hint = nearest(name, names.iter().map(String::as_str));
        let msg = match hint {
            Some(h) => format!("`{name}` is not defined; did you mean `{h}`?"),
            None => format!("`{name}` is not defined"),
        };
        Error::new("Abaco:undefined", msg)
    }

    fn matrix(&mut self, rows: &[Vec<Expr>]) -> Result<Value> {
        let mut row_vals: Vec<Value> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut items: Vec<Value> = Vec::with_capacity(row.len());
            for e in row {
                items.extend(self.eval_multi(e, 1)?);
            }
            self.charge(items.len() as u64)?;
            row_vals.push(index::concat(&items, true)?);
        }
        if row_vals.is_empty() {
            return Ok(Value::empty());
        }
        index::concat(&row_vals, false)
    }

    fn cell_literal(&mut self, rows: &[Vec<Expr>]) -> Result<Value> {
        let mut row_vals: Vec<Arr<Value>> = Vec::new();
        for row in rows {
            let mut items: Vec<Value> = Vec::new();
            for e in row {
                for v in self.eval_multi(e, 1)? {
                    items.push(v);
                }
            }
            row_vals.push(Arr::row(items));
        }
        // a nested cell inside braces stays nested; rows must have equal length
        let c = row_vals.first().map_or(0, |r| r.c);
        if row_vals.iter().any(|r| r.c != c) {
            return err(
                "Abaco:size",
                "every row of a cell array needs the same number of elements",
            );
        }
        let r = row_vals.len();
        let mut d = Vec::with_capacity(r * c);
        for j in 0..c {
            for row in &row_vals {
                if let Some(x) = row.d.get(j) {
                    d.push(x.clone());
                }
            }
        }
        Ok(Value::Cell(Arr {
            r,
            c: if r == 0 { 0 } else { c },
            d,
        }))
    }

    /// Evaluates index arguments with `end` bound to `dims`.
    fn eval_args_with_end(&mut self, args: &[Expr], dims: (usize, usize)) -> Result<Vec<Value>> {
        let n = args.len();
        let mut out = Vec::with_capacity(n);
        for (k, a) in args.iter().enumerate() {
            self.end_stack.push((dims.0, dims.1, k * 1000 + n));
            let r = self.eval_multi(a, 1);
            self.end_stack.pop();
            out.extend(r?);
        }
        Ok(out)
    }

    fn sels(&self, vals: &[Value], dims: (usize, usize)) -> Result<Vec<Sel>> {
        let n = vals.len();
        vals.iter()
            .enumerate()
            .map(|(k, v)| {
                let len = if n <= 1 {
                    dims.0 * dims.1
                } else if k == 0 {
                    dims.0
                } else if k == 1 {
                    dims.1
                } else {
                    1
                };
                Sel::from_value(v, len)
            })
            .collect()
    }

    fn index_or_call(
        &mut self,
        base: &Expr,
        args: &[Expr],
        nargout: usize,
        span: Span,
    ) -> Result<Vec<Value>> {
        // `name(args)`: a variable is indexed; otherwise it is a call
        if let ExprKind::Ident(name) = &base.kind {
            if self.get(name).is_none() {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.extend(self.eval_multi(a, 1)?);
                }
                return self.call_function(name, vals, nargout).map_err(|e| {
                    let e = if e.id == "Abaco:undefined" {
                        self.undefined_message(name)
                    } else {
                        e
                    };
                    e.or_span(span)
                });
            }
        }
        let v = self.eval(base)?;
        if let Value::Fn(f) = &v {
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                vals.extend(self.eval_multi(a, 1)?);
            }
            return self.call_handle(f, vals, nargout);
        }
        let dims = v.dims();
        let vals = self.eval_args_with_end(args, dims)?;
        let sels = self.sels(&vals, dims)?;
        Ok(vec![index::index_value(&v, &sels)?])
    }

    // ---------- calls ----------

    /// Calls a function by name: a user function first, then a built-in.
    pub fn call_function(
        &mut self,
        name: &str,
        args: Vec<Value>,
        nargout: usize,
    ) -> Result<Vec<Value>> {
        if let Some(f) = self.funcs.get(name).cloned() {
            return self.call_user(&f, args, nargout);
        }
        if let Some(r) = builtins::call(self, name, &args, nargout)? {
            return Ok(r);
        }
        if let Some(path) = self.find_m_file(name) {
            return self.call_m_file(name, &path, args, nargout);
        }
        err("Abaco:undefined", format!("`{name}` is not defined"))
    }

    // ---------- `.m` files on the search path ----------

    /// The folders searched for `name.m` after the current folder.
    pub fn search_path(&self) -> &[PathBuf] {
        &self.search_path
    }

    /// Adds a folder to the search path (at the front, or the end).
    pub fn add_path(&mut self, dir: PathBuf, front: bool) {
        self.forget_file_funcs();
        self.search_path.retain(|d| *d != dir);
        if front {
            self.search_path.insert(0, dir);
        } else {
            self.search_path.push(dir);
        }
    }

    /// Removes a folder from the search path; `false` when it was not there.
    pub fn remove_path(&mut self, dir: &Path) -> bool {
        self.forget_file_funcs();
        let before = self.search_path.len();
        self.search_path.retain(|d| d != dir);
        self.search_path.len() != before
    }

    /// `name.m` in the current folder or on the search path, when the host allows files.
    pub fn find_m_file(&self, name: &str) -> Option<PathBuf> {
        let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && name.len() <= 63;
        if !self.limits.files || !valid {
            return None;
        }
        let file = format!("{name}.m");
        std::iter::once(PathBuf::from("."))
            .chain(self.search_path.iter().cloned())
            .map(|d| d.join(&file))
            .find(|p| p.is_file())
    }

    fn read_m_file(path: &Path) -> Result<String> {
        const MAX_M_BYTES: u64 = 8 * 1024 * 1024;
        let shown = path.display();
        let meta = std::fs::metadata(path)
            .map_err(|e| Error::new("Abaco:file", format!("cannot read `{shown}`: {e}")))?;
        if meta.len() > MAX_M_BYTES {
            return err("Abaco:limit", format!("`{shown}` is larger than 8 MB"));
        }
        let bytes = std::fs::read(path)
            .map_err(|e| Error::new("Abaco:file", format!("cannot read `{shown}`: {e}")))?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
    }

    /// Parses a file, naming the file and the place in it when the syntax is wrong (the span of
    /// the error belongs to that file, not to the source being run).
    fn parse_file(path: &Path, text: &str) -> Result<Program> {
        parse(text).map_err(|e| match e.span {
            Some(sp) => {
                let upto = text.get(..sp.start.min(text.len())).unwrap_or("");
                let line = upto.matches('\n').count() + 1;
                let col = upto.len() - upto.rfind('\n').map_or(0, |i| i + 1) + 1;
                Error::new(
                    &e.id,
                    format!(
                        "{} (line {line}, column {col}): {}",
                        path.display(),
                        e.message
                    ),
                )
            }
            None => Error::new(&e.id, format!("{}: {}", path.display(), e.message)),
        })
    }

    /// Runs a script file in the current workspace.
    pub fn run_script_file(&mut self, path: &Path) -> Result<()> {
        if !self.limits.files {
            return err(
                "Abaco:files",
                "this run is not allowed to read or write files",
            );
        }
        let text = Self::read_m_file(path)?;
        let program = Self::parse_file(path, &text)?;
        self.exec_program(&program)
    }

    /// Calls `name` from `name.m`: a function file (its first function is `name`, the others are
    /// available once it is loaded) or a script, which runs in the caller's workspace.
    fn call_m_file(
        &mut self,
        name: &str,
        path: &Path,
        args: Vec<Value>,
        nargout: usize,
    ) -> Result<Vec<Value>> {
        let text = Self::read_m_file(path)?;
        let program = Self::parse_file(path, &text)?;
        let first = program.stmts.iter().find_map(|s| match &s.kind {
            StmtKind::Func(f) => Some(f.clone()),
            _ => None,
        });
        let is_function_file = matches!(
            program.stmts.first().map(|s| &s.kind),
            Some(StmtKind::Func(_))
        );
        if !is_function_file {
            if !args.is_empty() || nargout > 0 {
                return err(
                    "Abaco:call",
                    format!(
                        "`{name}` is a script file ({}); it takes no arguments and returns nothing",
                        path.display()
                    ),
                );
            }
            self.exec_program(&program)?;
            return Ok(vec![]);
        }
        let Some(main) = first else {
            return err(
                "Abaco:call",
                format!("`{}` defines no function", path.display()),
            );
        };
        for s in &program.stmts {
            if let StmtKind::Func(f) = &s.kind {
                if !self.funcs.contains_key(&f.name) {
                    self.funcs.insert(f.name.clone(), f.clone());
                    self.file_funcs.insert(f.name.clone());
                }
            }
        }
        // The file's name is the function's name, whatever the first line says.
        let mut def = (*main).clone();
        def.name = name.to_string();
        let def = Arc::new(def);
        self.funcs.insert(name.to_string(), def.clone());
        self.file_funcs.insert(name.to_string());
        self.call_user(&def, args, nargout)
    }

    /// Calls a function handle.
    pub fn call_handle(
        &mut self,
        f: &FnValue,
        args: Vec<Value>,
        nargout: usize,
    ) -> Result<Vec<Value>> {
        match f {
            FnValue::Named(n) => self.call_function(n, args, nargout),
            FnValue::Anon {
                params,
                body,
                captured,
                ..
            } => {
                if self.frames.len() >= self.limits.max_call_depth {
                    return err(
                        "Abaco:limit",
                        "functions are nested too deeply (runaway recursion?)",
                    );
                }
                let mut frame = Frame::new();
                for (k, v) in captured {
                    frame.vars.insert(k.clone(), v.clone());
                }
                let has_varargin = params.last().is_some_and(|p| p == "varargin");
                let fixed = if has_varargin {
                    params.len() - 1
                } else {
                    params.len()
                };
                if args.len() > fixed && !has_varargin {
                    return err(
                        "Abaco:arity",
                        format!(
                            "this function handle takes {} argument{}, got {}",
                            params.len(),
                            if params.len() == 1 { "" } else { "s" },
                            args.len()
                        ),
                    );
                }
                let nargs = args.len();
                let mut it = args.into_iter();
                for p in params.iter().take(fixed) {
                    if let Some(v) = it.next() {
                        if p != "~" {
                            frame.vars.insert(p.clone(), v);
                        }
                    }
                }
                if has_varargin {
                    frame
                        .vars
                        .insert("varargin".into(), Value::Cell(Arr::row(it.collect())));
                }
                frame.nargin = nargs;
                frame.nargout = nargout;
                self.frames.push(frame);
                let saved_end = std::mem::take(&mut self.end_stack);
                let r = self.eval_multi(body, nargout);
                self.end_stack = saved_end;
                self.frames.pop();
                r
            }
        }
    }

    /// Calls any callable value: a handle or the name of a function.
    pub fn call_value(
        &mut self,
        f: &Value,
        args: Vec<Value>,
        nargout: usize,
    ) -> Result<Vec<Value>> {
        match f {
            Value::Fn(h) => self.call_handle(h, args, nargout),
            Value::Str(s) => self.call_function(s, args, nargout),
            other => err(
                "Abaco:type",
                format!(
                    "expected a function handle or a function name, got {}",
                    other.describe()
                ),
            ),
        }
    }

    /// Calls a callable and returns its first output.
    pub fn call1(&mut self, f: &Value, args: Vec<Value>) -> Result<Value> {
        let mut r = self.call_value(f, args, 1)?;
        if r.is_empty() {
            return err("Abaco:value", "the function returned no value");
        }
        Ok(r.remove(0))
    }

    fn call_user(
        &mut self,
        f: &Arc<FuncDef>,
        args: Vec<Value>,
        nargout: usize,
    ) -> Result<Vec<Value>> {
        if self.frames.len() >= self.limits.max_call_depth {
            return err(
                "Abaco:limit",
                format!(
                    "`{}` is nested more than {} calls deep (runaway recursion?)",
                    f.name, self.limits.max_call_depth
                ),
            );
        }
        let has_varargin = f.params.last().is_some_and(|p| p == "varargin");
        let fixed = if has_varargin {
            f.params.len() - 1
        } else {
            f.params.len()
        };
        if args.len() > fixed && !has_varargin {
            return err(
                "Abaco:arity",
                format!(
                    "`{}` takes {} input{}, got {}",
                    f.name,
                    f.params.len(),
                    if f.params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            );
        }
        let has_varargout = f.outputs.last().is_some_and(|p| p == "varargout");
        let fixed_out = if has_varargout {
            f.outputs.len() - 1
        } else {
            f.outputs.len()
        };
        if nargout > fixed_out
            && !has_varargout
            && !(nargout == 1 && f.outputs.is_empty())
            && nargout > 1
        {
            return err(
                "Abaco:arity",
                format!(
                    "`{}` returns {} output{}, but {nargout} were requested",
                    f.name,
                    f.outputs.len(),
                    if f.outputs.len() == 1 { "" } else { "s" }
                ),
            );
        }
        let mut frame = Frame::new();
        frame.nargin = args.len();
        frame.nargout = nargout;
        let mut it = args.into_iter();
        for p in f.params.iter().take(fixed) {
            if let Some(v) = it.next() {
                if p != "~" {
                    frame.vars.insert(p.clone(), v);
                }
            }
        }
        if has_varargin {
            frame
                .vars
                .insert("varargin".into(), Value::Cell(Arr::row(it.collect())));
        }
        self.frames.push(frame);
        let saved_end = std::mem::take(&mut self.end_stack);
        let r = self.exec_block(&f.body);
        self.end_stack = saved_end;
        let frame = self.frames.pop();
        match r {
            Ok(_) => {}
            Err(mut e) => {
                e.trace.push(f.name.clone());
                return Err(e);
            }
        }
        let Some(mut frame) = frame else {
            return Ok(Vec::new());
        };
        let mut outs = Vec::new();
        let want = nargout
            .max(1)
            .min(if has_varargout { usize::MAX } else { fixed_out });
        for (k, name) in f.outputs.iter().enumerate().take(fixed_out) {
            if k >= want {
                break;
            }
            match frame.vars.remove(name) {
                Some(v) => outs.push(v),
                None => {
                    if k == 0 && nargout == 0 {
                        break;
                    }
                    return err(
                        "Abaco:output",
                        format!("`{}` did not set its output `{name}`", f.name),
                    );
                }
            }
        }
        if has_varargout && outs.len() == fixed_out {
            if let Some(Value::Cell(c)) = frame.vars.remove("varargout") {
                for v in c.d {
                    if outs.len() >= nargout.max(1) {
                        break;
                    }
                    outs.push(v);
                }
            }
        }
        Ok(outs)
    }
}

enum RAcc {
    Paren(Vec<Value>),
    Brace(Vec<Value>),
    Field(String),
}

trait ClonedDims {
    fn cloned_dims_only(self) -> Option<Value>;
}

impl ClonedDims for Option<&Value> {
    /// A placeholder value with the same shape, which is all `end` needs.
    fn cloned_dims_only(self) -> Option<Value> {
        self.map(|v| {
            let (r, c) = v.dims();
            Value::Num(Arr {
                r,
                c,
                d: Vec::new(),
            })
        })
    }
}

fn dims_sel(v: &Value, vals: &[Value]) -> Result<Vec<Sel>> {
    let dims = v.dims();
    let n = vals.len();
    vals.iter()
        .enumerate()
        .map(|(k, x)| {
            let len = if n <= 1 {
                dims.0 * dims.1
            } else if k == 0 {
                dims.0
            } else if k == 1 {
                dims.1
            } else {
                1
            };
            Sel::from_value(x, len)
        })
        .collect()
}

fn is_deletion(rhs: &Value) -> bool {
    matches!(rhs, Value::Num(a) if a.r == 0 && a.c == 0)
}

fn set_path(target: &mut Value, accs: &[RAcc], rhs: Value, top: bool) -> Result<()> {
    let Some((acc, rest)) = accs.split_first() else {
        *target = rhs;
        return Ok(());
    };
    match acc {
        RAcc::Field(name) => {
            if target.is_empty() && !matches!(target, Value::Struct(_)) {
                *target = Value::Struct(Box::default());
            }
            let Value::Struct(s) = target else {
                return err(
                    "Abaco:type",
                    format!(
                        "cannot set the field `{name}` of {}; it is not a struct",
                        target.describe()
                    ),
                );
            };
            if rest.is_empty() {
                s.set(name, rhs);
                return Ok(());
            }
            let mut inner = s.get(name).cloned().unwrap_or_else(Value::empty);
            // detach the field while it is edited so large values are not copied twice
            s.remove(name);
            let r = set_path(&mut inner, rest, rhs, false);
            s.set(name, inner);
            r
        }
        RAcc::Brace(args) => {
            if target.is_empty() && !matches!(target, Value::Cell(_)) {
                *target = Value::Cell(Arr::empty());
            }
            let Value::Cell(c) = target else {
                return err(
                    "Abaco:type",
                    format!(
                        "`{{}}` assignment needs a cell array, got {}",
                        target.describe()
                    ),
                );
            };
            let sels = dims_sel(
                &Value::Cell(Arr {
                    r: c.r,
                    c: c.c,
                    d: Vec::new(),
                }),
                args,
            )?;
            if rest.is_empty() {
                let one = Arr::scalar(rhs);
                return match sels.as_slice() {
                    [s] => index::set1(c, s, &one, &Value::empty()),
                    [r, cc] => index::set2(c, r, cc, &one, &Value::empty()),
                    _ => err("Abaco:index", "cell indexing takes one or two indices"),
                };
            }
            // nested: c{i}.f = v, c{i}(j) = v
            let pos = match sels.as_slice() {
                [s] => s.idx.first().copied(),
                [r, cc] => match (r.idx.first(), cc.idx.first()) {
                    (Some(&i), Some(&j)) => {
                        index::resize(c, i + 1, j + 1, &Value::empty())?;
                        Some(j * c.r + i)
                    }
                    _ => None,
                },
                _ => None,
            };
            let Some(p) = pos else {
                return err("Abaco:index", "invalid cell index");
            };
            if p >= c.d.len() {
                if c.d.is_empty() || c.r == 1 {
                    index::resize(c, 1, p + 1, &Value::empty())?;
                } else if c.c == 1 {
                    index::resize(c, p + 1, 1, &Value::empty())?;
                } else {
                    return err("Abaco:index", "index is out of range for this cell array");
                }
            }
            let Some(slot) = c.d.get_mut(p) else {
                return err("Abaco:index", "invalid cell index");
            };
            set_path(slot, rest, rhs, false)
        }
        RAcc::Paren(args) => {
            if !rest.is_empty() {
                return err(
                    "Abaco:type",
                    "struct arrays are not supported; use a cell array of structs, `c{i}.name`",
                );
            }
            let sels = dims_sel(target, args)?;
            if top && is_deletion(&rhs) {
                return index::delete_value(target, &sels);
            }
            index::assign_value(target, &sels, &rhs)
        }
    }
}

fn field_get(v: &Value, name: &str) -> Result<Value> {
    match v {
        Value::Struct(s) => s.get(name).cloned().ok_or_else(|| {
            let names: Vec<&str> = s.fields.iter().map(|(n, _)| n.as_str()).collect();
            let hint = nearest(name, names.iter().copied());
            Error::new(
                "Abaco:field",
                match hint {
                    Some(h) => format!("the struct has no field `{name}`; did you mean `{h}`?"),
                    None => format!("the struct has no field `{name}`"),
                },
            )
        }),
        other => err(
            "Abaco:type",
            format!(
                "cannot read the field `{name}` of {}; it is not a struct",
                other.describe()
            ),
        ),
    }
}

fn root_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Ident(n) => Some(n.clone()),
        ExprKind::Index(b, _)
        | ExprKind::CellIndex(b, _)
        | ExprKind::Field(b, _)
        | ExprKind::DynField(b, _) => root_name(b),
        _ => None,
    }
}

/// The identifiers an expression uses.
pub fn collect_idents(e: &Expr, out: &mut BTreeMap<String, ()>) {
    match &e.kind {
        ExprKind::Ident(n) | ExprKind::Handle(n) => {
            out.insert(n.clone(), ());
        }
        ExprKind::Unary(_, a) | ExprKind::Transpose(a, _) | ExprKind::Field(a, _) => {
            collect_idents(a, out)
        }
        ExprKind::Binary(_, a, b)
        | ExprKind::ShortAnd(a, b)
        | ExprKind::ShortOr(a, b)
        | ExprKind::DynField(a, b) => {
            collect_idents(a, out);
            collect_idents(b, out);
        }
        ExprKind::Index(b, args) | ExprKind::CellIndex(b, args) => {
            collect_idents(b, out);
            for a in args {
                collect_idents(a, out);
            }
        }
        ExprKind::Range(a, s, b) => {
            collect_idents(a, out);
            if let Some(s) = s {
                collect_idents(s, out);
            }
            collect_idents(b, out);
        }
        ExprKind::Matrix(rows) | ExprKind::CellLit(rows) => {
            for a in rows.iter().flatten() {
                collect_idents(a, out);
            }
        }
        ExprKind::Anon(_, body, _) => collect_idents(body, out),
        _ => {}
    }
}

fn switch_matches(v: &Value, case: &Value) -> bool {
    match case {
        Value::Cell(c) => c.d.iter().any(|x| switch_matches(v, x)),
        Value::Str(s) => matches!(v, Value::Str(t) if t == s),
        other => match (v, other) {
            (Value::Str(_), _) | (_, Value::Str(_)) => false,
            _ => match (v.to_real(), other.to_real()) {
                (Ok(a), Ok(b)) => a.d.len() == 1 && b.d.len() == 1 && a.d.first() == b.d.first(),
                _ => false,
            },
        },
    }
}

/// The `err` struct handed to a `catch err` block.
pub fn error_struct(e: &Error) -> Value {
    let stack: Vec<Value> = e
        .trace
        .iter()
        .map(|n| Value::structure(vec![("name", Value::text(n.clone()))]))
        .collect();
    Value::structure(vec![
        ("message", Value::text(e.message.clone())),
        ("identifier", Value::text(e.id.clone())),
        ("stack", Value::cell_row(stack)),
    ])
}

/// How many elements `a:step:b` has, with MATLAB's tolerance for rounding.
pub fn range_count(start: f64, step: f64, stop: f64) -> Result<usize> {
    if !(start.is_finite() && step.is_finite() && stop.is_finite()) {
        return err("Abaco:range", "the bounds of a range must be finite");
    }
    if step == 0.0 || (step > 0.0 && start > stop) || (step < 0.0 && start < stop) {
        return Ok(0);
    }
    let n = ((stop - start) / step * (1.0 + 4.0 * f64::EPSILON) + 4.0 * f64::EPSILON).floor();
    if n < 0.0 {
        return Ok(0);
    }
    let n = n + 1.0;
    if n > crate::ops::MAX_ELEMENTS as f64 {
        return err(
            "Abaco:limit",
            format!(
                "the range would have {n} elements; the limit is {}",
                crate::ops::MAX_ELEMENTS
            ),
        );
    }
    Ok(n as usize)
}

/// `start:step:stop` as a row vector.
pub fn make_range(start: f64, step: f64, stop: f64) -> Result<Value> {
    let n = range_count(start, step, stop)?;
    if n == 0 {
        return Ok(Value::Num(Arr {
            r: 1,
            c: 0,
            d: Vec::new(),
        }));
    }
    Ok(Value::row(
        (0..n).map(|k| start + k as f64 * step).collect(),
    ))
}

/// The candidate nearest to `name` (ignoring case, then small typos).
pub fn nearest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    if name.chars().count() > 32 || name.chars().count() < 3 {
        return None;
    }
    let lower = name.to_lowercase();
    let limit = if name.chars().count() <= 4 { 1 } else { 2 };
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        if c == name {
            continue;
        }
        if c.to_lowercase() == lower {
            return Some(c.to_string());
        }
        if let Some(d) = levenshtein(&lower, &c.to_lowercase(), limit) {
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c));
            }
        }
    }
    best.map(|(_, c)| c.to_string())
}

fn levenshtein(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            cur.push(sub.min(del).min(ins));
        }
        prev = cur;
    }
    let d = prev.last().copied().unwrap_or(usize::MAX);
    (d <= limit).then_some(d)
}

// Keep the otherwise unused imports honest.
#[allow(dead_code)]
fn _unused(_: BinOp, _: UnOp, _: StructV) {}
