// SPDX-License-Identifier: MIT OR Apache-2.0
//! The lexer. It knows enough context to tell a transpose from a string quote and to keep
//! whether a token had white space before it, which matrix literals need (`[1 -2]`).

use crate::error::{Error, Result, Span};

/// Reserved words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kw {
    If,
    Elseif,
    Else,
    End,
    For,
    While,
    Break,
    Continue,
    Return,
    Function,
    Switch,
    Case,
    Otherwise,
    Try,
    Catch,
    Global,
    Persistent,
}

/// Token kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    Imag(f64),
    Str(String),
    Ident(String),
    Kw(Kw),
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Caret,
    DotStar,
    DotSlash,
    DotBackslash,
    DotCaret,
    Transpose,
    DotTranspose,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    AndAnd,
    OrOr,
    Not,
    At,
    Dot,
    Newline,
    Eof,
}

impl Tok {
    /// A short description for messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Num(_) | Tok::Imag(_) => "a number".into(),
            Tok::Str(_) => "a string".into(),
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Kw(k) => format!("`{}`", kw_text(*k)),
            Tok::Newline => "the end of the line".into(),
            Tok::Eof => "the end of the input".into(),
            other => format!("`{}`", op_text(other)),
        }
    }
}

fn kw_text(k: Kw) -> &'static str {
    match k {
        Kw::If => "if",
        Kw::Elseif => "elseif",
        Kw::Else => "else",
        Kw::End => "end",
        Kw::For => "for",
        Kw::While => "while",
        Kw::Break => "break",
        Kw::Continue => "continue",
        Kw::Return => "return",
        Kw::Function => "function",
        Kw::Switch => "switch",
        Kw::Case => "case",
        Kw::Otherwise => "otherwise",
        Kw::Try => "try",
        Kw::Catch => "catch",
        Kw::Global => "global",
        Kw::Persistent => "persistent",
    }
}

fn op_text(t: &Tok) -> &'static str {
    match t {
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::Slash => "/",
        Tok::Backslash => "\\",
        Tok::Caret => "^",
        Tok::DotStar => ".*",
        Tok::DotSlash => "./",
        Tok::DotBackslash => ".\\",
        Tok::DotCaret => ".^",
        Tok::Transpose => "'",
        Tok::DotTranspose => ".'",
        Tok::LParen => "(",
        Tok::RParen => ")",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::LBrace => "{",
        Tok::RBrace => "}",
        Tok::Comma => ",",
        Tok::Semi => ";",
        Tok::Colon => ":",
        Tok::Assign => "=",
        Tok::PlusAssign => "+=",
        Tok::MinusAssign => "-=",
        Tok::StarAssign => "*=",
        Tok::SlashAssign => "/=",
        Tok::EqEq => "==",
        Tok::Ne => "~=",
        Tok::Lt => "<",
        Tok::Le => "<=",
        Tok::Gt => ">",
        Tok::Ge => ">=",
        Tok::And => "&",
        Tok::Or => "|",
        Tok::AndAnd => "&&",
        Tok::OrOr => "||",
        Tok::Not => "~",
        Tok::At => "@",
        Tok::Dot => ".",
        _ => "?",
    }
}

/// A token with its position and spacing.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    /// What it is.
    pub tok: Tok,
    /// Where it is.
    pub span: Span,
    /// White space (not a newline) came right before it.
    pub space_before: bool,
    /// White space or the end of the line follows it.
    pub space_after: bool,
}

/// The keyword `s` spells, if it is one.
pub fn keyword(s: &str) -> Option<Kw> {
    Some(match s {
        "if" => Kw::If,
        "elseif" => Kw::Elseif,
        "else" => Kw::Else,
        "end" | "endif" | "endfor" | "endwhile" | "endfunction" | "endswitch" | "end_try_catch"
        | "end_unwind_protect" => Kw::End,
        "for" => Kw::For,
        "while" => Kw::While,
        "break" => Kw::Break,
        "continue" => Kw::Continue,
        "return" => Kw::Return,
        "function" => Kw::Function,
        "switch" => Kw::Switch,
        "case" => Kw::Case,
        "otherwise" => Kw::Otherwise,
        "try" => Kw::Try,
        "catch" => Kw::Catch,
        "global" => Kw::Global,
        "persistent" => Kw::Persistent,
        _ => return None,
    })
}

fn ends_value(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Num(_)
            | Tok::Imag(_)
            | Tok::Str(_)
            | Tok::Ident(_)
            | Tok::RParen
            | Tok::RBracket
            | Tok::RBrace
            | Tok::Transpose
            | Tok::DotTranspose
            | Tok::Kw(Kw::End)
    )
}

/// Splits `src` into tokens. Always ends with [`Tok::Eof`].
pub fn lex(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let at = |k: usize| chars.get(k).map(|&(_, c)| c);
    let pos = |k: usize| chars.get(k).map_or(src.len(), |&(p, _)| p);
    let mut out: Vec<Token> = Vec::new();
    // Innermost open bracket: '(' , '[' or '{'.
    let mut stack: Vec<char> = Vec::new();
    let mut i = 0usize;
    let mut space = false;
    let mut line_start = true;
    while i < n {
        let c = chars.get(i).map_or(' ', |&(_, c)| c);
        // Block comments `%{ ... %}` on lines of their own.
        if line_start && (c == '%' || c == '#') && at(i + 1) == Some('{') {
            let rest_blank = {
                let mut k = i + 2;
                while at(k).is_some_and(|x| x == ' ' || x == '\t' || x == '\r') {
                    k += 1;
                }
                at(k).is_none() || at(k) == Some('\n')
            };
            if rest_blank {
                let mut depth = 0usize;
                loop {
                    // read one line
                    let mut k = i;
                    while at(k).is_some_and(|x| x != '\n') {
                        k += 1;
                    }
                    let line: String = (i..k).filter_map(at).collect();
                    let t = line.trim();
                    if t == "%{" || t == "#{" {
                        depth += 1;
                    } else if t == "%}" || t == "#}" {
                        depth = depth.saturating_sub(1);
                    }
                    i = k;
                    if depth == 0 || i >= n {
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        match c {
            ' ' | '\t' | '\r' => {
                space = true;
                i += 1;
                continue;
            }
            '%' | '#' => {
                while at(i).is_some_and(|x| x != '\n') {
                    i += 1;
                }
                continue;
            }
            '.' if at(i + 1) == Some('.') && at(i + 2) == Some('.') => {
                // continuation: skip the rest of the line and the newline
                while at(i).is_some_and(|x| x != '\n') {
                    i += 1;
                }
                i += 1;
                space = true;
                continue;
            }
            _ => {}
        }
        let start = pos(i);
        let mut len = 1usize;
        let tok = match c {
            '\n' => {
                line_start = true;
                if stack.is_empty() {
                    // collapse blank lines
                    if !matches!(out.last().map(|t| &t.tok), Some(Tok::Newline) | None) {
                        push(&mut out, Tok::Newline, start, start + 1, space);
                    }
                    space = false;
                    i += 1;
                    continue;
                }
                // inside `[...]`/`{...}` a newline separates rows; inside `(...)` it is space
                if matches!(stack.last(), Some('[') | Some('{')) {
                    push(&mut out, Tok::Semi, start, start + 1, space);
                }
                space = true;
                i += 1;
                continue;
            }
            '0'..='9' => {
                let (tok, used) = lex_number(src, &chars, i)?;
                len = used;
                tok
            }
            '.' if at(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let (tok, used) = lex_number(src, &chars, i)?;
                len = used;
                tok
            }
            '"' => {
                let (s, used) = lex_dq(src, &chars, i)?;
                len = used;
                Tok::Str(s)
            }
            '\'' => {
                let prev_value = out.last().is_some_and(|t| ends_value(&t.tok));
                let in_list = matches!(stack.last(), Some('[') | Some('{'));
                let transpose = prev_value && !(in_list && space);
                if transpose {
                    Tok::Transpose
                } else {
                    let (s, used) = lex_sq(src, &chars, i)?;
                    len = used;
                    Tok::Str(s)
                }
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut k = i;
                while at(k).is_some_and(|x| x.is_alphanumeric() || x == '_') {
                    k += 1;
                }
                len = k - i;
                let word: String = (i..k).filter_map(at).collect();
                // `x.end`-style fields and keywords after `.` are plain names
                let after_dot = matches!(out.last().map(|t| &t.tok), Some(Tok::Dot));
                match keyword(&word) {
                    Some(kw) if !after_dot => Tok::Kw(kw),
                    _ => Tok::Ident(word),
                }
            }
            '+' => two(&chars, i, '=', Tok::PlusAssign, Tok::Plus, &mut len),
            '-' => two(&chars, i, '=', Tok::MinusAssign, Tok::Minus, &mut len),
            '*' => two(&chars, i, '=', Tok::StarAssign, Tok::Star, &mut len),
            '/' => two(&chars, i, '=', Tok::SlashAssign, Tok::Slash, &mut len),
            '\\' => Tok::Backslash,
            '^' => Tok::Caret,
            '(' => {
                stack.push('(');
                Tok::LParen
            }
            ')' => {
                stack.pop();
                Tok::RParen
            }
            '[' => {
                stack.push('[');
                Tok::LBracket
            }
            ']' => {
                stack.pop();
                Tok::RBracket
            }
            '{' => {
                stack.push('{');
                Tok::LBrace
            }
            '}' => {
                stack.pop();
                Tok::RBrace
            }
            ',' => Tok::Comma,
            ';' => Tok::Semi,
            ':' => Tok::Colon,
            '@' => Tok::At,
            '=' => two(&chars, i, '=', Tok::EqEq, Tok::Assign, &mut len),
            '~' | '!' => two(&chars, i, '=', Tok::Ne, Tok::Not, &mut len),
            '<' => two(&chars, i, '=', Tok::Le, Tok::Lt, &mut len),
            '>' => two(&chars, i, '=', Tok::Ge, Tok::Gt, &mut len),
            '&' => two(&chars, i, '&', Tok::AndAnd, Tok::And, &mut len),
            '|' => two(&chars, i, '|', Tok::OrOr, Tok::Or, &mut len),
            '.' => match at(i + 1) {
                Some('*') => {
                    len = 2;
                    Tok::DotStar
                }
                Some('/') => {
                    len = 2;
                    Tok::DotSlash
                }
                Some('\\') => {
                    len = 2;
                    Tok::DotBackslash
                }
                Some('^') => {
                    len = 2;
                    Tok::DotCaret
                }
                Some('\'') => {
                    len = 2;
                    Tok::DotTranspose
                }
                _ => Tok::Dot,
            },
            other => {
                return Err(Error::syntax(
                    Span::new(start, start + other.len_utf8()),
                    format!("unexpected character `{other}`"),
                ))
            }
        };
        let end = pos(i + len);
        let space_after =
            at(i + len).is_none_or(|x| x == ' ' || x == '\t' || x == '\n' || x == '\r');
        out.push(Token {
            tok,
            span: Span::new(start, end),
            space_before: space,
            space_after,
        });
        space = false;
        line_start = false;
        i += len;
    }
    let end = src.len();
    out.push(Token {
        tok: Tok::Eof,
        span: Span::new(end, end),
        space_before: false,
        space_after: true,
    });
    Ok(out)
}

fn push(out: &mut Vec<Token>, tok: Tok, start: usize, end: usize, space: bool) {
    out.push(Token {
        tok,
        span: Span::new(start, end),
        space_before: space,
        space_after: true,
    });
}

fn two(chars: &[(usize, char)], i: usize, next: char, yes: Tok, no: Tok, len: &mut usize) -> Tok {
    if chars.get(i + 1).is_some_and(|&(_, c)| c == next) {
        *len = 2;
        yes
    } else {
        no
    }
}

fn lex_number(src: &str, chars: &[(usize, char)], i: usize) -> Result<(Tok, usize)> {
    let at = |k: usize| chars.get(k).map(|&(_, c)| c);
    let mut k = i;
    // hexadecimal
    if at(k) == Some('0')
        && matches!(at(k + 1), Some('x') | Some('X'))
        && at(k + 2).is_some_and(|c| c.is_ascii_hexdigit())
    {
        k += 2;
        let mut v = 0f64;
        while let Some(d) = at(k).and_then(|c| c.to_digit(16)) {
            v = v * 16.0 + f64::from(d);
            k += 1;
        }
        return Ok((Tok::Num(v), k - i));
    }
    let mut text = String::new();
    while at(k).is_some_and(|c| c.is_ascii_digit()) {
        text.extend(at(k));
        k += 1;
    }
    if at(k) == Some('.')
        && !matches!(
            at(k + 1),
            Some('*') | Some('/') | Some('\\') | Some('^') | Some('\'')
        )
        && !(at(k + 1) == Some('.') && at(k + 2) == Some('.'))
    {
        text.push('.');
        k += 1;
        while at(k).is_some_and(|c| c.is_ascii_digit()) {
            text.extend(at(k));
            k += 1;
        }
    }
    if matches!(at(k), Some('e') | Some('E') | Some('d') | Some('D')) {
        let sign = matches!(at(k + 1), Some('+') | Some('-'));
        let d = if sign { at(k + 2) } else { at(k + 1) };
        if d.is_some_and(|c| c.is_ascii_digit()) {
            text.push('e');
            k += 1;
            if sign {
                text.extend(at(k));
                k += 1;
            }
            while at(k).is_some_and(|c| c.is_ascii_digit()) {
                text.extend(at(k));
                k += 1;
            }
        }
    }
    let start = chars.get(i).map_or(0, |&(p, _)| p);
    let value: f64 = text.parse().map_err(|_| {
        Error::syntax(
            Span::new(start, start + text.len()),
            format!("invalid number `{text}`"),
        )
    })?;
    if matches!(at(k), Some('i') | Some('j'))
        && !at(k + 1).is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        let _ = src;
        return Ok((Tok::Imag(value), k + 1 - i));
    }
    Ok((Tok::Num(value), k - i))
}

fn lex_sq(src: &str, chars: &[(usize, char)], i: usize) -> Result<(String, usize)> {
    let mut s = String::new();
    let mut k = i + 1;
    loop {
        match chars.get(k).map(|&(_, c)| c) {
            None | Some('\n') => {
                let p = chars.get(i).map_or(0, |&(p, _)| p);
                return Err(Error::syntax(
                    Span::new(p, src.len().min(p + 1)),
                    "unterminated string",
                ));
            }
            Some('\'') => {
                if chars.get(k + 1).is_some_and(|&(_, c)| c == '\'') {
                    s.push('\'');
                    k += 2;
                } else {
                    return Ok((s, k + 1 - i));
                }
            }
            Some(c) => {
                s.push(c);
                k += 1;
            }
        }
    }
}

fn lex_dq(src: &str, chars: &[(usize, char)], i: usize) -> Result<(String, usize)> {
    let mut s = String::new();
    let mut k = i + 1;
    loop {
        match chars.get(k).map(|&(_, c)| c) {
            None | Some('\n') => {
                let p = chars.get(i).map_or(0, |&(p, _)| p);
                return Err(Error::syntax(
                    Span::new(p, src.len().min(p + 1)),
                    "unterminated string",
                ));
            }
            Some('"') => {
                if chars.get(k + 1).is_some_and(|&(_, c)| c == '"') {
                    s.push('"');
                    k += 2;
                } else {
                    return Ok((s, k + 1 - i));
                }
            }
            Some('\\') => {
                let e = chars.get(k + 1).map(|&(_, c)| c);
                match e {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some('\'') => s.push('\''),
                    Some('0') => s.push('\0'),
                    Some(o) => {
                        s.push('\\');
                        s.push(o);
                    }
                    None => s.push('\\'),
                }
                k += 2;
            }
            Some(c) => {
                s.push(c);
                k += 1;
            }
        }
    }
}
