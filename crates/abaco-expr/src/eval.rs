// SPDX-License-Identifier: MIT OR Apache-2.0
//! The evaluator: environments, user functions and the tree walk.
//!
//! Name resolution for a bare name is: parameter or variable, then the special constants
//! (`pi`, `e`, `tau`, `i`, `j`), then physical constants (`c`, `g0`, `k_B`, ...), then units
//! (`m`, `kg`, `h` for the hour). A function name is looked up among user functions first and
//! built-ins second. A user function sees its parameters and the names it captured when it
//! was defined (a lexical closure); it never sees the caller's variables.

use crate::ast::{free_names, BinOp, Expr, ExprKind, Program, Stmt, UnOp};
use crate::builtins;
use crate::error::{ErrorKind, ExprError, Span};
use crate::limits::Limits;
use crate::ops::{self, err, from_q, map_scalars, Ctx, R};
use crate::parser::{parse_with, KEYWORDS};
use crate::value::{Mat, Value};
use nodalia_units::{constants, Quantity, Unit};
use num_complex::Complex64;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A function defined with `f(x, y) = ...`.
#[derive(Clone, Debug, PartialEq)]
pub struct UserFn {
    /// The function name.
    pub name: String,
    /// Parameter names, in order.
    pub params: Vec<String>,
    /// The body.
    pub body: Expr,
    /// Variables of the defining environment the body uses, captured by value.
    pub captured_vars: BTreeMap<String, Value>,
    /// Functions of the defining environment the body calls.
    pub captured_fns: BTreeMap<String, Arc<UserFn>>,
}

/// The names visible at the top level of a cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Env {
    vars: BTreeMap<String, Value>,
    funcs: BTreeMap<String, Arc<UserFn>>,
}

impl Env {
    /// An empty environment.
    pub fn new() -> Env {
        Env::default()
    }

    /// The variable `name`.
    pub fn var(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    /// The user function `name`.
    pub fn func(&self, name: &str) -> Option<&Arc<UserFn>> {
        self.funcs.get(name)
    }

    /// Defines or replaces a variable.
    pub fn set_var(&mut self, name: impl Into<String>, value: Value) {
        self.vars.insert(name.into(), value);
    }

    /// Defines or replaces a function.
    pub fn set_fn(&mut self, f: Arc<UserFn>) {
        self.funcs.insert(f.name.clone(), f);
    }

    /// Removes a variable and a function of that name.
    pub fn remove(&mut self, name: &str) {
        self.vars.remove(name);
        self.funcs.remove(name);
    }

    /// Variable names in order.
    pub fn var_names(&self) -> impl Iterator<Item = &str> {
        self.vars.keys().map(String::as_str)
    }

    /// Function names in order.
    pub fn fn_names(&self) -> impl Iterator<Item = &str> {
        self.funcs.keys().map(String::as_str)
    }
}

/// What evaluating a cell produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// The value of the last statement (`None` for a function definition or an empty cell).
    pub value: Option<Value>,
    /// Names the cell defined (variables and functions).
    pub defined: Vec<String>,
    /// Warnings (for example an ill-conditioned linear system).
    pub warnings: Vec<String>,
    /// Evaluation steps used.
    pub steps: u64,
}

/// Parses and evaluates `src` against `env` with default limits.
pub fn eval_str(src: &str, env: &mut Env) -> Result<Evaluation, ExprError> {
    eval_str_with(src, env, &Limits::default())
}

/// Parses and evaluates `src` against `env`.
pub fn eval_str_with(src: &str, env: &mut Env, limits: &Limits) -> Result<Evaluation, ExprError> {
    let program = parse_with(src, limits)?;
    eval_program(&program, env, limits)
}

/// Evaluates a parsed cell. All-or-nothing: when a statement fails, the definitions made by
/// the earlier statements of the same cell are undone.
pub fn eval_program(
    program: &Program,
    env: &mut Env,
    limits: &Limits,
) -> Result<Evaluation, ExprError> {
    let mut it = Interp {
        env,
        ctx: Ctx::new(*limits),
        depth: 0,
        call_depth: 0,
        journal: Vec::new(),
    };
    let mut value = None;
    let mut failure = None;
    for stmt in &program.stmts {
        match it.stmt(stmt) {
            Ok(v) => value = v,
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failure {
        it.rollback();
        return Err(e);
    }
    Ok(Evaluation {
        value,
        defined: program.defined_names(),
        warnings: it.ctx.warnings,
        steps: it.ctx.steps,
    })
}

enum Undo {
    Var(String, Option<Value>),
    Fn(String, Option<Arc<UserFn>>),
}

struct Frame {
    f: Arc<UserFn>,
    args: Vec<Value>,
}

enum Resolved {
    Value(Value),
    Unit(Unit),
}

struct Interp<'a> {
    env: &'a mut Env,
    ctx: Ctx,
    depth: usize,
    call_depth: usize,
    journal: Vec<Undo>,
}

fn special_constant(name: &str) -> Option<Value> {
    use std::f64::consts::{E, PI, TAU};
    match name {
        "pi" | "π" => Some(Value::Real(PI)),
        "e" => Some(Value::Real(E)),
        "tau" | "τ" => Some(Value::Real(TAU)),
        "i" | "j" => Some(Value::Complex(Complex64::new(0.0, 1.0))),
        _ => None,
    }
}

const SPECIAL_CONSTANTS: [&str; 7] = ["pi", "π", "e", "tau", "τ", "i", "j"];

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

/// The candidate nearest to `name`, ignoring case first and typos second.
fn nearest<'c>(name: &str, candidates: impl Iterator<Item = &'c str>) -> Option<String> {
    if name.chars().count() > 32 {
        return None;
    }
    let lower = name.to_lowercase();
    let limit = if name.chars().count() <= 3 { 1 } else { 2 };
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        if c == name {
            continue;
        }
        if c.to_lowercase() == lower {
            return Some(c.to_string());
        }
        // Very short names are too close to everything; only a different case counts.
        if name.chars().count() < 3 {
            continue;
        }
        if let Some(d) = levenshtein(&lower, &c.to_lowercase(), limit) {
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, c));
            }
        }
    }
    best.map(|(_, c)| c.to_string())
}

fn with_hint(msg: String, hint: Option<String>) -> String {
    match hint {
        Some(h) => format!("{msg}; did you mean `{h}`?"),
        None => msg,
    }
}

impl Interp<'_> {
    fn rollback(&mut self) {
        while let Some(u) = self.journal.pop() {
            match u {
                Undo::Var(n, Some(v)) => {
                    self.env.vars.insert(n, v);
                }
                Undo::Var(n, None) => {
                    self.env.vars.remove(&n);
                }
                Undo::Fn(n, Some(f)) => {
                    self.env.funcs.insert(n, f);
                }
                Undo::Fn(n, None) => {
                    self.env.funcs.remove(&n);
                }
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) -> R<Option<Value>> {
        match s {
            Stmt::Expr(e) => self.eval(e, None).map(Some),
            Stmt::Assign {
                name,
                name_span,
                value,
            } => {
                let v = self.eval(value, None)?;
                if self.env.funcs.contains_key(name) {
                    return Err(ExprError::at(
                        ErrorKind::Name,
                        *name_span,
                        format!("`{name}` is already a function; choose another name"),
                    ));
                }
                let old = self.env.vars.insert(name.clone(), v.clone());
                self.journal.push(Undo::Var(name.clone(), old));
                Ok(Some(v))
            }
            Stmt::FuncDef {
                name,
                name_span,
                params,
                body,
            } => {
                if builtins::is_builtin(name) {
                    return Err(ExprError::at(
                        ErrorKind::Name,
                        *name_span,
                        format!("`{name}` is a built-in function and cannot be redefined"),
                    ));
                }
                for (k, p) in params.iter().enumerate() {
                    if params.get(..k).is_some_and(|prev| prev.contains(p)) {
                        return Err(ExprError::at(
                            ErrorKind::Name,
                            *name_span,
                            format!("the parameter `{p}` appears twice in `{name}`"),
                        ));
                    }
                }
                let mut bound = params.clone();
                bound.push(name.clone());
                let mut captured_vars = BTreeMap::new();
                let mut captured_fns = BTreeMap::new();
                for n in free_names(body, &bound) {
                    if let Some(v) = self.env.vars.get(&n) {
                        captured_vars.insert(n.clone(), v.clone());
                    }
                    if let Some(f) = self.env.funcs.get(&n) {
                        captured_fns.insert(n, f.clone());
                    }
                }
                let f = Arc::new(UserFn {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    captured_vars,
                    captured_fns,
                });
                let old = self.env.funcs.insert(name.clone(), f);
                self.journal.push(Undo::Fn(name.clone(), old));
                Ok(None)
            }
        }
    }

    // ---------- name resolution ----------

    fn var(&self, name: &str, fr: Option<&Frame>) -> Option<Value> {
        match fr {
            Some(f) => {
                if let Some(i) = f.f.params.iter().position(|p| p == name) {
                    return f.args.get(i).cloned();
                }
                f.f.captured_vars.get(name).cloned()
            }
            None => self.env.vars.get(name).cloned(),
        }
    }

    fn user_fn(&self, name: &str, fr: Option<&Frame>) -> Option<Arc<UserFn>> {
        match fr {
            Some(f) => {
                if f.f.name == name {
                    Some(f.f.clone())
                } else {
                    f.f.captured_fns.get(name).cloned()
                }
            }
            None => self.env.funcs.get(name).cloned(),
        }
    }

    fn resolve(&mut self, name: &str, fr: Option<&Frame>) -> Result<Resolved, ()> {
        if let Some(v) = self.var(name, fr) {
            return Ok(Resolved::Value(v));
        }
        if let Some(v) = special_constant(name) {
            return Ok(Resolved::Value(v));
        }
        if let Some(c) = constants::lookup(name) {
            return match c.quantity() {
                Ok(q) => Ok(Resolved::Value(Value::quantity(q))),
                Err(_) => Err(()),
            };
        }
        match Unit::from_name(name) {
            Ok(u) => Ok(Resolved::Unit(u)),
            Err(_) => Err(()),
        }
    }

    fn unknown_name(&self, name: &str, fr: Option<&Frame>, span: Span) -> ExprError {
        if self.user_fn(name, fr).is_some() || builtins::is_builtin(name) {
            return ExprError::at(
                ErrorKind::Type,
                span,
                format!("`{name}` is a function; call it with arguments, like `{name}(x)`"),
            );
        }
        let mut names: Vec<&str> = Vec::new();
        match fr {
            Some(f) => {
                names.extend(f.f.params.iter().map(String::as_str));
                names.extend(f.f.captured_vars.keys().map(String::as_str));
            }
            None => names.extend(self.env.var_names()),
        }
        names.extend(SPECIAL_CONSTANTS);
        names.extend(constants::CONSTANTS.iter().map(|c| c.name));
        names.extend(KEYWORDS);
        let hint = nearest(name, names.into_iter()).or_else(|| nodalia_units::suggest(name));
        ExprError::at(
            ErrorKind::Name,
            span,
            with_hint(format!("unknown name `{name}`"), hint),
        )
    }

    fn unknown_function(&self, name: &str, fr: Option<&Frame>, span: Span) -> ExprError {
        let mut names: Vec<&str> = builtins::BUILTINS.iter().map(|(n, _, _)| *n).collect();
        match fr {
            Some(f) => {
                names.push(&f.f.name);
                names.extend(f.f.captured_fns.keys().map(String::as_str));
            }
            None => names.extend(self.env.fn_names()),
        }
        let hint = nearest(name, names.into_iter());
        ExprError::at(
            ErrorKind::Name,
            span,
            with_hint(format!("unknown function `{name}`"), hint),
        )
    }

    // ---------- the tree walk ----------

    fn eval(&mut self, e: &Expr, fr: Option<&Frame>) -> R<Value> {
        self.ctx.charge(1)?;
        if self.depth >= self.ctx.limits.max_eval_depth {
            return Err(ExprError::at(
                ErrorKind::Limit,
                e.span,
                format!(
                    "the evaluation is nested deeper than {} levels (check for runaway recursion)",
                    self.ctx.limits.max_eval_depth
                ),
            ));
        }
        self.depth += 1;
        let r = self.eval_kind(e, fr);
        self.depth -= 1;
        r.map_err(|x| x.or_span(e.span))
    }

    fn eval_kind(&mut self, e: &Expr, fr: Option<&Frame>) -> R<Value> {
        // Each arm lives in its own function so this frame, which every level of nesting
        // pays for, stays small (debug builds give every local its own stack slot).
        match &e.kind {
            ExprKind::Number(x) => Ok(Value::Real(*x)),
            ExprKind::Imag(x) => Ok(Value::Complex(Complex64::new(0.0, *x))),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Ident(name) => self.ident(name, fr, e.span),
            ExprKind::Paren(a) => self.eval(a, fr),
            ExprKind::Unary(op, a) => self.unary(*op, a, fr),
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), a, b) => self.logic(*op, a, b, fr),
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b, fr),
            ExprKind::Implicit(a, b) => self.implicit(a, b, fr),
            ExprKind::Call(name, name_span, args) => self.call(name, *name_span, args, e.span, fr),
            ExprKind::Index(base, idx) => self.indexed(base, idx, fr),
            ExprKind::ColonAll => Err(err(
                ErrorKind::Syntax,
                "`:` alone is only allowed inside an index, like `A[1, :]`",
            )),
            ExprKind::Vector(items) => self.vector_literal(items, fr),
            ExprKind::Matrix(rows) => self.matrix_literal(rows, fr),
            ExprKind::Range(a, step, b) => self.range(a, step.as_deref(), b, fr),
            ExprKind::Convert(a, unit) => self.convert(a, unit, fr),
        }
    }

    fn ident(&mut self, name: &str, fr: Option<&Frame>, span: Span) -> R<Value> {
        match self.resolve(name, fr) {
            Ok(Resolved::Value(v)) => {
                match &v {
                    Value::Vector(items) => self.ctx.charge(items.len() as u64)?,
                    Value::Matrix(m) => self.ctx.charge(m.data.len() as u64)?,
                    _ => {}
                }
                Ok(v)
            }
            Ok(Resolved::Unit(u)) => Ok(Value::quantity(Quantity::new(1.0, &u))),
            Err(()) => Err(self.unknown_name(name, fr, span)),
        }
    }

    fn unary(&mut self, op: UnOp, a: &Expr, fr: Option<&Frame>) -> R<Value> {
        let v = self.eval(a, fr)?;
        match op {
            UnOp::Neg => ops::negate(&mut self.ctx, &v),
            UnOp::Pos => {
                if v.is_scalar() || matches!(v, Value::Vector(_) | Value::Matrix(_)) {
                    Ok(v)
                } else {
                    Err(err(
                        ErrorKind::Type,
                        format!("unary `+` needs a number, got {}", v.type_name()),
                    ))
                }
            }
            UnOp::Not => match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                other => Err(err(
                    ErrorKind::Type,
                    format!("`not` needs a boolean, got {}", other.type_name()),
                )),
            },
        }
    }

    fn logic(&mut self, op: BinOp, a: &Expr, b: &Expr, fr: Option<&Frame>) -> R<Value> {
        let left = self.boolean(a, fr, op.symbol())?;
        if matches!(op, BinOp::Or) == left {
            return Ok(Value::Bool(left));
        }
        Ok(Value::Bool(self.boolean(b, fr, op.symbol())?))
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr, fr: Option<&Frame>) -> R<Value> {
        let x = self.eval(a, fr)?;
        let y = self.eval(b, fr)?;
        ops::binary(&mut self.ctx, op, &x, &y)
    }

    fn implicit(&mut self, a: &Expr, b: &Expr, fr: Option<&Frame>) -> R<Value> {
        let x = self.eval(a, fr)?;
        // `25 degC`: a plain number followed by a unit name builds the quantity directly,
        // which is the only way to write a temperature point.
        if let (Value::Real(n), ExprKind::Ident(name)) = (&x, &b.kind) {
            if let Ok(Resolved::Unit(u)) = self.resolve(name, fr) {
                self.ctx.charge(1)?;
                let q = Quantity::new(*n, &u);
                if !q.si_value().is_finite() {
                    return Err(err(ErrorKind::Numeric, "the result is not a finite number"));
                }
                return Ok(Value::quantity(q));
            }
        }
        let y = self.eval(b, fr)?;
        ops::binary(&mut self.ctx, BinOp::Mul, &x, &y)
    }

    fn indexed(&mut self, base: &Expr, idx: &[Expr], fr: Option<&Frame>) -> R<Value> {
        let b = self.eval(base, fr)?;
        self.index(&b, idx, fr)
    }

    fn vector_literal(&mut self, items: &[Expr], fr: Option<&Frame>) -> R<Value> {
        let mut out = Vec::new();
        for it in items {
            let v = self.eval(it, fr)?;
            self.push_flat(&mut out, v)?;
        }
        self.ctx.charge(out.len() as u64)?;
        Ok(Value::Vector(out))
    }

    fn range(&mut self, a: &Expr, step: Option<&Expr>, b: &Expr, fr: Option<&Frame>) -> R<Value> {
        let x = self.eval(a, fr)?;
        let s = match step {
            Some(s) => Some(self.eval(s, fr)?),
            None => None,
        };
        let y = self.eval(b, fr)?;
        builtins::range(&mut self.ctx, &x, s.as_ref(), &y)
    }

    fn convert(&mut self, a: &Expr, unit: &Unit, fr: Option<&Frame>) -> R<Value> {
        let v = self.eval(a, fr)?;
        map_scalars(&mut self.ctx, &v, &mut |x| match x.as_quantity() {
            Some(q) => from_q(q.to(unit)),
            None => Err(err(
                ErrorKind::Type,
                format!("cannot convert {} to a unit", x.type_name()),
            )),
        })
    }

    fn boolean(&mut self, e: &Expr, fr: Option<&Frame>, op: &str) -> R<bool> {
        match self.eval(e, fr)? {
            Value::Bool(b) => Ok(b),
            other => Err(ExprError::at(
                ErrorKind::Type,
                e.span,
                format!(
                    "`{op}` needs booleans such as comparisons (`x > 0`), got {}",
                    other.type_name()
                ),
            )),
        }
    }

    /// Appends a scalar, or the scalars of a vector, to a literal under construction.
    fn push_flat(&self, out: &mut Vec<Value>, v: Value) -> R<()> {
        match v {
            Value::Vector(items) => out.extend(items),
            s if s.is_scalar() => out.push(s),
            other => {
                return Err(err(
                    ErrorKind::Type,
                    format!(
                        "a vector or matrix holds numbers and quantities, not {}",
                        other.type_name()
                    ),
                ))
            }
        }
        self.ctx.check_elements(out.len())
    }

    fn matrix_literal(&mut self, rows: &[Vec<Expr>], fr: Option<&Frame>) -> R<Value> {
        let mut data: Vec<Value> = Vec::new();
        let mut cols: Option<usize> = None;
        for (r, row) in rows.iter().enumerate() {
            let mut line = Vec::new();
            for it in row {
                let v = self.eval(it, fr)?;
                self.push_flat(&mut line, v)?;
            }
            match cols {
                None => cols = Some(line.len()),
                Some(c) if c != line.len() => {
                    return Err(ExprError::at(
                        ErrorKind::Index,
                        row.first()
                            .zip(row.last())
                            .map_or(Span::default(), |(a, b)| a.span.to(b.span)),
                        format!(
                            "row {} has {} elements but row 1 has {c}; every row of a matrix needs the same length",
                            r + 1,
                            line.len()
                        ),
                    ))
                }
                Some(_) => {}
            }
            data.extend(line);
            self.ctx.check_elements(data.len())?;
        }
        self.ctx.charge(data.len() as u64)?;
        Ok(Value::Matrix(Mat {
            rows: rows.len(),
            cols: cols.unwrap_or(0),
            data,
        }))
    }

    // ---------- calls ----------

    fn call(
        &mut self,
        name: &str,
        name_span: Span,
        args: &[Expr],
        span: Span,
        fr: Option<&Frame>,
    ) -> R<Value> {
        if name == "if" {
            return self.call_if(args, span, fr);
        }
        let user = self.user_fn(name, fr);
        let is_builtin = user.is_none() && builtins::is_builtin(name);
        if user.is_none() && !is_builtin {
            // `x(3)` with `x` a variable, constant or unit is implicit multiplication.
            if let [arg] = args {
                if let Ok(res) = self.resolve(name, fr) {
                    let lhs = match res {
                        Resolved::Value(v) => v,
                        Resolved::Unit(u) => Value::quantity(Quantity::new(1.0, &u)),
                    };
                    let rhs = self.eval(arg, fr)?;
                    return ops::binary(&mut self.ctx, BinOp::Mul, &lhs, &rhs);
                }
            }
            return Err(self.unknown_function(name, fr, name_span));
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a, fr)?);
        }
        if let Some(f) = user {
            return self.call_user(&f, vals, span);
        }
        match builtins::call(&mut self.ctx, name, &vals) {
            Some(r) => r.map_err(|e| e.or_span(span)),
            None => Err(self.unknown_function(name, fr, name_span)),
        }
    }

    fn call_if(&mut self, args: &[Expr], span: Span, fr: Option<&Frame>) -> R<Value> {
        let [cond, a, b] = args else {
            return Err(ExprError::at(
                ErrorKind::Arity,
                span,
                format!(
                    "`if` takes 3 arguments (condition, then, else), got {}",
                    args.len()
                ),
            ));
        };
        let c = match self.eval(cond, fr)? {
            Value::Bool(b) => b,
            other => {
                return Err(ExprError::at(
                    ErrorKind::Type,
                    cond.span,
                    format!(
                        "the condition of `if` must be a boolean such as `x > 0`, got {}",
                        other.type_name()
                    ),
                ))
            }
        };
        self.eval(if c { a } else { b }, fr)
    }

    fn call_user(&mut self, f: &Arc<UserFn>, args: Vec<Value>, span: Span) -> R<Value> {
        if args.len() != f.params.len() {
            return Err(ExprError::at(
                ErrorKind::Arity,
                span,
                format!(
                    "`{}({})` takes {} argument{}, got {}",
                    f.name,
                    f.params.join(", "),
                    f.params.len(),
                    if f.params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            ));
        }
        if self.call_depth >= self.ctx.limits.max_call_depth {
            return Err(ExprError::at(
                ErrorKind::Limit,
                span,
                format!(
                    "`{}` calls nest deeper than {} levels (runaway recursion?)",
                    f.name, self.ctx.limits.max_call_depth
                ),
            ));
        }
        let frame = Frame { f: f.clone(), args };
        self.call_depth += 1;
        let r = self.eval(&f.body, Some(&frame));
        self.call_depth -= 1;
        r.map_err(|e| {
            // Positions inside the body belong to the definition's source, not this one.
            let message = if e.message.starts_with("in `") {
                e.message
            } else {
                format!("in `{}`: {}", f.name, e.message)
            };
            ExprError {
                kind: e.kind,
                message,
                span: Some(span),
            }
        })
    }

    // ---------- indexing ----------

    fn index(&mut self, base: &Value, idx: &[Expr], fr: Option<&Frame>) -> R<Value> {
        match (base, idx) {
            (Value::Vector(v), [i]) => {
                let sel = self.selector(i, fr, v.len())?;
                match sel {
                    Sel::One(k) => v
                        .get(k)
                        .cloned()
                        .ok_or_else(|| err(ErrorKind::Index, "index out of range")),
                    Sel::Many(ks) => Ok(Value::Vector(
                        ks.iter().filter_map(|k| v.get(*k).cloned()).collect(),
                    )),
                    Sel::All => Ok(base.clone()),
                }
            }
            (Value::Vector(_), _) => Err(err(
                ErrorKind::Index,
                format!("a vector takes one index, got {}", idx.len()),
            )),
            (Value::Matrix(m), [r, c]) => {
                let rs = self.selector(r, fr, m.rows)?;
                let cs = self.selector(c, fr, m.cols)?;
                let rows = rs.indices(m.rows);
                let cols = cs.indices(m.cols);
                self.ctx.charge((rows.len() * cols.len()) as u64)?;
                let pick = |r: usize, c: usize| m.get(r, c).cloned();
                match (&rs, &cs) {
                    (Sel::One(r), Sel::One(c)) => {
                        pick(*r, *c).ok_or_else(|| err(ErrorKind::Index, "index out of range"))
                    }
                    (Sel::One(_), _) | (_, Sel::One(_)) => {
                        let mut out = Vec::new();
                        for r in &rows {
                            for c in &cols {
                                out.extend(pick(*r, *c));
                            }
                        }
                        Ok(Value::Vector(out))
                    }
                    _ => {
                        let mut data = Vec::new();
                        for r in &rows {
                            for c in &cols {
                                data.extend(pick(*r, *c));
                            }
                        }
                        Ok(Value::Matrix(Mat {
                            rows: rows.len(),
                            cols: cols.len(),
                            data,
                        }))
                    }
                }
            }
            (Value::Matrix(_), _) => Err(err(
                ErrorKind::Index,
                format!(
                    "a matrix takes two indices, `A[row, col]`, got {}",
                    idx.len()
                ),
            )),
            (other, _) => Err(err(
                ErrorKind::Type,
                format!(
                    "cannot index {}; only vectors and matrices have elements",
                    other.type_name()
                ),
            )),
        }
    }

    fn selector(&mut self, e: &Expr, fr: Option<&Frame>, len: usize) -> R<Sel> {
        if matches!(e.kind, ExprKind::ColonAll) {
            return Ok(Sel::All);
        }
        let v = self.eval(e, fr)?;
        let one = |x: &Value| -> R<usize> {
            let n = builtins::real_arg(x, "index")?;
            if n.fract() != 0.0 {
                return Err(ExprError::at(
                    ErrorKind::Index,
                    e.span,
                    format!("an index must be a whole number, got {n}"),
                ));
            }
            if n < 1.0 {
                return Err(ExprError::at(
                    ErrorKind::Index,
                    e.span,
                    format!("index {n} is out of range: indices start at 1"),
                ));
            }
            if n > len as f64 {
                return Err(ExprError::at(
                    ErrorKind::Index,
                    e.span,
                    format!("index {n} is out of range: the length is {len}"),
                ));
            }
            Ok(n as usize - 1)
        };
        match &v {
            Value::Vector(items) => {
                self.ctx.charge(items.len() as u64)?;
                let mut out = Vec::with_capacity(items.len());
                for x in items {
                    out.push(one(x)?);
                }
                Ok(Sel::Many(out))
            }
            other => Ok(Sel::One(one(other)?)),
        }
    }
}

/// Which elements an index argument picks along one axis (0-based).
enum Sel {
    One(usize),
    Many(Vec<usize>),
    All,
}

impl Sel {
    fn indices(&self, len: usize) -> Vec<usize> {
        match self {
            Sel::One(k) => vec![*k],
            Sel::Many(ks) => ks.clone(),
            Sel::All => (0..len).collect(),
        }
    }
}
