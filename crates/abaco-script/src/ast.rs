// SPDX-License-Identifier: MIT OR Apache-2.0
//! The syntax tree.

use crate::error::Span;
use std::sync::Arc;

/// Binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    LDiv,
    Pow,
    ElemMul,
    ElemDiv,
    ElemLDiv,
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

/// Unary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
}

/// An expression with its position.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub span: Span,
    /// Height of the tree below and including this node; the parser bounds it so that
    /// evaluating and dropping a tree never exhausts the stack.
    pub height: u32,
}

impl Expr {
    /// A node of `kind` at `span`, with its height worked out from its children.
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        let below = match &kind {
            ExprKind::Unary(_, a)
            | ExprKind::Transpose(a, _)
            | ExprKind::Field(a, _)
            | ExprKind::Anon(_, a, _) => a.height,
            ExprKind::Binary(_, a, b)
            | ExprKind::ShortAnd(a, b)
            | ExprKind::ShortOr(a, b)
            | ExprKind::DynField(a, b) => a.height.max(b.height),
            ExprKind::Index(a, args) | ExprKind::CellIndex(a, args) => {
                args.iter().map(|e| e.height).fold(a.height, u32::max)
            }
            ExprKind::Range(a, b, c) => a
                .height
                .max(c.height)
                .max(b.as_ref().map_or(0, |e| e.height)),
            ExprKind::Matrix(rows) | ExprKind::CellLit(rows) => {
                rows.iter().flatten().map(|e| e.height).max().unwrap_or(0)
            }
            _ => 0,
        };
        Expr {
            kind,
            span,
            height: below.saturating_add(1),
        }
    }
}

/// Kinds of expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Num(f64),
    Imag(f64),
    Str(String),
    Ident(String),
    /// `:` as a whole-dimension index.
    Colon,
    /// `end` inside an index.
    End,
    /// `~` in `[~, k] = f(x)`.
    Tilde,
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a && b`.
    ShortAnd(Box<Expr>, Box<Expr>),
    /// `a || b`.
    ShortOr(Box<Expr>, Box<Expr>),
    /// `a'` (conjugate) or `a.'`.
    Transpose(Box<Expr>, bool),
    /// `a(args)`: an index or a call.
    Index(Box<Expr>, Vec<Expr>),
    /// `a{args}`.
    CellIndex(Box<Expr>, Vec<Expr>),
    /// `a.name`.
    Field(Box<Expr>, String),
    /// `a.(expr)`.
    DynField(Box<Expr>, Box<Expr>),
    /// `a:b` or `a:step:b`.
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    /// `[a, b; c, d]`.
    Matrix(Vec<Vec<Expr>>),
    /// `{a, b; c, d}`.
    CellLit(Vec<Vec<Expr>>),
    /// `@(x, y) body`; the last field is the source text of the whole handle.
    Anon(Vec<String>, Box<Expr>, String),
    /// `@name`.
    Handle(String),
}

/// A statement with its position and whether its result is printed.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    /// What it is.
    pub kind: StmtKind,
    /// Where it is.
    pub span: Span,
    /// `true` when no `;` follows, so a result is shown.
    pub print: bool,
}

/// Kinds of statement.
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Expr(Expr),
    /// `targets = value`; `None` is a `~` placeholder.
    Assign(Vec<Option<Expr>>, Expr),
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    For(Expr, Expr, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    Switch(Expr, Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    Try(Vec<Stmt>, Option<String>, Vec<Stmt>),
    Break,
    Continue,
    Return,
    Func(Arc<FuncDef>),
    /// Command syntax such as `hold on` or `format long`.
    Command(String, Vec<String>),
    Global(Vec<String>),
}

/// A function defined in a script.
#[derive(Clone, Debug, PartialEq)]
pub struct FuncDef {
    /// Its name.
    pub name: String,
    /// Input names; the last may be `varargin`.
    pub params: Vec<String>,
    /// Output names; the last may be `varargout`.
    pub outputs: Vec<String>,
    /// The body.
    pub body: Vec<Stmt>,
}

/// A parsed script: statements and the functions it defines.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// Top-level statements.
    pub stmts: Vec<Stmt>,
}
