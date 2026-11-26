// SPDX-License-Identifier: MIT OR Apache-2.0
//! Physical dimensions: exponents of the seven SI base dimensions.

use crate::error::UnitError;
use crate::ratio::Ratio;
use std::fmt;

/// Symbols of the seven base dimensions, in storage order.
pub const BASE_SYMBOLS: [&str; 7] = ["L", "M", "T", "I", "Th", "N", "J"];

/// Exponents of the seven SI base dimensions: length `L`, mass `M`, time `T`,
/// electric current `I`, thermodynamic temperature `Th`, amount of substance `N`
/// and luminous intensity `J`.
///
/// Exponents are exact [`Ratio`]s, so `sqrt` of a dimension is exact whenever the
/// result is representable. Plane and solid angle are dimensionless (`rad` and `sr`
/// are units with the dimension of a pure number).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Dimension {
    exp: [Ratio; 7],
}

impl Dimension {
    /// The dimension of a pure number.
    pub const NONE: Dimension = Dimension::ints([0; 7]);
    /// Length.
    pub const LENGTH: Dimension = Dimension::ints([1, 0, 0, 0, 0, 0, 0]);
    /// Mass.
    pub const MASS: Dimension = Dimension::ints([0, 1, 0, 0, 0, 0, 0]);
    /// Time.
    pub const TIME: Dimension = Dimension::ints([0, 0, 1, 0, 0, 0, 0]);
    /// Electric current.
    pub const CURRENT: Dimension = Dimension::ints([0, 0, 0, 1, 0, 0, 0]);
    /// Thermodynamic temperature.
    pub const TEMPERATURE: Dimension = Dimension::ints([0, 0, 0, 0, 1, 0, 0]);
    /// Amount of substance.
    pub const AMOUNT: Dimension = Dimension::ints([0, 0, 0, 0, 0, 1, 0]);
    /// Luminous intensity.
    pub const LUMINOUS: Dimension = Dimension::ints([0, 0, 0, 0, 0, 0, 1]);

    /// Builds a dimension from integer exponents `[L, M, T, I, Th, N, J]`.
    pub const fn ints(e: [i16; 7]) -> Dimension {
        Dimension {
            exp: [
                Ratio::int(e[0]),
                Ratio::int(e[1]),
                Ratio::int(e[2]),
                Ratio::int(e[3]),
                Ratio::int(e[4]),
                Ratio::int(e[5]),
                Ratio::int(e[6]),
            ],
        }
    }

    /// Builds a dimension from exact exponents `[L, M, T, I, Th, N, J]`.
    pub const fn from_ratios(exp: [Ratio; 7]) -> Dimension {
        Dimension { exp }
    }

    /// The exponents `[L, M, T, I, Th, N, J]`.
    pub fn exponents(&self) -> &[Ratio; 7] {
        &self.exp
    }

    /// `true` for the dimension of a pure number.
    pub fn is_dimensionless(&self) -> bool {
        self.exp.iter().all(|r| r.is_zero())
    }

    /// Product of two dimensions (exponents add).
    pub fn mul(&self, o: &Dimension) -> Result<Dimension, UnitError> {
        self.zip(o, "multiplication", Ratio::checked_add)
    }

    /// Quotient of two dimensions (exponents subtract).
    pub fn div(&self, o: &Dimension) -> Result<Dimension, UnitError> {
        self.zip(o, "division", Ratio::checked_sub)
    }

    /// Raises to a rational power (exponents multiply).
    pub fn pow(&self, p: Ratio) -> Result<Dimension, UnitError> {
        let mut out = [Ratio::ZERO; 7];
        for (o, e) in out.iter_mut().zip(self.exp.iter()) {
            *o = e.checked_mul(p).ok_or(UnitError::Overflow("power"))?;
        }
        Ok(Dimension { exp: out })
    }

    /// The `n`-th root (exponents divide by `n`); exact for rational exponents.
    pub fn root(&self, n: i16) -> Result<Dimension, UnitError> {
        if n == 0 {
            return Err(UnitError::Domain("the 0th root is undefined".into()));
        }
        self.pow(Ratio::new(1, i64::from(n)).ok_or(UnitError::Overflow("root"))?)
    }

    /// The reciprocal dimension.
    pub fn recip(&self) -> Result<Dimension, UnitError> {
        self.pow(Ratio::int(-1))
    }

    fn zip(
        &self,
        o: &Dimension,
        what: &'static str,
        f: fn(Ratio, Ratio) -> Option<Ratio>,
    ) -> Result<Dimension, UnitError> {
        let mut out = [Ratio::ZERO; 7];
        for ((r, a), b) in out.iter_mut().zip(self.exp.iter()).zip(o.exp.iter()) {
            *r = f(*a, *b).ok_or(UnitError::Overflow(what))?;
        }
        Ok(Dimension { exp: out })
    }

    /// A human name for common dimensions (`"length"`, `"force"`, ...), if there is one.
    pub fn name(&self) -> Option<&'static str> {
        NAMES.iter().find(|(d, _)| d == self).map(|(_, n)| *n)
    }

    /// The name if it has one, otherwise the exponent formula such as `L M T^-2`.
    pub fn describe(&self) -> String {
        match self.name() {
            Some(n) => n.to_string(),
            None => self.to_string(),
        }
    }
}

impl fmt::Display for Dimension {
    /// Formats as a product of base symbols with exponents, e.g. `L M T^-2`;
    /// a pure number prints as `1`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dimensionless() {
            return write!(f, "1");
        }
        let mut first = true;
        for (sym, e) in BASE_SYMBOLS.iter().zip(self.exp.iter()) {
            if e.is_zero() {
                continue;
            }
            if !first {
                write!(f, " ")?;
            }
            first = false;
            if *e == Ratio::ONE {
                write!(f, "{sym}")?;
            } else if e.is_integer() {
                write!(f, "{sym}^{e}")?;
            } else {
                write!(f, "{sym}^({e})")?;
            }
        }
        Ok(())
    }
}

pub(crate) const fn d(l: i16, m: i16, t: i16, i: i16, th: i16, n: i16, j: i16) -> Dimension {
    Dimension::ints([l, m, t, i, th, n, j])
}

const NAMES: &[(Dimension, &str)] = &[
    (Dimension::NONE, "dimensionless"),
    (Dimension::LENGTH, "length"),
    (Dimension::MASS, "mass"),
    (Dimension::TIME, "time"),
    (Dimension::CURRENT, "electric current"),
    (Dimension::TEMPERATURE, "temperature"),
    (Dimension::AMOUNT, "amount of substance"),
    (Dimension::LUMINOUS, "luminous intensity"),
    (d(2, 0, 0, 0, 0, 0, 0), "area"),
    (d(3, 0, 0, 0, 0, 0, 0), "volume"),
    (d(1, 0, -1, 0, 0, 0, 0), "velocity"),
    (d(1, 0, -2, 0, 0, 0, 0), "acceleration"),
    (d(0, 0, -1, 0, 0, 0, 0), "frequency"),
    (d(1, 1, -2, 0, 0, 0, 0), "force"),
    (d(-1, 1, -2, 0, 0, 0, 0), "pressure"),
    (d(2, 1, -2, 0, 0, 0, 0), "energy"),
    (d(2, 1, -3, 0, 0, 0, 0), "power"),
    (d(0, 0, 1, 1, 0, 0, 0), "electric charge"),
    (d(2, 1, -3, -1, 0, 0, 0), "voltage"),
    (d(-2, -1, 4, 2, 0, 0, 0), "capacitance"),
    (d(2, 1, -3, -2, 0, 0, 0), "resistance"),
    (d(-2, -1, 3, 2, 0, 0, 0), "conductance"),
    (d(2, 1, -2, -1, 0, 0, 0), "magnetic flux"),
    (d(0, 1, -2, -1, 0, 0, 0), "magnetic flux density"),
    (d(2, 1, -2, -2, 0, 0, 0), "inductance"),
    (d(-3, 1, 0, 0, 0, 0, 0), "mass density"),
    (d(0, 0, -1, 0, 0, 1, 0), "catalytic activity"),
    (d(2, 0, -2, 0, 0, 0, 0), "specific energy"),
    (d(-2, 0, 0, 0, 0, 0, 1), "illuminance"),
    (d(2, 1, -2, 0, -1, 0, 0), "heat capacity"),
    (d(1, 1, -3, 0, -1, 0, 0), "thermal conductivity"),
    (d(0, 1, -2, 0, 0, 0, 0), "surface tension"),
    (d(-1, 1, -1, 0, 0, 0, 0), "dynamic viscosity"),
    (d(2, 0, -1, 0, 0, 0, 0), "kinematic viscosity"),
    (d(1, 1, -1, 0, 0, 0, 0), "momentum"),
    (d(2, 1, -1, 0, 0, 0, 0), "angular momentum"),
    (d(0, 1, -3, 0, 0, 0, 0), "irradiance"),
];
