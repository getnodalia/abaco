// SPDX-License-Identifier: MIT OR Apache-2.0
//! The scalar types matrices can hold: `f64` and `Complex64`.

use std::fmt::Debug;
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

use num_complex::Complex64;

/// A field element the dense algorithms work over.
pub trait Scalar:
    Copy
    + Debug
    + PartialEq
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
    + AddAssign
    + SubAssign
    + MulAssign
    + Send
    + Sync
    + 'static
{
    /// Additive identity.
    fn zero() -> Self;
    /// Multiplicative identity.
    fn one() -> Self;
    /// Embeds a real number.
    fn from_f64(x: f64) -> Self;
    /// Modulus (absolute value for reals).
    fn modulus(self) -> f64;
    /// Complex conjugate (identity for reals).
    fn conj(self) -> Self;
    /// Real part.
    fn re(self) -> f64;
    /// True when neither part is NaN or infinite.
    fn is_finite_scalar(self) -> bool;
}

impl Scalar for f64 {
    fn zero() -> Self {
        0.0
    }
    fn one() -> Self {
        1.0
    }
    fn from_f64(x: f64) -> Self {
        x
    }
    fn modulus(self) -> f64 {
        self.abs()
    }
    fn conj(self) -> Self {
        self
    }
    fn re(self) -> f64 {
        self
    }
    fn is_finite_scalar(self) -> bool {
        self.is_finite()
    }
}

impl Scalar for Complex64 {
    fn zero() -> Self {
        Complex64::new(0.0, 0.0)
    }
    fn one() -> Self {
        Complex64::new(1.0, 0.0)
    }
    fn from_f64(x: f64) -> Self {
        Complex64::new(x, 0.0)
    }
    fn modulus(self) -> f64 {
        self.norm()
    }
    fn conj(self) -> Self {
        Complex64::conj(&self)
    }
    fn re(self) -> f64 {
        self.re
    }
    fn is_finite_scalar(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}
