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
//! Ábaco Formats: the file formats of MATLAB and Simulink, read and written in Rust with
//! nothing but this crate: DEFLATE and ZIP, XML, HDF5, MAT-files 4, 5 and 7.3, and the `.mdl`
//! and `.slx` model files. Every input is hostile: sizes, depths and counts are capped and every
//! failure is an error with a message, never a panic.

pub mod deflate;
pub mod error;
pub mod mat;

pub use error::{FormatError, Result};
