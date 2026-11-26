// SPDX-License-Identifier: MIT OR Apache-2.0
//! The error type of the crate.

use thiserror::Error;

/// Everything that can go wrong parsing units or doing arithmetic on quantities.
///
/// Messages are written for the person who typed the expression: they name the units
/// involved and, where useful, what to do instead.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum UnitError {
    /// The unit expression is empty or only whitespace.
    #[error("the unit expression is empty")]
    Empty,
    /// The unit expression exceeds [`MAX_UNIT_LEN`](crate::MAX_UNIT_LEN).
    #[error("the unit expression is longer than {0} characters")]
    TooLong(usize),
    /// The unit expression is malformed. `pos` is a byte offset into it.
    #[error("invalid unit expression at position {pos}: {message}")]
    Syntax {
        /// Byte offset of the problem.
        pos: usize,
        /// What was wrong.
        message: String,
    },
    /// A name is not a known unit.
    #[error("unknown unit `{name}`{}", suggestion.as_ref().map(|s| format!("; did you mean `{s}`?")).unwrap_or_default())]
    UnknownUnit {
        /// The name that was not found.
        name: String,
        /// The closest known unit symbol, if one is near enough.
        suggestion: Option<String>,
    },
    /// Two quantities (or a quantity and a unit) have different dimensions.
    #[error("{0}")]
    Mismatch(String),
    /// An operation is not allowed on temperatures on an offset scale (degC, degF).
    #[error("{0}")]
    Affine(String),
    /// A dimension exponent left the representable range.
    #[error("dimension exponent overflow in {0}")]
    Overflow(&'static str),
    /// The operation is not defined for these arguments.
    #[error("{0}")]
    Domain(String),
}

impl UnitError {
    pub(crate) fn syntax(pos: usize, message: impl Into<String>) -> UnitError {
        UnitError::Syntax {
            pos,
            message: message.into(),
        }
    }
}
