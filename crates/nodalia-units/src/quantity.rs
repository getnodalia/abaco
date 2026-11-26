// SPDX-License-Identifier: MIT OR Apache-2.0
//! Quantities: a number with a dimension, stored in coherent SI.

use crate::dimension::Dimension;
use crate::error::UnitError;
use crate::fmt::format_sig;
use crate::ratio::Ratio;
use crate::unit::Unit;
use std::cmp::Ordering;
use std::fmt;

/// A physical quantity: an `f64` value in coherent SI units, its [`Dimension`], and
/// (for display) the unit the person wrote.
///
/// # Arithmetic
///
/// * `add`/`sub` need equal dimensions; the message names both
///   (`cannot add m and s (length and time)`). The result keeps the left operand's unit.
/// * `mul`/`div` combine dimensions and units (`kg` times `m/s^2` displays as `kg*m/s^2`).
///   A product or quotient whose dimension cancels, with a unit on both sides, is a plain
///   number.
/// * `pow` takes an exact rational exponent (`sqrt` of `m^2` is `m`; `sqrt(m)` is
///   `m^(1/2)`).
///
/// # Temperatures
///
/// `K` and the `delta_*` units are ordinary quantities. A value in `degC` or `degF` is an
/// absolute *point* on an offset scale; it is stored as kelvin plus the display unit, and:
///
/// * point `-` point is a plain `K` difference; point `±` plain temperature is a point;
///   plain `-` point is a plain value;
/// * point `+` point, products, quotients, powers and negation of a point are errors
///   (`20 degC + 20 degC`, `2 * 20 degC`): convert to `K` first;
/// * comparisons work on the absolute value.
#[derive(Clone, Debug, PartialEq)]
pub struct Quantity {
    value: f64,
    dim: Dimension,
    unit: Option<Unit>,
}

fn not_finite(op: &str) -> UnitError {
    UnitError::Domain(format!(
        "the result of {op} is not a finite number (overflow or invalid operand)"
    ))
}

fn kelvin() -> Option<Unit> {
    Unit::from_name("K").ok()
}

impl Quantity {
    /// A pure number.
    pub fn dimensionless(value: f64) -> Quantity {
        Quantity {
            value,
            dim: Dimension::NONE,
            unit: None,
        }
    }

    /// A quantity from a value already in coherent SI units.
    pub fn from_si(value: f64, dim: Dimension) -> Quantity {
        Quantity {
            value,
            dim,
            unit: None,
        }
    }

    /// A quantity from a value expressed in `unit` (offset scales convert correctly:
    /// `Quantity::new(20.0, degC)` is 293.15 K).
    pub fn new(value: f64, unit: &Unit) -> Quantity {
        Quantity {
            value: (value + unit.zero()) * unit.scale(),
            dim: *unit.dimension(),
            unit: if unit.is_one() {
                None
            } else {
                Some(unit.clone())
            },
        }
    }

    /// Parses `"9.81 m/s^2"`, `"5 km"`, `"20 degC"`, `"1e3 Pa"`; a missing number means 1
    /// (`"kg"`); a missing unit means a pure number.
    pub fn parse(src: &str) -> Result<Quantity, UnitError> {
        let s = src.trim();
        if s.is_empty() {
            return Err(UnitError::Empty);
        }
        let (num, rest) = split_number(s);
        let value = match num {
            Some(text) => text
                .replace('_', "")
                .parse::<f64>()
                .map_err(|_| UnitError::syntax(0, format!("invalid number `{text}`")))?,
            None => 1.0,
        };
        if !value.is_finite() {
            return Err(UnitError::syntax(0, "the number is too large"));
        }
        let rest = rest.trim();
        if rest.is_empty() {
            return Ok(Quantity::dimensionless(value));
        }
        Ok(Quantity::new(value, &Unit::parse(rest)?))
    }

    /// The value in coherent SI units.
    pub fn si_value(&self) -> f64 {
        self.value
    }

    /// The dimension.
    pub fn dimension(&self) -> &Dimension {
        &self.dim
    }

    /// The unit the quantity is displayed in, if one was given.
    pub fn display_unit(&self) -> Option<&Unit> {
        self.unit.as_ref()
    }

    /// `true` for a value on an offset temperature scale (`degC`, `degF`).
    pub fn is_temperature_point(&self) -> bool {
        self.unit.as_ref().is_some_and(Unit::is_affine)
    }

    /// The numeric value in the display unit (SI value if there is none).
    pub fn display_value(&self) -> f64 {
        match &self.unit {
            Some(u) => self.value / u.scale() - u.zero(),
            None => self.value,
        }
    }

    /// The numeric value expressed in `unit`.
    pub fn value_in(&self, unit: &Unit) -> Result<f64, UnitError> {
        if unit.dimension() != &self.dim {
            return Err(UnitError::Mismatch(format!(
                "cannot convert {} to {} ({} and {})",
                self.operand(),
                unit,
                self.dim.describe(),
                unit.dimension().describe()
            )));
        }
        let v = self.value / unit.scale() - unit.zero();
        if v.is_finite() {
            Ok(v)
        } else {
            Err(not_finite("the conversion"))
        }
    }

    /// Converts for display in `unit` (`x -> unit`).
    pub fn to(&self, unit: &Unit) -> Result<Quantity, UnitError> {
        self.value_in(unit)?;
        if unit.is_delta() && self.is_temperature_point() {
            return Err(UnitError::Affine(format!(
                "{} is a temperature difference unit; a temperature on {} cannot be converted to it (subtract two temperatures to get a difference)",
                unit,
                self.operand()
            )));
        }
        Ok(Quantity {
            value: self.value,
            dim: self.dim,
            unit: if unit.is_one() {
                None
            } else {
                Some(unit.clone())
            },
        })
    }

    /// Re-expresses in a derived SI unit when the dimension has one (`kg*m/s^2` becomes
    /// `N`), else in the coherent SI base units. Offset temperatures and pure numbers are
    /// returned unchanged.
    pub fn simplify(&self) -> Quantity {
        if self.is_temperature_point() || self.dim.is_dimensionless() {
            return self.clone();
        }
        const DERIVED: [&str; 14] = [
            "N", "Pa", "J", "W", "C", "V", "F", "Ω", "S", "Wb", "T", "H", "Hz", "kat",
        ];
        for name in DERIVED {
            if let Ok(u) = Unit::from_name(name) {
                if u.dimension() == &self.dim {
                    return Quantity {
                        unit: Some(u),
                        ..self.clone()
                    };
                }
            }
        }
        Quantity {
            unit: Some(Unit::coherent_si(&self.dim)),
            ..self.clone()
        }
    }

    /// Formats with `sig` significant digits: `98.1 kg*m/s^2`, `20 degC`.
    pub fn format(&self, sig: usize) -> String {
        let v = format_sig(self.display_value(), sig);
        match &self.unit {
            Some(u) => format!("{v} {u}"),
            None if self.dim.is_dimensionless() => v,
            None => format!("{v} {}", Unit::coherent_si(&self.dim)),
        }
    }

    /// `true` when both quantities have the same dimension and values within the relative
    /// tolerance `rel` (absolute `rel` near zero).
    pub fn approx_eq(&self, o: &Quantity, rel: f64) -> bool {
        self.dim == o.dim
            && (self.value - o.value).abs() <= rel * self.value.abs().max(o.value.abs()).max(1e-300)
    }

    fn operand(&self) -> String {
        match &self.unit {
            Some(u) => u.to_string(),
            None if self.dim.is_dimensionless() => "a number".to_string(),
            None => Unit::coherent_si(&self.dim).to_string(),
        }
    }

    fn mismatch(&self, o: &Quantity, verb: &str) -> UnitError {
        UnitError::Mismatch(format!(
            "cannot {verb} {} and {} ({} and {})",
            self.operand(),
            o.operand(),
            self.dim.describe(),
            o.dim.describe()
        ))
    }

    fn point_error(&self, what: &str) -> UnitError {
        UnitError::Affine(format!(
            "cannot {what} a temperature on the offset scale {}; convert to K first (`-> K`)",
            self.operand()
        ))
    }

    fn finish(
        value: f64,
        dim: Dimension,
        unit: Option<Unit>,
        op: &str,
    ) -> Result<Quantity, UnitError> {
        if !value.is_finite() {
            return Err(not_finite(op));
        }
        let unit = unit.filter(|u| !u.is_one());
        Ok(Quantity { value, dim, unit })
    }

    /// Sum; dimensions must match.
    pub fn add(&self, o: &Quantity) -> Result<Quantity, UnitError> {
        if self.dim != o.dim {
            return Err(self.mismatch(o, "add"));
        }
        let unit = match (self.is_temperature_point(), o.is_temperature_point()) {
            (true, true) => {
                return Err(UnitError::Affine(format!(
                    "cannot add two temperatures on offset scales ({} + {}); add a difference in K instead",
                    self.operand(),
                    o.operand()
                )))
            }
            (true, false) => self.unit.clone(),
            (false, true) => o.unit.clone(),
            (false, false) => self.sum_unit(o),
        };
        Quantity::finish(self.value + o.value, self.dim, unit, "addition")
    }

    /// Difference; dimensions must match.
    pub fn sub(&self, o: &Quantity) -> Result<Quantity, UnitError> {
        if self.dim != o.dim {
            return Err(self.mismatch(o, "subtract"));
        }
        let unit = match (self.is_temperature_point(), o.is_temperature_point()) {
            (true, true) => kelvin(),
            (true, false) => self.unit.clone(),
            (false, true) => self.unit.clone().or_else(kelvin),
            (false, false) => self.sum_unit(o),
        };
        Quantity::finish(self.value - o.value, self.dim, unit, "subtraction")
    }

    fn sum_unit(&self, o: &Quantity) -> Option<Unit> {
        if self.dim.is_dimensionless() && (self.unit.is_none() || o.unit.is_none()) {
            return None;
        }
        self.unit.clone().or_else(|| o.unit.clone())
    }

    /// Negation (an error for an offset temperature).
    pub fn neg(&self) -> Result<Quantity, UnitError> {
        if self.is_temperature_point() {
            return Err(self.point_error("negate"));
        }
        Ok(Quantity {
            value: -self.value,
            ..self.clone()
        })
    }

    /// Absolute value (an error for an offset temperature).
    pub fn abs(&self) -> Result<Quantity, UnitError> {
        if self.is_temperature_point() {
            return Err(self.point_error("take the absolute value of"));
        }
        Ok(Quantity {
            value: self.value.abs(),
            ..self.clone()
        })
    }

    fn unit_or_si(&self) -> Unit {
        match &self.unit {
            Some(u) => u.clone(),
            None => Unit::coherent_si(&self.dim),
        }
    }

    fn combined_unit(
        &self,
        o: &Quantity,
        dim: &Dimension,
        divide: bool,
    ) -> Result<Option<Unit>, UnitError> {
        if self.unit.is_none() && o.unit.is_none() {
            return Ok(None);
        }
        if dim.is_dimensionless() && self.unit.is_some() && o.unit.is_some() {
            return Ok(None);
        }
        let a = self.unit_or_si();
        let b = o.unit_or_si();
        Ok(Some(if divide { a.div(&b)? } else { a.mul(&b)? }))
    }

    /// Product.
    pub fn mul(&self, o: &Quantity) -> Result<Quantity, UnitError> {
        if self.is_temperature_point() {
            return Err(self.point_error("multiply"));
        }
        if o.is_temperature_point() {
            return Err(o.point_error("multiply"));
        }
        let dim = self.dim.mul(&o.dim)?;
        let unit = self.combined_unit(o, &dim, false)?;
        Quantity::finish(self.value * o.value, dim, unit, "multiplication")
    }

    /// Quotient.
    pub fn div(&self, o: &Quantity) -> Result<Quantity, UnitError> {
        if self.is_temperature_point() {
            return Err(self.point_error("divide"));
        }
        if o.is_temperature_point() {
            return Err(o.point_error("divide by"));
        }
        if o.value == 0.0 {
            return Err(UnitError::Domain("division by zero".into()));
        }
        let dim = self.dim.div(&o.dim)?;
        let unit = self.combined_unit(o, &dim, true)?;
        Quantity::finish(self.value / o.value, dim, unit, "division")
    }

    /// Raises to an exact rational power. Fractional powers of negative values are an
    /// error (use complex numbers in the expression layer).
    pub fn pow(&self, e: Ratio) -> Result<Quantity, UnitError> {
        if self.is_temperature_point() {
            return Err(self.point_error("raise to a power"));
        }
        let dim = self.dim.pow(e)?;
        let value = if e.is_integer() {
            self.value.powi(i32::from(e.numer()))
        } else {
            if self.value < 0.0 {
                return Err(UnitError::Domain(format!(
                    "cannot take the {e} power of a negative quantity"
                )));
            }
            self.value.powf(e.to_f64())
        };
        if value.is_nan() && self.value == 0.0 {
            return Err(UnitError::Domain("zero to a negative power".into()));
        }
        let unit = match &self.unit {
            Some(u) => Some(u.pow(e)?),
            None => None,
        };
        Quantity::finish(value, dim, unit, "the power")
    }

    /// Raises to an `f64` power. A dimensioned quantity needs an exponent that is an exact
    /// small rational (`2`, `0.5`, `1/3`); a pure number accepts any real exponent.
    pub fn pow_f64(&self, e: f64) -> Result<Quantity, UnitError> {
        if self.dim.is_dimensionless() && self.unit.is_none() {
            return Quantity::finish(self.value.powf(e), Dimension::NONE, None, "the power");
        }
        match Ratio::from_f64(e) {
            Some(r) => self.pow(r),
            None => Err(UnitError::Domain(format!(
                "a quantity with dimension {} can only be raised to a rational power (got {e})",
                self.dim.describe()
            ))),
        }
    }

    /// Square root; halves the dimension (`sqrt(m)` is `m^(1/2)`).
    pub fn sqrt(&self) -> Result<Quantity, UnitError> {
        self.pow(Ratio::new(1, 2).unwrap_or(Ratio::ONE))
    }

    /// The `n`-th root.
    pub fn root(&self, n: i16) -> Result<Quantity, UnitError> {
        match Ratio::new(1, i64::from(n)) {
            Some(r) => self.pow(r),
            None => Err(UnitError::Domain("the 0th root is undefined".into())),
        }
    }

    /// Orders two quantities of the same dimension by their absolute value.
    pub fn compare(&self, o: &Quantity) -> Result<Ordering, UnitError> {
        if self.dim != o.dim {
            return Err(self.mismatch(o, "compare"));
        }
        self.value
            .partial_cmp(&o.value)
            .ok_or_else(|| UnitError::Domain("cannot compare NaN".into()))
    }
}

impl fmt::Display for Quantity {
    /// 15 significant digits followed by the unit.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format(15))
    }
}

/// Splits a leading decimal number off `s`: `("9.81", " m/s^2")`.
fn split_number(s: &str) -> (Option<&str>, &str) {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits_start = i;
    while matches!(b.get(i), Some(c) if c.is_ascii_digit() || *c == b'_') {
        i += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while matches!(b.get(i), Some(c) if c.is_ascii_digit() || *c == b'_') {
            i += 1;
        }
    }
    if i == digits_start || (i == digits_start + 1 && b.get(digits_start) == Some(&b'.')) {
        return (None, s);
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if matches!(b.get(j), Some(c) if c.is_ascii_digit()) {
            while matches!(b.get(j), Some(c) if c.is_ascii_digit()) {
                j += 1;
            }
            i = j;
        }
    }
    (Some(&s[..i]), &s[i..])
}
