// SPDX-License-Identifier: MIT OR Apache-2.0
//! The syntax tree. Every node carries its source span.

use crate::error::Span;
use nodalia_units::Unit;
use std::collections::BTreeSet;

/// Binary operators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    /// `.*`
    ElemMul,
    /// `./`
    ElemDiv,
    /// `.^`
    ElemPow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    /// The operator as written.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
            BinOp::ElemMul => ".*",
            BinOp::ElemDiv => "./",
            BinOp::ElemPow => ".^",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }
}

/// Unary operators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
}

/// An expression node.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is in the source.
    pub span: Span,
    /// Height of the subtree (a leaf is 1); bounded by the parser.
    pub depth: u32,
}

/// The kinds of expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A real literal.
    Number(f64),
    /// An imaginary literal such as `3i` (the value is the imaginary part).
    Imag(f64),
    /// `true` or `false`.
    Bool(bool),
    /// A string literal.
    Str(String),
    /// A name: variable, constant or unit.
    Ident(String),
    /// A parenthesised expression.
    Paren(Box<Expr>),
    /// `-x`, `+x`, `not x`.
    Unary(UnOp, Box<Expr>),
    /// `a op b`.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Implicit multiplication by juxtaposition: `2x`, `3(4+5)`, `9.81 m`.
    Implicit(Box<Expr>, Box<Expr>),
    /// `name(args)`.
    Call(String, Span, Vec<Expr>),
    /// `base[i]`, `base[i, j]`.
    Index(Box<Expr>, Vec<Expr>),
    /// `:` as a whole-dimension index, only valid inside `[...]` indexing.
    ColonAll,
    /// `[a, b, c]`.
    Vector(Vec<Expr>),
    /// `[a, b; c, d]` (rows).
    Matrix(Vec<Vec<Expr>>),
    /// `start:stop` or `start:step:stop` (inclusive).
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    /// `expr -> unit` / `expr to unit`.
    Convert(Box<Expr>, Unit),
}

/// A statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `name = expr`.
    Assign {
        name: String,
        name_span: Span,
        value: Expr,
    },
    /// `name(params) = body`.
    FuncDef {
        name: String,
        name_span: Span,
        params: Vec<String>,
        body: Expr,
    },
    /// A bare expression.
    Expr(Expr),
}

/// A parsed cell: zero or more statements separated by `;` or newlines.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// The statements in order.
    pub stmts: Vec<Stmt>,
}

impl Program {
    /// `true` when the source had no statements (blank or only comments).
    pub fn is_empty(&self) -> bool {
        self.stmts.is_empty()
    }

    /// Names (variables and functions) this program defines.
    pub fn defined_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.stmts {
            let name = match s {
                Stmt::Assign { name, .. } | Stmt::FuncDef { name, .. } => name,
                Stmt::Expr(_) => continue,
            };
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    /// Free names the program reads from outside itself: identifiers and called function
    /// names that are not function parameters and not defined by an earlier statement of the
    /// same program. Built-in functions, constants and units are included when they appear
    /// (they simply match nothing a document defines).
    pub fn used_names(&self) -> BTreeSet<String> {
        let mut used = BTreeSet::new();
        let mut defined: BTreeSet<String> = BTreeSet::new();
        for s in &self.stmts {
            match s {
                Stmt::Assign { name, value, .. } => {
                    collect(value, &defined, &mut BTreeSet::new(), &mut used);
                    defined.insert(name.clone());
                }
                Stmt::FuncDef {
                    name, params, body, ..
                } => {
                    let mut bound: BTreeSet<String> = params.iter().cloned().collect();
                    bound.insert(name.clone());
                    collect(body, &defined, &mut bound, &mut used);
                    defined.insert(name.clone());
                }
                Stmt::Expr(e) => collect(e, &defined, &mut BTreeSet::new(), &mut used),
            }
        }
        used
    }
}

/// Free names of one expression: identifiers and called function names that are not in
/// `bound`.
pub fn free_names(e: &Expr, bound: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut b: BTreeSet<String> = bound.iter().cloned().collect();
    collect(e, &BTreeSet::new(), &mut b, &mut out);
    out
}

fn collect(
    e: &Expr,
    defined: &BTreeSet<String>,
    bound: &mut BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    let note = |name: &str, out: &mut BTreeSet<String>| {
        if !bound.contains(name) && !defined.contains(name) {
            out.insert(name.to_string());
        }
    };
    match &e.kind {
        ExprKind::Number(_)
        | ExprKind::Imag(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::ColonAll => {}
        ExprKind::Ident(n) => note(n, out),
        ExprKind::Paren(a) | ExprKind::Unary(_, a) | ExprKind::Convert(a, _) => {
            collect(a, defined, bound, out)
        }
        ExprKind::Binary(_, a, b) | ExprKind::Implicit(a, b) => {
            collect(a, defined, bound, out);
            collect(b, defined, bound, out);
        }
        ExprKind::Call(n, _, args) => {
            note(n, out);
            for a in args {
                collect(a, defined, bound, out);
            }
        }
        ExprKind::Index(b, args) => {
            collect(b, defined, bound, out);
            for a in args {
                collect(a, defined, bound, out);
            }
        }
        ExprKind::Vector(items) => {
            for a in items {
                collect(a, defined, bound, out);
            }
        }
        ExprKind::Matrix(rows) => {
            for a in rows.iter().flatten() {
                collect(a, defined, bound, out);
            }
        }
        ExprKind::Range(a, step, c) => {
            collect(a, defined, bound, out);
            if let Some(s) = step {
                collect(s, defined, bound, out);
            }
            collect(c, defined, bound, out);
        }
    }
}
