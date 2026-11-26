// SPDX-License-Identifier: MIT OR Apache-2.0
//! The data of a document: cells, settings, results and errors.

use abaco_expr::{ExprError, Span, Value};
use thiserror::Error;

/// Most cells one document may hold.
pub const MAX_CELLS: usize = 20_000;
/// Longest source of one math cell, in bytes.
pub const MAX_SOURCE_BYTES: usize = 100_000;
/// Longest text of one text cell, in bytes.
pub const MAX_TEXT_BYTES: usize = 1_000_000;
/// Largest cell id (ids stay exactly representable as JSON numbers in any consumer).
pub const MAX_CELL_ID: u64 = (1 << 53) - 1;
/// Significant digits shown for numbers unless the document says otherwise.
pub const DEFAULT_SIGNIFICANT_DIGITS: usize = 6;

/// A cell's identity. It never changes while the cell exists and is never reused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CellId(pub u64);

impl std::fmt::Display for CellId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// What a cell holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CellKind {
    /// Source in the expression language.
    Math {
        /// The text of the cell.
        source: String,
    },
    /// Markdown prose; never evaluated.
    Text {
        /// The markdown.
        markdown: String,
    },
}

/// One cell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    /// Stable identity.
    pub id: CellId,
    /// The content.
    pub kind: CellKind,
}

/// Document-wide settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// Significant digits when a number is shown (1 to 17).
    pub significant_digits: usize,
    /// When `true`, using a name before the cell that defines it (further down the page)
    /// is flagged with a warning. Evaluation order follows dependencies either way.
    pub strict_order: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            significant_digits: DEFAULT_SIGNIFICANT_DIGITS,
            strict_order: false,
        }
    }
}

impl Settings {
    /// Checks the ranges.
    pub fn validate(&self) -> Result<(), DocError> {
        if !(1..=17).contains(&self.significant_digits) {
            return Err(DocError::InvalidSetting(format!(
                "significant digits must be between 1 and 17, got {}",
                self.significant_digits
            )));
        }
        Ok(())
    }
}

/// Why a cell has no value.
#[derive(Clone, PartialEq, Debug)]
pub enum CellError {
    /// The cell's own text is wrong (syntax, name, units, domain, limit...).
    Expr(ExprError),
    /// The cell is part of a circular definition.
    Cycle {
        /// The cells in the cycle, starting with the first on the page.
        path: Vec<CellId>,
        /// For example ``circular reference: `a` -> `b` -> `a` ``.
        message: String,
    },
    /// A name the cell uses comes from a cell that failed.
    Dependency {
        /// The cell that failed.
        cell: CellId,
        /// The name that cannot be computed.
        name: String,
        /// A message a person can act on.
        message: String,
    },
    /// A name the cell defines is already defined by an earlier cell.
    Duplicate {
        /// The name.
        name: String,
        /// The earlier cell that owns it.
        first: CellId,
        /// A message a person can act on.
        message: String,
    },
}

impl CellError {
    /// The message.
    pub fn message(&self) -> &str {
        match self {
            CellError::Expr(e) => &e.message,
            CellError::Cycle { message, .. }
            | CellError::Dependency { message, .. }
            | CellError::Duplicate { message, .. } => message,
        }
    }

    /// Where in the cell's source the problem is, if it has a position.
    pub fn span(&self) -> Option<Span> {
        match self {
            CellError::Expr(e) => e.span,
            _ => None,
        }
    }

    /// A short machine-readable category.
    pub fn code(&self) -> &'static str {
        match self {
            CellError::Expr(e) => match e.kind {
                abaco_expr::ErrorKind::Syntax => "syntax",
                abaco_expr::ErrorKind::Name => "name",
                abaco_expr::ErrorKind::Type => "type",
                abaco_expr::ErrorKind::Dimension => "dimension",
                abaco_expr::ErrorKind::Domain => "domain",
                abaco_expr::ErrorKind::Arity => "arity",
                abaco_expr::ErrorKind::Index => "index",
                abaco_expr::ErrorKind::Limit => "limit",
                abaco_expr::ErrorKind::Numeric => "numeric",
            },
            CellError::Cycle { .. } => "cycle",
            CellError::Dependency { .. } => "dependency",
            CellError::Duplicate { .. } => "duplicate",
        }
    }
}

/// What a successful math cell produced.
#[derive(Clone, PartialEq, Debug)]
pub struct CellOutput {
    /// The value of the last statement; `None` for a definition-only or empty cell.
    pub value: Option<Value>,
    /// Names the cell defines.
    pub defined: Vec<String>,
}

/// The outcome of evaluating a math cell.
#[derive(Clone, PartialEq, Debug)]
pub struct CellResult {
    /// The value or the error.
    pub outcome: Result<CellOutput, CellError>,
    /// Warnings that did not stop the cell (ill-conditioned system, name used too early).
    pub warnings: Vec<String>,
}

/// Mistakes in how a document is edited.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum DocError {
    /// No cell has this id.
    #[error("there is no cell {0}")]
    UnknownCell(CellId),
    /// The cell is not of the kind the operation needs.
    #[error("cell {0} is not a {1} cell")]
    WrongKind(CellId, &'static str),
    /// The position is past the end.
    #[error("position {index} is out of range: the document has {len} cells")]
    BadIndex {
        /// The position asked for.
        index: usize,
        /// The number of cells.
        len: usize,
    },
    /// A size limit.
    #[error("{0}")]
    TooLarge(String),
    /// A setting is outside its range.
    #[error("{0}")]
    InvalidSetting(String),
}
