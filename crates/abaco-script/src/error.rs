// SPDX-License-Identifier: MIT OR Apache-2.0
//! Errors and source positions.

use std::fmt;

/// A byte range in the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// The 1-based line and column of the start in `src`.
    pub fn line_col(self, src: &str) -> (usize, usize) {
        let upto = src.get(..self.start.min(src.len())).unwrap_or("");
        let line = upto.matches('\n').count() + 1;
        let col = upto.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line, col)
    }
}

/// An error raised by parsing or running a script. Catchable by `try`/`catch`.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    /// A MATLAB-style identifier such as `Abaco:index`.
    pub id: String,
    /// What went wrong, in one sentence that says what to do.
    pub message: String,
    /// Where, when known.
    pub span: Option<Span>,
    /// Names of the user functions the error passed through, innermost first.
    pub trace: Vec<String>,
}

/// The result of anything that can fail.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A new error with identifier `id`.
    pub fn new(id: &str, message: impl Into<String>) -> Error {
        Error {
            id: id.to_string(),
            message: message.into(),
            span: None,
            trace: Vec::new(),
        }
    }

    /// A syntax error at `span`.
    pub fn syntax(span: Span, message: impl Into<String>) -> Error {
        Error {
            id: "Abaco:syntax".into(),
            message: message.into(),
            span: Some(span),
            trace: Vec::new(),
        }
    }

    /// Attaches `span` unless the error already has one.
    pub fn or_span(mut self, span: Span) -> Error {
        if self.span.is_none() {
            self.span = Some(span);
        }
        self
    }

    /// `line N: message`, with the function trace, for showing to a person.
    pub fn render(&self, src: &str) -> String {
        let mut out = String::new();
        if let Some(s) = self.span {
            let (l, c) = s.line_col(src);
            out.push_str(&format!("line {l}, column {c}: "));
        }
        out.push_str(&self.message);
        for f in &self.trace {
            out.push_str(&format!("\n  in {f}"));
        }
        out
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Shorthand for a runtime error.
pub fn err<T>(id: &str, message: impl Into<String>) -> Result<T> {
    Err(Error::new(id, message))
}
