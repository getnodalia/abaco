// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
#![allow(clippy::needless_range_loop)]
//! Ábaco Script: a matrix language in the spirit of MATLAB, with scripts, functions,
//! control flow, cells, structs, function handles and a large numerical library, all in
//! Rust with no external numerical engine.

pub mod ast;
pub mod builtins;
pub mod display;
pub mod error;
pub mod index;
pub mod interp;
pub mod lexer;
pub mod linalg;
pub mod ops;
pub mod parser;
pub mod plot;
pub mod value;

pub use error::{Error, Result, Span};
pub use interp::{Event, Interp, Limits};
pub use value::{Arr, Value};
