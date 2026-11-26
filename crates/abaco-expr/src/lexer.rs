// SPDX-License-Identifier: MIT OR Apache-2.0
//! The lexer.

use crate::error::{ErrorKind, ExprError, Span};

/// A token.
#[derive(Clone, PartialEq, Debug)]
pub enum Tok {
    Num(f64),
    Imag(f64),
    Str(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    DotStar,
    DotSlash,
    DotCaret,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    Assign,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Arrow,
    AndAnd,
    OrOr,
    Bang,
    Newline,
    Sup(i32),
    Eof,
}

impl Tok {
    /// A short description for error messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Num(_) | Tok::Imag(_) => "a number".into(),
            Tok::Str(_) => "a string".into(),
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Plus => "`+`".into(),
            Tok::Minus => "`-`".into(),
            Tok::Star => "`*`".into(),
            Tok::Slash => "`/`".into(),
            Tok::Caret => "`^`".into(),
            Tok::DotStar => "`.*`".into(),
            Tok::DotSlash => "`./`".into(),
            Tok::DotCaret => "`.^`".into(),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::LBracket => "`[`".into(),
            Tok::RBracket => "`]`".into(),
            Tok::Comma => "`,`".into(),
            Tok::Semi => "`;`".into(),
            Tok::Colon => "`:`".into(),
            Tok::Assign => "`=`".into(),
            Tok::EqEq => "`==`".into(),
            Tok::NotEq => "`!=`".into(),
            Tok::Lt => "`<`".into(),
            Tok::Le => "`<=`".into(),
            Tok::Gt => "`>`".into(),
            Tok::Ge => "`>=`".into(),
            Tok::Arrow => "`->`".into(),
            Tok::AndAnd => "`&&`".into(),
            Tok::OrOr => "`||`".into(),
            Tok::Bang => "`!`".into(),
            Tok::Newline => "the end of the line".into(),
            Tok::Sup(_) => "a superscript".into(),
            Tok::Eof => "the end of the input".into(),
        }
    }
}

/// A token with its position.
#[derive(Clone, PartialEq, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

const SUP_DIGITS: &str = "⁰¹²³⁴⁵⁶⁷⁸⁹";

fn sup_digit(c: char) -> Option<i32> {
    SUP_DIGITS.chars().position(|d| d == c).map(|i| i as i32)
}

fn is_ident_start(c: char) -> bool {
    (c.is_alphabetic() && sup_digit(c).is_none()) || matches!(c, '_' | '°' | '′' | '″' | '℃' | '℉')
}

fn is_ident_part(c: char) -> bool {
    (c.is_alphanumeric() || c == '_') && sup_digit(c).is_none()
}

fn err(span: Span, msg: impl Into<String>) -> ExprError {
    ExprError::at(ErrorKind::Syntax, span, msg)
}

/// Splits `src` into tokens. Always ends with [`Tok::Eof`].
pub fn lex(src: &str) -> Result<Vec<Token>, ExprError> {
    let mut out: Vec<Token> = Vec::new();
    let mut it = src.char_indices().peekable();
    let end_of = |pos: usize, c: char| pos + c.len_utf8();
    while let Some((pos, c)) = it.next() {
        let mut end = end_of(pos, c);
        let tok = match c {
            '\n' => {
                if matches!(out.last().map(|t| &t.tok), Some(Tok::Newline)) {
                    continue;
                }
                Tok::Newline
            }
            c if c.is_whitespace() => continue,
            '#' => {
                while let Some(&(_, n)) = it.peek() {
                    if n == '\n' {
                        break;
                    }
                    it.next();
                }
                continue;
            }
            '+' => Tok::Plus,
            '-' | '−' => {
                if c == '-' && matches!(it.peek(), Some(&(_, '>'))) {
                    it.next();
                    end += 1;
                    Tok::Arrow
                } else {
                    Tok::Minus
                }
            }
            '→' => Tok::Arrow,
            '*' | '×' | '·' | '⋅' => Tok::Star,
            '/' | '÷' => Tok::Slash,
            '^' => Tok::Caret,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            ',' => Tok::Comma,
            ';' => Tok::Semi,
            ':' => Tok::Colon,
            '=' => {
                if matches!(it.peek(), Some(&(_, '='))) {
                    it.next();
                    end += 1;
                    Tok::EqEq
                } else {
                    Tok::Assign
                }
            }
            '!' => {
                if matches!(it.peek(), Some(&(_, '='))) {
                    it.next();
                    end += 1;
                    Tok::NotEq
                } else {
                    Tok::Bang
                }
            }
            '≠' => Tok::NotEq,
            '<' => {
                if matches!(it.peek(), Some(&(_, '='))) {
                    it.next();
                    end += 1;
                    Tok::Le
                } else {
                    Tok::Lt
                }
            }
            '≤' => Tok::Le,
            '>' => {
                if matches!(it.peek(), Some(&(_, '='))) {
                    it.next();
                    end += 1;
                    Tok::Ge
                } else {
                    Tok::Gt
                }
            }
            '≥' => Tok::Ge,
            '&' => {
                if matches!(it.peek(), Some(&(_, '&'))) {
                    it.next();
                    end += 1;
                    Tok::AndAnd
                } else {
                    return Err(err(
                        Span::new(pos, end),
                        "unexpected `&`; use `&&` or `and`",
                    ));
                }
            }
            '|' => {
                if matches!(it.peek(), Some(&(_, '|'))) {
                    it.next();
                    end += 1;
                    Tok::OrOr
                } else {
                    return Err(err(Span::new(pos, end), "unexpected `|`; use `||` or `or`"));
                }
            }
            '.' => match it.peek().copied() {
                Some((_, '*')) => {
                    it.next();
                    end += 1;
                    Tok::DotStar
                }
                Some((_, '/')) => {
                    it.next();
                    end += 1;
                    Tok::DotSlash
                }
                Some((_, '^')) => {
                    it.next();
                    end += 1;
                    Tok::DotCaret
                }
                Some((_, d)) if d.is_ascii_digit() => {
                    let (tok, e) = lex_number(src, pos, &mut it)?;
                    end = e;
                    tok
                }
                _ => return Err(err(Span::new(pos, end), "unexpected `.`")),
            },
            '0'..='9' => {
                let (tok, e) = lex_number(src, pos, &mut it)?;
                end = e;
                tok
            }
            '"' => {
                let mut s = String::new();
                let mut closed = false;
                while let Some((p, n)) = it.next() {
                    end = end_of(p, n);
                    match n {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match it.next() {
                            Some((p2, e)) => {
                                end = end_of(p2, e);
                                match e {
                                    'n' => s.push('\n'),
                                    't' => s.push('\t'),
                                    '"' => s.push('"'),
                                    '\\' => s.push('\\'),
                                    other => {
                                        return Err(err(
                                            Span::new(p, end),
                                            format!("unknown escape `\\{other}` in string"),
                                        ))
                                    }
                                }
                            }
                            None => break,
                        },
                        other => s.push(other),
                    }
                }
                if !closed {
                    return Err(err(Span::new(pos, end), "unterminated string"));
                }
                Tok::Str(s)
            }
            c if c == '⁻' || sup_digit(c).is_some() => {
                let neg = c == '⁻';
                let mut v: i32 = sup_digit(c).unwrap_or(0);
                let mut digits = usize::from(!neg);
                while let Some(&(p, n)) = it.peek() {
                    if let Some(d) = sup_digit(n) {
                        v = v.saturating_mul(10).saturating_add(d);
                        digits += 1;
                        end = end_of(p, n);
                        it.next();
                    } else {
                        break;
                    }
                }
                if digits == 0 {
                    return Err(err(
                        Span::new(pos, end),
                        "a superscript exponent needs digits",
                    ));
                }
                Tok::Sup(if neg { -v } else { v })
            }
            c if is_ident_start(c) => {
                let mut s = String::from(c);
                while let Some(&(p, n)) = it.peek() {
                    if is_ident_part(n) {
                        s.push(n);
                        end = end_of(p, n);
                        it.next();
                    } else {
                        break;
                    }
                }
                Tok::Ident(s)
            }
            other => {
                return Err(err(
                    Span::new(pos, end),
                    format!("unexpected character `{other}`"),
                ))
            }
        };
        out.push(Token {
            tok,
            span: Span::new(pos, end),
        });
    }
    out.push(Token {
        tok: Tok::Eof,
        span: Span::new(src.len(), src.len()),
    });
    Ok(out)
}

/// Lexes a number starting at byte `start` (its first char already consumed from `it`).
fn lex_number(
    src: &str,
    start: usize,
    it: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Result<(Tok, usize), ExprError> {
    let first = src[start..].chars().next().unwrap_or('0');
    let mut end = start + first.len_utf8();
    let mut text = String::new();
    text.push(first);
    let digits = |it: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
                  text: &mut String,
                  end: &mut usize| {
        while let Some(&(p, n)) = it.peek() {
            if n.is_ascii_digit() {
                text.push(n);
                *end = p + 1;
                it.next();
            } else if n == '_' {
                // an underscore is a separator only between digits
                let after = src[p + 1..].chars().next();
                if after.is_some_and(|a| a.is_ascii_digit()) {
                    *end = p + 1;
                    it.next();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    };
    if first != '.' {
        digits(it, &mut text, &mut end);
    }
    if first == '.' || matches!(it.peek(), Some(&(_, '.'))) {
        let dot_pos = if first == '.' { start } else { end };
        let next = src[dot_pos + 1..].chars().next();
        if next.is_some_and(|c| c.is_ascii_digit()) {
            if first != '.' {
                it.next();
                text.push('.');
                end = dot_pos + 1;
            }
            digits(it, &mut text, &mut end);
        }
    }
    // exponent only when digits follow, so `2e` is 2 times the constant `e`
    if matches!(it.peek(), Some(&(_, 'e' | 'E'))) {
        let rest = &src[end..];
        let mut chars = rest.chars();
        chars.next();
        let mut look = chars.clone();
        let sign = matches!(look.next(), Some('+' | '-'));
        let after = if sign { chars.nth(1) } else { chars.next() };
        if after.is_some_and(|c| c.is_ascii_digit()) {
            let (p, e) = it.next().unwrap_or((end, 'e'));
            text.push(e);
            end = p + 1;
            if sign {
                if let Some((p, s)) = it.next() {
                    text.push(s);
                    end = p + 1;
                }
            }
            digits(it, &mut text, &mut end);
        }
    }
    let value: f64 = text
        .parse()
        .map_err(|_| err(Span::new(start, end), format!("invalid number `{text}`")))?;
    if !value.is_finite() {
        return Err(err(Span::new(start, end), "the number is too large"));
    }
    // imaginary literal: `3i`, `2.5j` (not `3in`)
    if let Some(&(p, c)) = it.peek() {
        if c == 'i' || c == 'j' {
            let after = src[p + 1..].chars().next();
            if !after.is_some_and(is_ident_part) {
                it.next();
                return Ok((Tok::Imag(value), p + 1));
            }
        }
    }
    Ok((Tok::Num(value), end))
}
