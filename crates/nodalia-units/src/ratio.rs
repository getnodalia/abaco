// SPDX-License-Identifier: MIT OR Apache-2.0
//! Small exact rationals used for dimension exponents.

use std::fmt;

/// A rational number `num/den` in lowest terms with `den > 0`, stored in `i16`.
///
/// Used for the exponents of a [`Dimension`](crate::Dimension) so that roots such as
/// `sqrt(Hz)` (dimension `T^-1/2`) are exact. All arithmetic is checked: operations
/// return `None` when the result does not fit in `i16`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ratio {
    num: i16,
    den: i16,
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

impl Ratio {
    /// Zero.
    pub const ZERO: Ratio = Ratio { num: 0, den: 1 };
    /// One.
    pub const ONE: Ratio = Ratio { num: 1, den: 1 };

    /// The integer `n`.
    pub const fn int(n: i16) -> Ratio {
        Ratio { num: n, den: 1 }
    }

    /// Builds `num/den` in lowest terms. `None` if `den == 0` or the reduced value
    /// does not fit in `i16`.
    pub fn new(num: i64, den: i64) -> Option<Ratio> {
        if den == 0 {
            return None;
        }
        let g = gcd(num, den);
        let (mut n, mut d) = if g == 0 { (0, 1) } else { (num / g, den / g) };
        if d < 0 {
            n = n.checked_neg()?;
            d = d.checked_neg()?;
        }
        Some(Ratio {
            num: i16::try_from(n).ok()?,
            den: i16::try_from(d).ok()?,
        })
    }

    /// Numerator (sign carrier).
    pub fn numer(self) -> i16 {
        self.num
    }

    /// Denominator, always positive.
    pub fn denom(self) -> i16 {
        self.den
    }

    /// `true` for zero.
    pub fn is_zero(self) -> bool {
        self.num == 0
    }

    /// `true` when the denominator is 1.
    pub fn is_integer(self) -> bool {
        self.den == 1
    }

    /// The value as `f64`.
    pub fn to_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Exact sum, `None` on overflow.
    pub fn checked_add(self, o: Ratio) -> Option<Ratio> {
        let n = i64::from(self.num) * i64::from(o.den) + i64::from(o.num) * i64::from(self.den);
        Ratio::new(n, i64::from(self.den) * i64::from(o.den))
    }

    /// Exact difference, `None` on overflow.
    pub fn checked_sub(self, o: Ratio) -> Option<Ratio> {
        self.checked_add(o.checked_neg()?)
    }

    /// Exact product, `None` on overflow.
    pub fn checked_mul(self, o: Ratio) -> Option<Ratio> {
        Ratio::new(
            i64::from(self.num) * i64::from(o.num),
            i64::from(self.den) * i64::from(o.den),
        )
    }

    /// Exact quotient, `None` on overflow or division by zero.
    pub fn checked_div(self, o: Ratio) -> Option<Ratio> {
        Ratio::new(
            i64::from(self.num) * i64::from(o.den),
            i64::from(self.den) * i64::from(o.num),
        )
    }

    /// Negation, `None` only for `i16::MIN`.
    pub fn checked_neg(self) -> Option<Ratio> {
        Some(Ratio {
            num: self.num.checked_neg()?,
            den: self.den,
        })
    }

    /// Parses a decimal such as `-2`, `0.5` or `1.25` into an exact ratio.
    /// Returns `None` for anything that is not a short plain decimal.
    pub fn from_decimal_str(s: &str) -> Option<Ratio> {
        let s = s.trim();
        let (neg, body) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (int_part, frac_part) = match body.split_once('.') {
            Some((i, f)) => (i, f),
            None => (body, ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        if !int_part.chars().all(|c| c.is_ascii_digit())
            || !frac_part.chars().all(|c| c.is_ascii_digit())
            || frac_part.len() > 4
            || int_part.len() > 5
        {
            return None;
        }
        let mut den: i64 = 1;
        for _ in 0..frac_part.len() {
            den *= 10;
        }
        let digits = format!("{int_part}{frac_part}");
        let n: i64 = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        Ratio::new(if neg { -n } else { n }, den)
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl PartialOrd for Ratio {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ratio {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (i64::from(self.num) * i64::from(other.den))
            .cmp(&(i64::from(other.num) * i64::from(self.den)))
    }
}

impl Ratio {
    /// The exact small rational equal to `x` (denominator up to 24), if there is one.
    /// `0.5` gives `1/2`, `2.0` gives `2`, `0.3333333333333333` gives `1/3`; `0.1234` gives `None`.
    pub fn from_f64(x: f64) -> Option<Ratio> {
        if !x.is_finite() || x.abs() > 1000.0 {
            return None;
        }
        for den in 1..=24i64 {
            let n = x * den as f64;
            let r = n.round();
            if (n - r).abs() <= 1e-9 * (1.0 + n.abs()) {
                return Ratio::new(r as i64, den);
            }
        }
        None
    }
}
