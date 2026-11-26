// SPDX-License-Identifier: MIT OR Apache-2.0
//! The one error type of the numerical engine.
//!
//! A numerical routine never hands back a plausible-looking wrong answer: a singular system, a
//! non-finite value or a failure to converge is an error that carries the numbers that show it.

use thiserror::Error;

/// Why a numerical routine could not give a trustworthy answer.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum NumError {
    /// A matrix or vector does not have the shape the operation needs.
    #[error("dimension mismatch: {0}")]
    DimensionMismatch(String),
    /// The matrix is singular: elimination found no usable pivot in this column.
    #[error("the matrix is singular (no pivot in column {column})")]
    Singular {
        /// Zero-based column in which the pivot was exactly zero.
        column: usize,
    },
    /// A Cholesky factorisation met a leading minor that is not positive.
    #[error("the matrix is not positive definite (failed at row {row})")]
    NotPositiveDefinite {
        /// Zero-based row at which the diagonal was not positive.
        row: usize,
    },
    /// A least-squares problem whose columns are linearly dependent.
    #[error("the matrix is rank deficient (column {column} is dependent on the others)")]
    RankDeficient {
        /// Zero-based column found dependent.
        column: usize,
    },
    /// An input or an intermediate value was NaN or infinite.
    #[error("non-finite value in {0}")]
    NonFinite(&'static str),
    /// An iterative method did not reach its tolerance.
    #[error("no convergence after {iterations} iterations (last residual {residual:e})")]
    NoConvergence {
        /// Iterations spent.
        iterations: usize,
        /// Size of the last residual or step.
        residual: f64,
    },
    /// A bracketing root finder was given an interval whose ends have the same sign.
    #[error("the interval does not bracket a root (f(a) and f(b) have the same sign)")]
    NoBracket,
    /// The step size of an ODE integration fell below what the arithmetic can resolve.
    #[error("step size became too small at t = {t:e}; the problem may be stiff or singular")]
    StepTooSmall {
        /// Time at which the integration gave up.
        t: f64,
    },
    /// The caller asked for the computation to stop.
    #[error("cancelled")]
    Cancelled,
    /// The request itself is invalid (empty input, negative tolerance, unsorted times…).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The request is larger than the engine accepts.
    #[error("too large: {0}")]
    TooLarge(String),
}

/// Shorthand for results of this crate.
pub type Result<T> = std::result::Result<T, NumError>;
