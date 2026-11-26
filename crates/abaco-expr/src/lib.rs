// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! The Ábaco expression language: a hand-written lexer and parser, an AST with source spans
//! and an evaluator over real, complex, quantity (number with unit), boolean, string, vector
//! and matrix values.
//!
//! ```
//! use abaco_expr::{eval_str, Env};
//! let mut env = Env::new();
//! let r = eval_str("10 kg * 9.81 m/s^2 -> N", &mut env).unwrap();
//! assert_eq!(r.value.unwrap().to_string(), "98.1 N");
//! ```

/// This crate's version, for the engine information a saved document records.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod ast;
pub mod builtins;
pub mod error;
pub mod eval;
pub mod lexer;
pub mod limits;
pub mod linalg;
pub mod ops;
pub mod parser;
pub mod value;

pub use ast::{Expr, ExprKind, Program, Stmt};
pub use builtins::{is_builtin, BUILTINS};
pub use error::{line_col, ErrorKind, ExprError, Span};
pub use eval::{eval_program, eval_str, eval_str_with, Env, Evaluation, UserFn};
pub use limits::Limits;
pub use parser::{parse, parse_with, KEYWORDS};
pub use value::{Mat, Value};
