// SPDX-License-Identifier: MIT OR Apache-2.0
//! Recursive-descent parser for Ábaco Script.
//!
//! Precedence, loosest to tightest: `||`, `&&`, `|`, `&`, comparisons, `:`, `+ -`,
//! `* / \ .* ./ .\`, unary `+ - ~`, `^ .^ ' .'`. A unary minus binds looser than a power, so
//! `-2^2` is `-4`, but the exponent may carry its own sign: `2^-2`.

use crate::ast::{BinOp, Expr, ExprKind, FuncDef, Program, Stmt, StmtKind, UnOp};
use crate::error::{Error, Result, Span};
use crate::lexer::{lex, Kw, Tok, Token};
use std::sync::Arc;

/// Most levels of nested brackets and blocks the parser follows.
const MAX_DEPTH: usize = 120;

/// Tallest expression tree the parser accepts (a chain `a + b + c + ...` is as tall as
/// it has terms).
const MAX_HEIGHT: u32 = 300;

/// Native stack the parser may use for nesting (a debug build uses far more per level than
/// a release build, so the depth limit alone is not enough on a small thread).
const MAX_PARSE_STACK: usize = 700_000;

/// Words that start command syntax (`hold on`).
const COMMANDS: [&str; 12] = [
    "hold",
    "format",
    "close",
    "clear",
    "clc",
    "figure",
    "grid",
    "axis",
    "clearvars",
    "more",
    "pkg",
    "warning",
];

/// Parses a script.
pub fn parse(src: &str) -> Result<Program> {
    if src.len() > 2_000_000 {
        return Err(Error::new("Abaco:limit", "the script is larger than 2 MB"));
    }
    let toks = lex(src)?;
    let mut p = Parser {
        src,
        toks,
        i: 0,
        index_depth: 0,
        matrix_depth: 0,
        depth: 0,
        stack_base: stack_addr(),
    };
    p.program()
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    i: usize,
    /// Inside the arguments of an index, where `end` is a value.
    index_depth: usize,
    /// Inside `[...]` or `{...}`, where spaces separate elements.
    matrix_depth: usize,
    depth: usize,
    stack_base: usize,
}

fn stack_addr() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

impl Parser<'_> {
    fn tok(&self) -> &Tok {
        self.toks.get(self.i).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn tok_at(&self, k: usize) -> &Tok {
        self.toks.get(self.i + k).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn cur(&self) -> Token {
        self.toks.get(self.i).cloned().unwrap_or(Token {
            tok: Tok::Eof,
            span: Span::new(self.src.len(), self.src.len()),
            space_before: false,
            space_after: true,
        })
    }

    fn span(&self) -> Span {
        self.cur().span
    }

    fn prev_end(&self) -> usize {
        self.i
            .checked_sub(1)
            .and_then(|k| self.toks.get(k))
            .map_or(0, |t| t.span.end)
    }

    fn bump(&mut self) -> Token {
        let t = self.cur();
        if self.i < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.tok() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<Token> {
        if self.tok() == t {
            Ok(self.bump())
        } else {
            Err(Error::syntax(
                self.span(),
                format!("expected {what}, found {}", self.tok().describe()),
            ))
        }
    }

    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH || self.stack_base.saturating_sub(stack_addr()) > MAX_PARSE_STACK
        {
            return Err(Error::new(
                "Abaco:limit",
                format!("the script nests too deeply (more than about {MAX_DEPTH} levels of brackets or blocks)"),
            )
            .or_span(self.span()));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn mk(kind: ExprKind, span: Span) -> Expr {
        Expr::new(kind, span)
    }

    // ---------- statements ----------

    fn program(&mut self) -> Result<Program> {
        let stmts = self.block(&[])?;
        if !matches!(self.tok(), Tok::Eof) {
            return Err(Error::syntax(
                self.span(),
                format!("unexpected {}", self.tok().describe()),
            ));
        }
        Ok(Program { stmts })
    }

    fn at_terminator(&self, stops: &[Kw]) -> bool {
        match self.tok() {
            Tok::Eof => true,
            Tok::Kw(k) => stops.contains(k),
            _ => false,
        }
    }

    /// Statements until one of the `stops` keywords (not consumed) or the end.
    fn block(&mut self, stops: &[Kw]) -> Result<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            while matches!(self.tok(), Tok::Newline | Tok::Semi | Tok::Comma) {
                self.bump();
            }
            if self.at_terminator(stops) {
                return Ok(out);
            }
            if let Tok::Kw(
                k @ (Kw::End | Kw::Else | Kw::Elseif | Kw::Case | Kw::Otherwise | Kw::Catch),
            ) = self.tok()
            {
                if !stops.contains(k) {
                    return Err(Error::syntax(
                        self.span(),
                        format!("unexpected {}", self.tok().describe()),
                    ));
                }
            }
            out.push(self.stmt()?);
        }
    }

    /// Reads the end of a statement: a separator that says whether to print.
    fn end_of_stmt(&mut self) -> Result<bool> {
        match self.tok() {
            Tok::Semi => {
                self.bump();
                Ok(false)
            }
            Tok::Comma | Tok::Newline => {
                self.bump();
                Ok(true)
            }
            Tok::Eof => Ok(true),
            Tok::Kw(_) => Ok(true),
            other => Err(Error::syntax(
                self.span(),
                format!(
                    "unexpected {} (separate statements with `;`, `,` or a new line)",
                    other.describe()
                ),
            )),
        }
    }

    fn stmt(&mut self) -> Result<Stmt> {
        self.enter()?;
        let r = self.stmt_inner();
        self.leave();
        r
    }

    fn stmt_inner(&mut self) -> Result<Stmt> {
        let start = self.span();
        let kind = match self.tok().clone() {
            Tok::Kw(Kw::If) => self.if_stmt()?,
            Tok::Kw(Kw::For) => self.for_stmt()?,
            Tok::Kw(Kw::While) => {
                self.bump();
                let cond = self.expr()?;
                let body = self.block(&[Kw::End])?;
                self.expect(&Tok::Kw(Kw::End), "`end` to close the `while`")?;
                StmtKind::While(cond, body)
            }
            Tok::Kw(Kw::Switch) => self.switch_stmt()?,
            Tok::Kw(Kw::Try) => {
                self.bump();
                self.eat(&Tok::Comma);
                let body = self.block(&[Kw::Catch, Kw::End])?;
                let mut name = None;
                let mut handler = Vec::new();
                if self.eat(&Tok::Kw(Kw::Catch)) {
                    // `catch err` on the same line names the error
                    if let Tok::Ident(n) = self.tok().clone() {
                        if !matches!(self.tok_at(1), Tok::Assign | Tok::LParen | Tok::Dot) {
                            self.bump();
                            name = Some(n);
                        }
                    }
                    handler = self.block(&[Kw::End])?;
                }
                self.expect(&Tok::Kw(Kw::End), "`end` to close the `try`")?;
                StmtKind::Try(body, name, handler)
            }
            Tok::Kw(Kw::Break) => {
                self.bump();
                StmtKind::Break
            }
            Tok::Kw(Kw::Continue) => {
                self.bump();
                StmtKind::Continue
            }
            Tok::Kw(Kw::Return) => {
                self.bump();
                StmtKind::Return
            }
            Tok::Kw(Kw::Function) => self.function()?,
            Tok::Kw(Kw::Global) | Tok::Kw(Kw::Persistent) => {
                self.bump();
                let mut names = Vec::new();
                while let Tok::Ident(n) = self.tok().clone() {
                    self.bump();
                    names.push(n);
                }
                StmtKind::Global(names)
            }
            Tok::Ident(name) if self.is_command(&name) => self.command(name),
            _ => self.simple()?,
        };
        let is_block = matches!(
            kind,
            StmtKind::If(..)
                | StmtKind::For(..)
                | StmtKind::While(..)
                | StmtKind::Switch(..)
                | StmtKind::Try(..)
                | StmtKind::Func(_)
        );
        let print = if is_block { false } else { self.end_of_stmt()? };
        let span = start.to(Span::new(self.prev_end(), self.prev_end()));
        Ok(Stmt { kind, span, print })
    }

    fn is_command(&self, name: &str) -> bool {
        if !COMMANDS.contains(&name) {
            return false;
        }
        // `hold on`, `clear x y`, `clc`: a word followed by words, or nothing
        match self.tok_at(1) {
            Tok::Newline | Tok::Semi | Tok::Comma | Tok::Eof => true,
            Tok::Ident(_) | Tok::Num(_) | Tok::Str(_) => {
                !matches!(self.tok_at(2), Tok::Assign | Tok::LParen)
            }
            _ => false,
        }
    }

    fn command(&mut self, name: String) -> StmtKind {
        self.bump();
        let mut args = Vec::new();
        loop {
            match self.tok().clone() {
                Tok::Ident(a) => args.push(a),
                Tok::Num(x) => args.push(format!("{x}")),
                Tok::Str(s) => args.push(s),
                _ => break,
            }
            self.bump();
        }
        StmtKind::Command(name, args)
    }

    fn if_stmt(&mut self) -> Result<StmtKind> {
        self.bump();
        let mut branches = Vec::new();
        let cond = self.expr()?;
        let body = self.block(&[Kw::Elseif, Kw::Else, Kw::End])?;
        branches.push((cond, body));
        let mut otherwise = Vec::new();
        loop {
            match self.tok() {
                Tok::Kw(Kw::Elseif) => {
                    self.bump();
                    let c = self.expr()?;
                    let b = self.block(&[Kw::Elseif, Kw::Else, Kw::End])?;
                    branches.push((c, b));
                }
                Tok::Kw(Kw::Else) => {
                    self.bump();
                    otherwise = self.block(&[Kw::End])?;
                }
                _ => break,
            }
        }
        self.expect(&Tok::Kw(Kw::End), "`end` to close the `if`")?;
        Ok(StmtKind::If(branches, otherwise))
    }

    fn for_stmt(&mut self) -> Result<StmtKind> {
        self.bump();
        let paren = self.eat(&Tok::LParen);
        let var = self.postfix_chain()?;
        self.expect(&Tok::Assign, "`=` after the loop variable")?;
        let iter = self.expr()?;
        if paren {
            self.expect(&Tok::RParen, "`)`")?;
        }
        let body = self.block(&[Kw::End])?;
        self.expect(&Tok::Kw(Kw::End), "`end` to close the `for`")?;
        Ok(StmtKind::For(var, iter, body))
    }

    fn switch_stmt(&mut self) -> Result<StmtKind> {
        self.bump();
        let subject = self.expr()?;
        let mut cases = Vec::new();
        let mut otherwise = Vec::new();
        loop {
            while matches!(self.tok(), Tok::Newline | Tok::Semi | Tok::Comma) {
                self.bump();
            }
            match self.tok() {
                Tok::Kw(Kw::Case) => {
                    self.bump();
                    let v = self.expr()?;
                    let b = self.block(&[Kw::Case, Kw::Otherwise, Kw::End])?;
                    cases.push((v, b));
                }
                Tok::Kw(Kw::Otherwise) => {
                    self.bump();
                    otherwise = self.block(&[Kw::End])?;
                }
                _ => break,
            }
        }
        self.expect(&Tok::Kw(Kw::End), "`end` to close the `switch`")?;
        Ok(StmtKind::Switch(subject, cases, otherwise))
    }

    fn function(&mut self) -> Result<StmtKind> {
        self.bump();
        let mut outputs = Vec::new();
        // `function [a, b] = name(...)`, `function a = name(...)` or `function name(...)`
        let name;
        if self.eat(&Tok::LBracket) {
            while !matches!(self.tok(), Tok::RBracket | Tok::Eof) {
                match self.bump().tok {
                    Tok::Ident(n) => outputs.push(n),
                    Tok::Comma => {}
                    other => {
                        return Err(Error::syntax(
                            self.span(),
                            format!("expected an output name, found {}", other.describe()),
                        ))
                    }
                }
            }
            self.expect(&Tok::RBracket, "`]`")?;
            self.expect(&Tok::Assign, "`=` after the outputs")?;
            name = self.fn_name()?;
        } else {
            let first = self.fn_name()?;
            if self.eat(&Tok::Assign) {
                outputs.push(first);
                name = self.fn_name()?;
            } else {
                name = first;
            }
        }
        let mut params = Vec::new();
        if self.eat(&Tok::LParen) {
            while !matches!(self.tok(), Tok::RParen | Tok::Eof) {
                match self.bump().tok {
                    Tok::Ident(n) => params.push(n),
                    Tok::Not => params.push("~".into()),
                    Tok::Comma => {}
                    other => {
                        return Err(Error::syntax(
                            self.span(),
                            format!("expected a parameter name, found {}", other.describe()),
                        ))
                    }
                }
            }
            self.expect(&Tok::RParen, "`)`")?;
        }
        let body = self.block(&[Kw::End, Kw::Function])?;
        // A function ends with `end`, or with the next `function` or the end of the file.
        if matches!(self.tok(), Tok::Kw(Kw::End)) {
            self.bump();
        }
        Ok(StmtKind::Func(Arc::new(FuncDef {
            name,
            params,
            outputs,
            body,
        })))
    }

    fn fn_name(&mut self) -> Result<String> {
        match self.bump().tok {
            Tok::Ident(n) => Ok(n),
            other => Err(Error::syntax(
                self.span(),
                format!("expected a function name, found {}", other.describe()),
            )),
        }
    }

    /// An expression statement or an assignment.
    fn simple(&mut self) -> Result<StmtKind> {
        // `[a, ~, b] = f(x)`: try the multiple-assignment form first
        if matches!(self.tok(), Tok::LBracket) {
            if let Some(targets) = self.try_multi_targets()? {
                let value = self.expr()?;
                return Ok(StmtKind::Assign(targets, value));
            }
        }
        let lhs = self.expr()?;
        match self.tok() {
            Tok::Assign => {
                if !is_lvalue(&lhs) {
                    return Err(Error::syntax(
                        lhs.span,
                        "the left side of `=` must be a variable, an element or a field",
                    ));
                }
                self.bump();
                let value = self.expr()?;
                Ok(StmtKind::Assign(vec![Some(lhs)], value))
            }
            Tok::PlusAssign | Tok::MinusAssign | Tok::StarAssign | Tok::SlashAssign => {
                if !is_lvalue(&lhs) {
                    return Err(Error::syntax(
                        lhs.span,
                        "the left side of the assignment must be a variable, an element or a field",
                    ));
                }
                let op = match self.bump().tok {
                    Tok::PlusAssign => BinOp::Add,
                    Tok::MinusAssign => BinOp::Sub,
                    Tok::StarAssign => BinOp::Mul,
                    _ => BinOp::Div,
                };
                let rhs = self.expr()?;
                let span = lhs.span.to(rhs.span);
                let value = Self::mk(
                    ExprKind::Binary(op, Box::new(lhs.clone()), Box::new(rhs)),
                    span,
                );
                Ok(StmtKind::Assign(vec![Some(lhs)], value))
            }
            _ => Ok(StmtKind::Expr(lhs)),
        }
    }

    /// If the tokens are `[ targets ] =`, parses them and the `=`.
    fn try_multi_targets(&mut self) -> Result<Option<Vec<Option<Expr>>>> {
        // find the matching `]` and check for `=`
        let mut depth = 0usize;
        let mut k = 0usize;
        loop {
            match self.tok_at(k) {
                Tok::LBracket | Tok::LParen | Tok::LBrace => depth += 1,
                Tok::RBracket | Tok::RParen | Tok::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Eof | Tok::Newline => return Ok(None),
                _ => {}
            }
            k += 1;
        }
        if !matches!(self.tok_at(k + 1), Tok::Assign) {
            return Ok(None);
        }
        self.bump(); // [
        self.matrix_depth += 1;
        let mut targets = Vec::new();
        loop {
            match self.tok() {
                Tok::RBracket => break,
                Tok::Comma => {
                    self.bump();
                }
                Tok::Not if matches!(self.tok_at(1), Tok::Comma | Tok::RBracket) => {
                    self.bump();
                    targets.push(None);
                }
                _ => {
                    let e = self.postfix_chain()?;
                    if !is_lvalue(&e) {
                        self.matrix_depth -= 1;
                        return Err(Error::syntax(
                            e.span,
                            "an output of `[...] =` must be a variable, an element or a field",
                        ));
                    }
                    targets.push(Some(e));
                }
            }
        }
        self.matrix_depth -= 1;
        self.expect(&Tok::RBracket, "`]`")?;
        self.expect(&Tok::Assign, "`=`")?;
        Ok(Some(targets))
    }

    // ---------- expressions ----------

    fn expr(&mut self) -> Result<Expr> {
        self.enter()?;
        let r = self.or_or();
        self.leave();
        let e = r?;
        if e.height > MAX_HEIGHT {
            return Err(Error::new(
                "Abaco:limit",
                format!("the expression is more than {MAX_HEIGHT} operations deep; split it into several statements"),
            )
            .or_span(e.span));
        }
        Ok(e)
    }

    fn or_or(&mut self) -> Result<Expr> {
        let mut l = self.and_and()?;
        while matches!(self.tok(), Tok::OrOr) {
            self.bump();
            let r = self.and_and()?;
            let span = l.span.to(r.span);
            l = Self::mk(ExprKind::ShortOr(Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn and_and(&mut self) -> Result<Expr> {
        let mut l = self.or_()?;
        while matches!(self.tok(), Tok::AndAnd) {
            self.bump();
            let r = self.or_()?;
            let span = l.span.to(r.span);
            l = Self::mk(ExprKind::ShortAnd(Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn bin(&self, op: BinOp, l: Expr, r: Expr) -> Expr {
        let span = l.span.to(r.span);
        Self::mk(ExprKind::Binary(op, Box::new(l), Box::new(r)), span)
    }

    fn or_(&mut self) -> Result<Expr> {
        let mut l = self.and_()?;
        while matches!(self.tok(), Tok::Or) {
            self.bump();
            let r = self.and_()?;
            l = self.bin(BinOp::Or, l, r);
        }
        Ok(l)
    }

    fn and_(&mut self) -> Result<Expr> {
        let mut l = self.cmp()?;
        while matches!(self.tok(), Tok::And) {
            self.bump();
            let r = self.cmp()?;
            l = self.bin(BinOp::And, l, r);
        }
        Ok(l)
    }

    fn cmp(&mut self) -> Result<Expr> {
        let mut l = self.range()?;
        loop {
            let op = match self.tok() {
                Tok::EqEq => BinOp::Eq,
                Tok::Ne => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.range()?;
            l = self.bin(op, l, r);
        }
    }

    /// Whether the `:` at the cursor ends an index argument (`a(2, :)`).
    fn colon_ends_arg(&self) -> bool {
        matches!(self.tok_at(1), Tok::RParen | Tok::Comma | Tok::RBrace)
    }

    fn range(&mut self) -> Result<Expr> {
        let first = self.additive()?;
        if !matches!(self.tok(), Tok::Colon) || self.colon_ends_arg() {
            return Ok(first);
        }
        self.bump();
        let second = self.additive()?;
        if matches!(self.tok(), Tok::Colon) && !self.colon_ends_arg() {
            self.bump();
            let third = self.additive()?;
            let span = first.span.to(third.span);
            return Ok(Self::mk(
                ExprKind::Range(Box::new(first), Some(Box::new(second)), Box::new(third)),
                span,
            ));
        }
        let span = first.span.to(second.span);
        Ok(Self::mk(
            ExprKind::Range(Box::new(first), None, Box::new(second)),
            span,
        ))
    }

    /// In `[1 -2]` a `+`/`-` with space before and none after starts a new element.
    fn starts_new_element(&self) -> bool {
        if self.matrix_depth == 0 {
            return false;
        }
        let t = self.cur();
        matches!(t.tok, Tok::Plus | Tok::Minus) && t.space_before && !t.space_after
    }

    fn additive(&mut self) -> Result<Expr> {
        let mut l = self.multiplicative()?;
        loop {
            let op = match self.tok() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(l),
            };
            if self.starts_new_element() {
                return Ok(l);
            }
            self.bump();
            let r = self.multiplicative()?;
            l = self.bin(op, l, r);
        }
    }

    fn multiplicative(&mut self) -> Result<Expr> {
        let mut l = self.unary()?;
        loop {
            let op = match self.tok() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Backslash => BinOp::LDiv,
                Tok::DotStar => BinOp::ElemMul,
                Tok::DotSlash => BinOp::ElemDiv,
                Tok::DotBackslash => BinOp::ElemLDiv,
                _ => return Ok(l),
            };
            self.bump();
            let r = self.unary()?;
            l = self.bin(op, l, r);
        }
    }

    fn unary(&mut self) -> Result<Expr> {
        let op = match self.tok() {
            Tok::Minus => UnOp::Neg,
            Tok::Plus => UnOp::Pos,
            Tok::Not => UnOp::Not,
            _ => return self.power(),
        };
        let start = self.bump().span;
        self.enter()?;
        let operand = self.unary();
        self.leave();
        let operand = operand?;
        let span = start.to(operand.span);
        Ok(Self::mk(ExprKind::Unary(op, Box::new(operand)), span))
    }

    fn power(&mut self) -> Result<Expr> {
        let mut base = self.postfix()?;
        loop {
            let op = match self.tok() {
                Tok::Caret => BinOp::Pow,
                Tok::DotCaret => BinOp::ElemPow,
                _ => return Ok(base),
            };
            self.bump();
            let exp = self.power_operand()?;
            base = self.bin(op, base, exp);
        }
    }

    /// The exponent: a postfix expression that may carry its own sign.
    fn power_operand(&mut self) -> Result<Expr> {
        let op = match self.tok() {
            Tok::Minus => Some(UnOp::Neg),
            Tok::Plus => Some(UnOp::Pos),
            Tok::Not => Some(UnOp::Not),
            _ => None,
        };
        match op {
            Some(op) => {
                let start = self.bump().span;
                self.enter()?;
                let inner = self.power_operand();
                self.leave();
                let inner = inner?;
                let span = start.to(inner.span);
                Ok(Self::mk(ExprKind::Unary(op, Box::new(inner)), span))
            }
            None => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr> {
        let e = self.postfix_chain()?;
        Ok(e)
    }

    fn args(&mut self, close: &Tok) -> Result<Vec<Expr>> {
        let mut out = Vec::new();
        self.index_depth += 1;
        let saved = self.matrix_depth;
        self.matrix_depth = 0;
        let r = (|| -> Result<()> {
            if self.tok() == close {
                return Ok(());
            }
            loop {
                if matches!(self.tok(), Tok::Colon)
                    && matches!(self.tok_at(1), Tok::Comma | Tok::RParen | Tok::RBrace)
                {
                    let s = self.bump().span;
                    out.push(Self::mk(ExprKind::Colon, s));
                } else {
                    out.push(self.expr()?);
                }
                if !self.eat(&Tok::Comma) {
                    return Ok(());
                }
            }
        })();
        self.matrix_depth = saved;
        self.index_depth -= 1;
        r?;
        Ok(out)
    }

    fn postfix_chain(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            match self.tok() {
                Tok::LParen => {
                    if self.matrix_depth > 0 && self.cur().space_before {
                        return Ok(e);
                    }
                    self.bump();
                    let args = self.args(&Tok::RParen)?;
                    let close = self.expect(&Tok::RParen, "`)`")?;
                    let span = e.span.to(close.span);
                    e = Self::mk(ExprKind::Index(Box::new(e), args), span);
                }
                Tok::LBrace => {
                    if self.matrix_depth > 0 && self.cur().space_before {
                        return Ok(e);
                    }
                    self.bump();
                    let args = self.args(&Tok::RBrace)?;
                    let close = self.expect(&Tok::RBrace, "`}`")?;
                    let span = e.span.to(close.span);
                    e = Self::mk(ExprKind::CellIndex(Box::new(e), args), span);
                }
                Tok::Dot => {
                    self.bump();
                    match self.tok().clone() {
                        Tok::Ident(n) => {
                            let s = self.bump().span;
                            let span = e.span.to(s);
                            e = Self::mk(ExprKind::Field(Box::new(e), n), span);
                        }
                        Tok::Kw(_) => {
                            let t = self.bump();
                            let name = self
                                .src
                                .get(t.span.start..t.span.end)
                                .unwrap_or("")
                                .to_string();
                            let span = e.span.to(t.span);
                            e = Self::mk(ExprKind::Field(Box::new(e), name), span);
                        }
                        Tok::LParen => {
                            self.bump();
                            let inner = self.expr()?;
                            let close = self.expect(&Tok::RParen, "`)`")?;
                            let span = e.span.to(close.span);
                            e = Self::mk(ExprKind::DynField(Box::new(e), Box::new(inner)), span);
                        }
                        other => {
                            return Err(Error::syntax(
                                self.span(),
                                format!(
                                    "expected a field name after `.`, found {}",
                                    other.describe()
                                ),
                            ))
                        }
                    }
                }
                Tok::Transpose | Tok::DotTranspose => {
                    let t = self.bump();
                    let conj = matches!(t.tok, Tok::Transpose);
                    let span = e.span.to(t.span);
                    e = Self::mk(ExprKind::Transpose(Box::new(e), conj), span);
                }
                _ => return Ok(e),
            }
        }
    }

    fn primary(&mut self) -> Result<Expr> {
        let t = self.cur();
        let span = t.span;
        match t.tok {
            Tok::Num(x) => {
                self.bump();
                Ok(Self::mk(ExprKind::Num(x), span))
            }
            Tok::Imag(x) => {
                self.bump();
                Ok(Self::mk(ExprKind::Imag(x), span))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Self::mk(ExprKind::Str(s), span))
            }
            Tok::Ident(n) => {
                self.bump();
                Ok(Self::mk(ExprKind::Ident(n), span))
            }
            Tok::Kw(Kw::End) if self.index_depth > 0 => {
                self.bump();
                Ok(Self::mk(ExprKind::End, span))
            }
            Tok::Colon if self.index_depth > 0 => {
                self.bump();
                Ok(Self::mk(ExprKind::Colon, span))
            }
            Tok::Not if matches!(self.tok_at(1), Tok::Comma | Tok::RBracket) => {
                self.bump();
                Ok(Self::mk(ExprKind::Tilde, span))
            }
            Tok::LParen => {
                self.bump();
                let saved = self.matrix_depth;
                self.matrix_depth = 0;
                let inner = self.expr();
                self.matrix_depth = saved;
                let inner = inner?;
                let close = self.expect(&Tok::RParen, "`)`")?;
                // Parentheses stop a cs-list and keep `(a)'` etc. as written; wrap as a
                // one-argument paren node by reusing the inner expression's kind.
                Ok(Expr {
                    kind: inner.kind,
                    span: span.to(close.span),
                    height: inner.height,
                })
            }
            Tok::LBracket => {
                self.bump();
                let rows = self.matrix_rows(&Tok::RBracket)?;
                let close = self.expect(&Tok::RBracket, "`]`")?;
                Ok(Self::mk(ExprKind::Matrix(rows), span.to(close.span)))
            }
            Tok::LBrace => {
                self.bump();
                let rows = self.matrix_rows(&Tok::RBrace)?;
                let close = self.expect(&Tok::RBrace, "`}`")?;
                Ok(Self::mk(ExprKind::CellLit(rows), span.to(close.span)))
            }
            Tok::At => {
                self.bump();
                if self.eat(&Tok::LParen) {
                    let mut params = Vec::new();
                    while !matches!(self.tok(), Tok::RParen | Tok::Eof) {
                        match self.bump().tok {
                            Tok::Ident(n) => params.push(n),
                            Tok::Not => params.push("~".into()),
                            Tok::Comma => {}
                            other => {
                                return Err(Error::syntax(
                                    self.span(),
                                    format!(
                                        "expected a parameter name, found {}",
                                        other.describe()
                                    ),
                                ))
                            }
                        }
                    }
                    self.expect(&Tok::RParen, "`)`")?;
                    let saved = (self.matrix_depth, self.index_depth);
                    self.matrix_depth = 0;
                    self.index_depth = 0;
                    let body = self.expr();
                    self.matrix_depth = saved.0;
                    self.index_depth = saved.1;
                    let body = body?;
                    let full = span.to(body.span);
                    let src = self.src.get(full.start..full.end).unwrap_or("").to_string();
                    Ok(Self::mk(ExprKind::Anon(params, Box::new(body), src), full))
                } else if let Tok::Ident(n) = self.tok().clone() {
                    let s = self.bump().span;
                    // `@pkg.name` is rare; accept dotted names
                    let mut name = n;
                    while matches!(self.tok(), Tok::Dot) {
                        if let Tok::Ident(m) = self.tok_at(1).clone() {
                            self.bump();
                            self.bump();
                            name.push('.');
                            name.push_str(&m);
                        } else {
                            break;
                        }
                    }
                    Ok(Self::mk(ExprKind::Handle(name), span.to(s)))
                } else {
                    Err(Error::syntax(
                        self.span(),
                        "expected `(` or a function name after `@`",
                    ))
                }
            }
            other => Err(Error::syntax(
                span,
                format!("unexpected {}", other.describe()),
            )),
        }
    }

    /// Rows of a matrix or cell literal, up to the closing bracket (not consumed).
    fn matrix_rows(&mut self, close: &Tok) -> Result<Vec<Vec<Expr>>> {
        self.matrix_depth += 1;
        let saved_index = self.index_depth;
        let r = self.matrix_rows_inner(close);
        self.matrix_depth -= 1;
        self.index_depth = saved_index;
        r
    }

    fn matrix_rows_inner(&mut self, close: &Tok) -> Result<Vec<Vec<Expr>>> {
        let mut rows: Vec<Vec<Expr>> = Vec::new();
        let mut row: Vec<Expr> = Vec::new();
        loop {
            match self.tok() {
                t if t == close => break,
                Tok::Eof => {
                    return Err(Error::syntax(self.span(), "the bracket is never closed"));
                }
                Tok::Semi | Tok::Newline => {
                    self.bump();
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                    }
                }
                Tok::Comma => {
                    self.bump();
                }
                _ => {
                    row.push(self.expr()?);
                }
            }
        }
        if !row.is_empty() {
            rows.push(row);
        }
        Ok(rows)
    }
}

fn is_lvalue(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Ident(_) => true,
        ExprKind::Index(b, _)
        | ExprKind::CellIndex(b, _)
        | ExprKind::Field(b, _)
        | ExprKind::DynField(b, _) => is_lvalue(b),
        _ => false,
    }
}
