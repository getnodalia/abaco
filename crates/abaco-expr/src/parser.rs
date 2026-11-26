// SPDX-License-Identifier: MIT OR Apache-2.0
//! Recursive-descent / precedence-climbing parser.
//!
//! Precedence, loosest to tightest: `->`/`to`, `or`, `and`, `not`, comparison, range `:`,
//! `+ -`, `* / .* ./` and implicit multiplication (juxtaposition, tighter), unary `-`,
//! `^ .^` (right associative; `-2^2 = -4`, `2^-3 = 1/8`), postfix (call, index, superscript).

use crate::ast::{BinOp, Expr, ExprKind, Program, Stmt, UnOp};
use crate::error::{ErrorKind, ExprError, Span};
use crate::lexer::{lex, Tok, Token};
use crate::limits::Limits;
use nodalia_units::Unit;

/// Words that are operators or literals and cannot be names.
pub const KEYWORDS: [&str; 7] = ["and", "or", "not", "to", "true", "false", "xor"];

/// Parses a cell with default [`Limits`].
pub fn parse(src: &str) -> Result<Program, ExprError> {
    parse_with(src, &Limits::default())
}

/// Parses a cell: statements separated by `;` or newlines.
pub fn parse_with(src: &str, limits: &Limits) -> Result<Program, ExprError> {
    if src.len() > limits.max_source {
        return Err(ExprError::new(
            ErrorKind::Limit,
            format!(
                "the source is {} bytes long; the limit is {}",
                src.len(),
                limits.max_source
            ),
        ));
    }
    let toks = lex(src)?;
    let mut p = Parser {
        src,
        toks,
        i: 0,
        group: 0,
        limits: *limits,
        elements: 0,
    };
    p.program()
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    i: usize,
    /// Nesting of `()` and `[]`; newlines inside are whitespace.
    group: usize,
    limits: Limits,
    elements: usize,
}

fn syntax(span: Span, msg: impl Into<String>) -> ExprError {
    ExprError::at(ErrorKind::Syntax, span, msg)
}

impl Parser<'_> {
    fn peek_tok(&mut self) -> &Tok {
        if self.group > 0 {
            while matches!(self.toks.get(self.i).map(|t| &t.tok), Some(Tok::Newline)) {
                self.i += 1;
            }
        }
        self.toks.get(self.i).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn peek_at(&self, off: usize) -> &Tok {
        self.toks.get(self.i + off).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn span(&mut self) -> Span {
        self.peek_tok();
        self.toks
            .get(self.i)
            .map_or(Span::new(self.src.len(), self.src.len()), |t| t.span)
    }

    fn bump(&mut self) -> Token {
        self.peek_tok();
        let t = self.toks.get(self.i).cloned().unwrap_or(Token {
            tok: Tok::Eof,
            span: Span::new(self.src.len(), self.src.len()),
        });
        if self.i < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek_tok() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<Token, ExprError> {
        if self.peek_tok() == t {
            Ok(self.bump())
        } else {
            let span = self.span();
            let found = self.peek_tok().describe();
            Err(syntax(span, format!("expected {what}, found {found}")))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.toks.get(self.i).map(|t| &t.tok), Some(Tok::Newline)) {
            self.i += 1;
        }
    }

    fn mk(&self, kind: ExprKind, span: Span, depth: u32) -> Result<Expr, ExprError> {
        if depth > self.limits.max_depth {
            return Err(ExprError::at(
                ErrorKind::Limit,
                span,
                format!(
                    "the expression is nested more than {} levels deep",
                    self.limits.max_depth
                ),
            ));
        }
        Ok(Expr { kind, span, depth })
    }

    fn program(&mut self) -> Result<Program, ExprError> {
        let mut stmts = Vec::new();
        loop {
            while matches!(self.peek_tok(), Tok::Newline | Tok::Semi) {
                self.bump();
            }
            if matches!(self.peek_tok(), Tok::Eof) {
                break;
            }
            stmts.push(self.stmt()?);
            match self.peek_tok() {
                Tok::Newline | Tok::Semi | Tok::Eof => {}
                other => {
                    let msg = format!("unexpected {}", other.describe());
                    let hint = if matches!(other, Tok::Num(_) | Tok::Ident(_) | Tok::LParen) {
                        " (write `*` between the factors if you mean a product)"
                    } else {
                        ""
                    };
                    return Err(syntax(self.span(), format!("{msg}{hint}")));
                }
            }
        }
        Ok(Program { stmts })
    }

    fn check_name(&self, name: &str, span: Span) -> Result<(), ExprError> {
        if KEYWORDS.contains(&name) {
            return Err(syntax(
                span,
                format!("`{name}` is a keyword and cannot be a name"),
            ));
        }
        Ok(())
    }

    fn stmt(&mut self) -> Result<Stmt, ExprError> {
        if let Tok::Ident(name) = self.peek_tok().clone() {
            if matches!(self.peek_at(1), Tok::Assign) {
                let name_span = self.span();
                self.check_name(&name, name_span)?;
                self.bump();
                self.bump();
                self.skip_newlines();
                let value = self.expr(0)?;
                return Ok(Stmt::Assign {
                    name,
                    name_span,
                    value,
                });
            }
            if matches!(self.peek_at(1), Tok::LParen) {
                if let Some(params) = self.lookahead_params() {
                    let name_span = self.span();
                    self.check_name(&name, name_span)?;
                    // consume name ( params ) =
                    let consumed = 2 + params.len() + params.len().saturating_sub(1) + 2;
                    for _ in 0..consumed {
                        self.bump();
                    }
                    for p in &params {
                        self.check_name(p, name_span)?;
                    }
                    for (i, p) in params.iter().enumerate() {
                        if params[..i].contains(p) {
                            return Err(syntax(
                                name_span,
                                format!("parameter `{p}` appears twice"),
                            ));
                        }
                    }
                    self.skip_newlines();
                    let body = self.expr(0)?;
                    return Ok(Stmt::FuncDef {
                        name,
                        name_span,
                        params,
                        body,
                    });
                }
            }
        }
        Ok(Stmt::Expr(self.expr(0)?))
    }

    /// If the tokens at the cursor are `name ( a, b ) =`, returns the parameter names.
    fn lookahead_params(&self) -> Option<Vec<String>> {
        let mut j = 2; // after name and `(`
        let mut params = Vec::new();
        if matches!(self.peek_at(j), Tok::RParen) {
            j += 1;
        } else {
            loop {
                match self.peek_at(j) {
                    Tok::Ident(p) => params.push(p.clone()),
                    _ => return None,
                }
                j += 1;
                match self.peek_at(j) {
                    Tok::Comma => j += 1,
                    Tok::RParen => {
                        j += 1;
                        break;
                    }
                    _ => return None,
                }
            }
        }
        matches!(self.peek_at(j), Tok::Assign).then_some(params)
    }

    // ---- expressions ----

    fn expr(&mut self, depth: u32) -> Result<Expr, ExprError> {
        if depth > self.limits.max_nesting {
            return Err(ExprError::at(
                ErrorKind::Limit,
                self.span(),
                format!(
                    "brackets and calls are nested more than {} levels deep",
                    self.limits.max_nesting
                ),
            ));
        }
        let mut lhs = self.or_level(depth)?;
        loop {
            let is_conv = match self.peek_tok() {
                Tok::Arrow => true,
                Tok::Ident(s) => s == "to",
                _ => false,
            };
            if !is_conv {
                return Ok(lhs);
            }
            self.bump();
            let (unit, uspan) = self.unit_slice()?;
            let span = lhs.span.to(uspan);
            let dep = lhs.depth + 1;
            lhs = self.mk(ExprKind::Convert(Box::new(lhs), unit), span, dep)?;
        }
    }

    fn unit_slice(&mut self) -> Result<(Unit, Span), ExprError> {
        let mut depth = 0usize;
        let mut first: Option<usize> = None;
        let mut last_end = 0usize;
        let mut prev: Option<Tok> = None;
        loop {
            let t = self.toks.get(self.i).map_or(Tok::Eof, |t| t.tok.clone());
            let span = self.toks.get(self.i).map_or(Span::default(), |t| t.span);
            let ok = match &t {
                Tok::Ident(s) => !KEYWORDS.contains(&s.as_str()),
                Tok::Star | Tok::Slash | Tok::Caret | Tok::Sup(_) => prev.is_some(),
                Tok::LParen => {
                    depth += 1;
                    true
                }
                Tok::RParen => {
                    if depth == 0 {
                        false
                    } else {
                        depth -= 1;
                        true
                    }
                }
                Tok::Num(_) => matches!(
                    prev,
                    None | Some(
                        Tok::Slash | Tok::Caret | Tok::LParen | Tok::Minus | Tok::Plus | Tok::Star
                    )
                ),
                Tok::Minus | Tok::Plus => {
                    matches!(prev, Some(Tok::Caret | Tok::LParen))
                }
                _ => false,
            };
            if !ok {
                break;
            }
            if first.is_none() {
                first = Some(span.start);
            }
            last_end = span.end;
            prev = Some(t);
            self.i += 1;
        }
        let Some(start) = first else {
            return Err(syntax(self.span(), "expected a unit after `->`"));
        };
        let text = self.src.get(start..last_end).unwrap_or("");
        let span = Span::new(start, last_end);
        match Unit::parse(text) {
            Ok(u) => Ok((u, span)),
            Err(e) => {
                let mut err = ExprError::from(e.clone());
                err.span = Some(match &e {
                    nodalia_units::UnitError::Syntax { pos, .. } => {
                        Span::new(start + pos, start + pos + 1)
                    }
                    _ => span,
                });
                Err(err)
            }
        }
    }

    fn binary(&self, op: BinOp, l: Expr, r: Expr) -> Result<Expr, ExprError> {
        let span = l.span.to(r.span);
        let dep = l.depth.max(r.depth) + 1;
        self.mk(ExprKind::Binary(op, Box::new(l), Box::new(r)), span, dep)
    }

    fn or_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut l = self.and_level(d)?;
        loop {
            let is_or = match self.peek_tok() {
                Tok::OrOr => true,
                Tok::Ident(s) => s == "or",
                _ => false,
            };
            if !is_or {
                return Ok(l);
            }
            self.bump();
            self.skip_newlines();
            let r = self.and_level(d)?;
            l = self.binary(BinOp::Or, l, r)?;
        }
    }

    fn and_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut l = self.not_level(d)?;
        loop {
            let is_and = match self.peek_tok() {
                Tok::AndAnd => true,
                Tok::Ident(s) => s == "and",
                _ => false,
            };
            if !is_and {
                return Ok(l);
            }
            self.bump();
            self.skip_newlines();
            let r = self.not_level(d)?;
            l = self.binary(BinOp::And, l, r)?;
        }
    }

    fn not_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let is_not = match self.peek_tok() {
            Tok::Bang => true,
            Tok::Ident(s) => s == "not",
            _ => false,
        };
        if is_not {
            let start = self.bump().span;
            let inner = self.not_level(d + 1)?;
            let span = start.to(inner.span);
            let dep = inner.depth + 1;
            return self.mk(ExprKind::Unary(UnOp::Not, Box::new(inner)), span, dep);
        }
        self.cmp_level(d)
    }

    fn cmp_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut l = self.range_level(d)?;
        loop {
            let op = match self.peek_tok() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => return Ok(l),
            };
            self.bump();
            self.skip_newlines();
            let r = self.range_level(d)?;
            l = self.binary(op, l, r)?;
        }
    }

    fn range_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let a = self.add_level(d)?;
        if !matches!(self.peek_tok(), Tok::Colon) || self.colon_is_index_end() {
            return Ok(a);
        }
        self.bump();
        let b = self.add_level(d)?;
        if matches!(self.peek_tok(), Tok::Colon) {
            self.bump();
            let c = self.add_level(d)?;
            let span = a.span.to(c.span);
            let dep = a.depth.max(b.depth).max(c.depth) + 1;
            return self.mk(
                ExprKind::Range(Box::new(a), Some(Box::new(b)), Box::new(c)),
                span,
                dep,
            );
        }
        let span = a.span.to(b.span);
        let dep = a.depth.max(b.depth) + 1;
        self.mk(ExprKind::Range(Box::new(a), None, Box::new(b)), span, dep)
    }

    /// A `:` directly followed by `,` or `]` is not a range operator.
    fn colon_is_index_end(&self) -> bool {
        matches!(self.peek_at(1), Tok::Comma | Tok::RBracket)
    }

    fn add_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut l = self.mul_level(d)?;
        loop {
            let op = match self.peek_tok() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(l),
            };
            self.bump();
            self.skip_newlines();
            let r = self.mul_level(d)?;
            l = self.binary(op, l, r)?;
        }
    }

    fn mul_level(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut l = self.unary(d)?;
        loop {
            let op = match self.peek_tok() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::DotStar => BinOp::ElemMul,
                Tok::DotSlash => BinOp::ElemDiv,
                _ => return Ok(l),
            };
            self.bump();
            self.skip_newlines();
            let r = self.unary(d)?;
            l = self.binary(op, l, r)?;
        }
    }

    fn unary(&mut self, d: u32) -> Result<Expr, ExprError> {
        if d > self.limits.max_nesting {
            return Err(ExprError::at(
                ErrorKind::Limit,
                self.span(),
                format!(
                    "operators are nested more than {} levels deep",
                    self.limits.max_nesting
                ),
            ));
        }
        let op = match self.peek_tok() {
            Tok::Minus => Some(UnOp::Neg),
            Tok::Plus => Some(UnOp::Pos),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.bump().span;
            let inner = self.unary(d + 1)?;
            let span = start.to(inner.span);
            let dep = inner.depth + 1;
            return self.mk(ExprKind::Unary(op, Box::new(inner)), span, dep);
        }
        self.term(d)
    }

    fn can_juxtapose(e: &Expr) -> bool {
        matches!(
            e.kind,
            ExprKind::Number(_)
                | ExprKind::Imag(_)
                | ExprKind::Paren(_)
                | ExprKind::Implicit(..)
                | ExprKind::Vector(_)
                | ExprKind::Matrix(_)
        )
    }

    fn term(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut l = self.power(d)?;
        loop {
            let starts_factor = match self.peek_tok() {
                Tok::Ident(s) => !KEYWORDS.contains(&s.as_str()),
                Tok::LParen => true,
                _ => false,
            };
            if !starts_factor || !Parser::can_juxtapose(&l) {
                return Ok(l);
            }
            let r = self.power(d)?;
            let span = l.span.to(r.span);
            let dep = l.depth.max(r.depth) + 1;
            l = self.mk(ExprKind::Implicit(Box::new(l), Box::new(r)), span, dep)?;
        }
    }

    fn power(&mut self, d: u32) -> Result<Expr, ExprError> {
        let base = self.postfix(d)?;
        let op = match self.peek_tok() {
            Tok::Caret => BinOp::Pow,
            Tok::DotCaret => BinOp::ElemPow,
            _ => return Ok(base),
        };
        self.bump();
        let exp = self.pow_operand(d + 1)?;
        self.binary(op, base, exp)
    }

    fn pow_operand(&mut self, d: u32) -> Result<Expr, ExprError> {
        if d > self.limits.max_nesting {
            return Err(ExprError::at(
                ErrorKind::Limit,
                self.span(),
                format!(
                    "operators are nested more than {} levels deep",
                    self.limits.max_nesting
                ),
            ));
        }
        let op = match self.peek_tok() {
            Tok::Minus => Some(UnOp::Neg),
            Tok::Plus => Some(UnOp::Pos),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.bump().span;
            let inner = self.pow_operand(d + 1)?;
            let span = start.to(inner.span);
            let dep = inner.depth + 1;
            return self.mk(ExprKind::Unary(op, Box::new(inner)), span, dep);
        }
        self.power(d)
    }

    fn postfix(&mut self, d: u32) -> Result<Expr, ExprError> {
        let mut e = self.primary(d)?;
        loop {
            match self.peek_tok() {
                Tok::Sup(n) => {
                    let n = *n;
                    let sp = self.bump().span;
                    let lit = Expr {
                        kind: ExprKind::Number(f64::from(n)),
                        span: sp,
                        depth: 1,
                    };
                    e = self.binary(BinOp::Pow, e, lit)?;
                }
                Tok::LBracket
                    if matches!(
                        e.kind,
                        ExprKind::Ident(_)
                            | ExprKind::Call(..)
                            | ExprKind::Index(..)
                            | ExprKind::Paren(_)
                    ) =>
                {
                    self.bump();
                    self.group += 1;
                    let mut args = Vec::new();
                    loop {
                        args.push(self.index_arg(d + 1)?);
                        if self.eat(&Tok::Comma) {
                            continue;
                        }
                        break;
                    }
                    let close = self.expect(&Tok::RBracket, "`]` or `,` in an index")?;
                    self.group -= 1;
                    if args.len() > 2 {
                        return Err(syntax(close.span, "an index has at most two subscripts"));
                    }
                    let span = e.span.to(close.span);
                    let dep = args.iter().map(|a| a.depth).max().unwrap_or(0).max(e.depth) + 1;
                    e = self.mk(ExprKind::Index(Box::new(e), args), span, dep)?;
                }
                _ => return Ok(e),
            }
        }
    }

    fn index_arg(&mut self, d: u32) -> Result<Expr, ExprError> {
        if matches!(self.peek_tok(), Tok::Colon)
            && matches!(self.peek_at(1), Tok::Comma | Tok::RBracket)
        {
            let sp = self.bump().span;
            return Ok(Expr {
                kind: ExprKind::ColonAll,
                span: sp,
                depth: 1,
            });
        }
        self.expr(d)
    }

    fn primary(&mut self, d: u32) -> Result<Expr, ExprError> {
        let t = self.bump();
        match t.tok {
            Tok::Num(v) => Ok(Expr {
                kind: ExprKind::Number(v),
                span: t.span,
                depth: 1,
            }),
            Tok::Imag(v) => Ok(Expr {
                kind: ExprKind::Imag(v),
                span: t.span,
                depth: 1,
            }),
            Tok::Str(s) => Ok(Expr {
                kind: ExprKind::Str(s),
                span: t.span,
                depth: 1,
            }),
            Tok::Ident(name) => {
                if name == "true" || name == "false" {
                    return Ok(Expr {
                        kind: ExprKind::Bool(name == "true"),
                        span: t.span,
                        depth: 1,
                    });
                }
                if KEYWORDS.contains(&name.as_str()) {
                    return Err(syntax(
                        t.span,
                        format!("unexpected keyword `{name}`; it needs an expression before it"),
                    ));
                }
                if matches!(self.peek_tok(), Tok::LParen) {
                    self.bump();
                    self.group += 1;
                    let mut args = Vec::new();
                    if !matches!(self.peek_tok(), Tok::RParen) {
                        loop {
                            args.push(self.expr(d + 1)?);
                            if self.eat(&Tok::Comma) {
                                continue;
                            }
                            break;
                        }
                    }
                    let close = self.expect(&Tok::RParen, "`)` or `,` in the argument list")?;
                    self.group -= 1;
                    let span = t.span.to(close.span);
                    let dep = args.iter().map(|a| a.depth).max().unwrap_or(0) + 1;
                    return self.mk(ExprKind::Call(name, t.span, args), span, dep);
                }
                Ok(Expr {
                    kind: ExprKind::Ident(name),
                    span: t.span,
                    depth: 1,
                })
            }
            Tok::LParen => {
                self.group += 1;
                let inner = self.expr(d + 1)?;
                let close = self.expect(&Tok::RParen, "`)`")?;
                self.group -= 1;
                let span = t.span.to(close.span);
                let dep = inner.depth + 1;
                self.mk(ExprKind::Paren(Box::new(inner)), span, dep)
            }
            Tok::LBracket => self.bracket(t.span, d),
            Tok::Eof | Tok::Newline => Err(syntax(
                t.span,
                "the expression is incomplete: expected a value",
            )),
            other => Err(syntax(
                t.span,
                format!("expected a value, found {}", other.describe()),
            )),
        }
    }

    fn bracket(&mut self, open: Span, d: u32) -> Result<Expr, ExprError> {
        self.group += 1;
        let mut rows: Vec<Vec<Expr>> = vec![Vec::new()];
        if matches!(self.peek_tok(), Tok::RBracket) {
            return Err(syntax(open, "an empty `[]` is not allowed"));
        }
        loop {
            let e = self.expr(d + 1)?;
            self.elements += 1;
            if self.elements > self.limits.max_elements {
                return Err(ExprError::at(
                    ErrorKind::Limit,
                    e.span,
                    format!(
                        "more than {} elements in literals",
                        self.limits.max_elements
                    ),
                ));
            }
            if let Some(r) = rows.last_mut() {
                r.push(e);
            }
            match self.peek_tok() {
                Tok::Comma => {
                    self.bump();
                }
                Tok::Semi => {
                    self.bump();
                    rows.push(Vec::new());
                }
                Tok::RBracket => break,
                other => {
                    let msg = format!("expected `,`, `;` or `]`, found {}", other.describe());
                    return Err(syntax(self.span(), msg));
                }
            }
            // allow a trailing separator before `]`
            if matches!(self.peek_tok(), Tok::RBracket) {
                break;
            }
        }
        let close = self.bump();
        self.group -= 1;
        let span = open.to(close.span);
        if rows.last().is_some_and(Vec::is_empty) {
            rows.pop();
        }
        let width = rows.first().map_or(0, Vec::len);
        for (i, r) in rows.iter().enumerate() {
            if r.len() != width {
                return Err(syntax(
                    span,
                    format!(
                        "matrix rows must have the same length (row 1 has {width}, row {} has {})",
                        i + 1,
                        r.len()
                    ),
                ));
            }
        }
        if rows.len() == 1 {
            let items = rows.into_iter().next().unwrap_or_default();
            let dep = items.iter().map(|a| a.depth).max().unwrap_or(0) + 1;
            return self.mk(ExprKind::Vector(items), span, dep);
        }
        let dep = rows.iter().flatten().map(|a| a.depth).max().unwrap_or(0) + 1;
        self.mk(ExprKind::Matrix(rows), span, dep)
    }
}
