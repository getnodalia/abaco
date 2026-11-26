// SPDX-License-Identifier: MIT OR Apache-2.0
//! Errors with a position.

use nodalia_units::UnitError;
use thiserror::Error;

/// A byte range in the source text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// A span from `start` to `end`.
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// The smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// What kind of problem an [`ExprError`] reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    /// The text is not a valid expression.
    Syntax,
    /// A name is not defined.
    Name,
    /// An operand has the wrong type (a string where a number is needed).
    Type,
    /// Units or dimensions do not agree.
    Dimension,
    /// The argument is outside the function's domain (`ln(-1)`, division by zero).
    Domain,
    /// Wrong number of arguments.
    Arity,
    /// An index or shape is out of range or mismatched.
    Index,
    /// A resource limit was hit (source size, nesting, elements, steps, recursion).
    Limit,
    /// A numerical routine failed (singular matrix, non-finite result).
    Numeric,
}

/// An error in parsing or evaluating an expression, with the byte range it refers to.
#[derive(Clone, PartialEq, Debug, Error)]
#[error("{message}")]
pub struct ExprError {
    /// The category.
    pub kind: ErrorKind,
    /// A message a person can act on.
    pub message: String,
    /// Where in the source, if known.
    pub span: Option<Span>,
}

impl ExprError {
    /// An error without a position (one is attached as it propagates out of the evaluator).
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> ExprError {
        ExprError {
            kind,
            message: message.into(),
            span: None,
        }
    }

    /// An error at `span`.
    pub fn at(kind: ErrorKind, span: Span, message: impl Into<String>) -> ExprError {
        ExprError {
            kind,
            message: message.into(),
            span: Some(span),
        }
    }

    /// Attaches `span` unless the error already has a (more precise) one.
    pub fn or_span(mut self, span: Span) -> ExprError {
        if self.span.is_none() {
            self.span = Some(span);
        }
        self
    }

    /// Moves the span by `offset` bytes (used to translate positions inside a sub-string).
    pub fn shifted(mut self, offset: usize) -> ExprError {
        if let Some(s) = &mut self.span {
            s.start += offset;
            s.end += offset;
        }
        self
    }

    /// 1-based `(line, column)` of the start of the span in `src`; columns count characters.
    pub fn line_col(&self, src: &str) -> Option<(usize, usize)> {
        let span = self.span?;
        Some(line_col(src, span.start))
    }

    /// A multi-line rendering with the source line and a caret underline.
    pub fn render(&self, src: &str) -> String {
        let Some(span) = self.span else {
            return format!("error: {}", self.message);
        };
        let (line, col) = line_col(src, span.start);
        let text = src.lines().nth(line - 1).unwrap_or("");
        let width = src
            .get(span.start..span.end.min(src.len()))
            .map_or(1, |s| s.lines().next().unwrap_or("").chars().count())
            .max(1);
        let gutter = line.to_string().len();
        format!(
            "error: {}\n{:>g$}--> {line}:{col}\n{:>g$} |\n{line} | {text}\n{:>g$} | {}{}",
            self.message,
            "",
            "",
            "",
            " ".repeat(col - 1),
            "^".repeat(width),
            g = gutter,
        )
    }
}

/// 1-based line and column (in characters) of byte offset `pos`.
pub fn line_col(src: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(src.len());
    let mut line = 1;
    let mut col = 1;
    for (i, c) in src.char_indices() {
        if i >= pos {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

impl From<UnitError> for ExprError {
    fn from(e: UnitError) -> ExprError {
        let kind = match &e {
            UnitError::Empty | UnitError::TooLong(_) | UnitError::Syntax { .. } => {
                ErrorKind::Syntax
            }
            UnitError::UnknownUnit { .. } => ErrorKind::Name,
            UnitError::Mismatch(_) | UnitError::Affine(_) => ErrorKind::Dimension,
            UnitError::Overflow(_) | UnitError::Domain(_) => ErrorKind::Domain,
        };
        ExprError::new(kind, e.to_string())
    }
}
