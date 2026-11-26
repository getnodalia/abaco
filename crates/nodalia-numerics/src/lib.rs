// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
// Dense linear algebra reads best with explicit indices; iterator rewrites obscure the maths.
#![allow(clippy::needless_range_loop)]
#![doc = include_str!("../README.md")]

pub mod cholesky;
pub mod eigen;
pub mod error;
pub mod lu;
pub mod matrix;
pub mod ode;
pub mod qr;
pub mod quad;
pub mod roots;
pub mod scalar;

pub use error::{NumError, Result};
pub use lu::{solve, Lu};
pub use matrix::Matrix;
pub use num_complex::Complex64;
pub use scalar::Scalar;
