// SPDX-License-Identifier: MIT OR Apache-2.0
//! Runtime values.

use nodalia_units::{format_sig, Quantity};
use num_complex::Complex64;
use serde_json::{json, Value as Json};

/// A dense row-major matrix of scalar values.
#[derive(Clone, Debug, PartialEq)]
pub struct Mat {
    /// Number of rows.
    pub rows: usize,
    /// Number of columns.
    pub cols: usize,
    /// Row-major elements; `data.len() == rows * cols`.
    pub data: Vec<Value>,
}

impl Mat {
    /// The element at 0-based `(r, c)`.
    pub fn get(&self, r: usize, c: usize) -> Option<&Value> {
        if r < self.rows && c < self.cols {
            self.data.get(r * self.cols + c)
        } else {
            None
        }
    }
}

/// A value an expression can produce.
///
/// Vectors and matrices hold only scalars (`Real`, `Complex`, `Quantity`). A `Quantity` that
/// is a plain number (no dimension, no display unit) is always normalised to `Real`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A real number.
    Real(f64),
    /// A complex number (stays complex even when the imaginary part is zero).
    Complex(Complex64),
    /// A number with a dimension and display unit.
    Quantity(Box<Quantity>),
    /// `true` or `false`.
    Bool(bool),
    /// Text.
    Str(String),
    /// A list of scalars (treated as a column by matrix products).
    Vector(Vec<Value>),
    /// A matrix of scalars.
    Matrix(Mat),
}

impl Value {
    /// Wraps a quantity, turning a plain number into `Real`.
    pub fn quantity(q: Quantity) -> Value {
        if q.dimension().is_dimensionless() && q.display_unit().is_none() {
            Value::Real(q.si_value())
        } else {
            Value::Quantity(Box::new(q))
        }
    }

    /// The kind of value, for messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Real(_) => "a number",
            Value::Complex(_) => "a complex number",
            Value::Quantity(_) => "a quantity",
            Value::Bool(_) => "a boolean",
            Value::Str(_) => "a string",
            Value::Vector(_) => "a vector",
            Value::Matrix(_) => "a matrix",
        }
    }

    /// `true` for `Real`, `Complex` and `Quantity`.
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            Value::Real(_) | Value::Complex(_) | Value::Quantity(_)
        )
    }

    /// The value as a quantity when it is a real or a quantity.
    pub fn as_quantity(&self) -> Option<Quantity> {
        match self {
            Value::Real(x) => Some(Quantity::dimensionless(*x)),
            Value::Quantity(q) => Some((**q).clone()),
            _ => None,
        }
    }

    /// Formats on one line with `sig` significant digits: `98.1 N`, `3 + 4i`,
    /// `[1, 2; 3, 4]`.
    pub fn display(&self, sig: usize) -> String {
        match self {
            Value::Real(x) => format_sig(*x, sig),
            Value::Complex(z) => format_complex(*z, sig),
            Value::Quantity(q) => q.format(sig),
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => format!("{s:?}"),
            Value::Vector(v) => {
                let parts: Vec<String> = v.iter().map(|x| x.display(sig)).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Matrix(m) => {
                let rows: Vec<String> = (0..m.rows)
                    .map(|r| {
                        let parts: Vec<String> = (0..m.cols)
                            .map(|c| m.get(r, c).map_or(String::new(), |x| x.display(sig)))
                            .collect();
                        parts.join(", ")
                    })
                    .collect();
                format!("[{}]", rows.join("; "))
            }
        }
    }

    /// A JSON description: `{"type": ..., ...}` with a `"text"` rendering at `sig` digits.
    pub fn to_json(&self, sig: usize) -> Json {
        match self {
            Value::Real(x) => json!({"type": "real", "value": x, "text": self.display(sig)}),
            Value::Complex(z) => {
                json!({"type": "complex", "re": z.re, "im": z.im, "text": self.display(sig)})
            }
            Value::Quantity(q) => json!({
                "type": "quantity",
                "value": q.display_value(),
                "unit": q.display_unit().map(ToString::to_string).unwrap_or_default(),
                "si": q.si_value(),
                "dimension": q.dimension().to_string(),
                "text": self.display(sig),
            }),
            Value::Bool(b) => json!({"type": "bool", "value": b, "text": self.display(sig)}),
            Value::Str(s) => json!({"type": "string", "value": s, "text": s}),
            Value::Vector(v) => json!({
                "type": "vector",
                "items": v.iter().map(|x| x.to_json(sig)).collect::<Vec<_>>(),
                "text": self.display(sig),
            }),
            Value::Matrix(m) => json!({
                "type": "matrix",
                "rows": m.rows,
                "cols": m.cols,
                "items": (0..m.rows).map(|r| {
                    (0..m.cols)
                        .filter_map(|c| m.get(r, c).map(|x| x.to_json(sig)))
                        .collect::<Vec<_>>()
                }).collect::<Vec<_>>(),
                "text": self.display(sig),
            }),
        }
    }
}

fn format_complex(z: Complex64, sig: usize) -> String {
    let re = format_sig(z.re, sig);
    let im_abs = format_sig(z.im.abs(), sig);
    let im = if im_abs == "1" {
        "i".to_string()
    } else {
        format!("{im_abs}i")
    };
    if z.re == 0.0 && z.im != 0.0 {
        return if z.im < 0.0 { format!("-{im}") } else { im };
    }
    if z.im < 0.0 || (z.im == 0.0 && z.im.is_sign_negative()) {
        format!("{re} - {im}")
    } else {
        format!("{re} + {im}")
    }
}

impl std::fmt::Display for Value {
    /// 15 significant digits.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display(15))
    }
}
