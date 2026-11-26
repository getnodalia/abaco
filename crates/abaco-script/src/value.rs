// SPDX-License-Identifier: MIT OR Apache-2.0
//! Values: two-dimensional arrays of numbers, booleans, text and cells, scalar structs and
//! function handles. Arrays are stored column by column, as in MATLAB.

use crate::ast::Expr;
use crate::error::{err, Result};
use num_complex::Complex64;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A two-dimensional array stored column-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Arr<T> {
    /// Rows.
    pub r: usize,
    /// Columns.
    pub c: usize,
    /// Elements, column by column; `d.len() == r * c`.
    pub d: Vec<T>,
}

impl<T: Clone> Arr<T> {
    /// An array of the given shape; the data length must match.
    pub fn new(r: usize, c: usize, d: Vec<T>) -> Arr<T> {
        debug_assert_eq!(r * c, d.len());
        Arr { r, c, d }
    }

    /// A 1×1 array.
    pub fn scalar(x: T) -> Arr<T> {
        Arr {
            r: 1,
            c: 1,
            d: vec![x],
        }
    }

    /// A 1×n row.
    pub fn row(d: Vec<T>) -> Arr<T> {
        Arr {
            r: 1,
            c: d.len(),
            d,
        }
    }

    /// An n×1 column.
    pub fn col(d: Vec<T>) -> Arr<T> {
        Arr {
            r: d.len(),
            c: 1,
            d,
        }
    }

    /// A 0×0 array.
    pub fn empty() -> Arr<T> {
        Arr {
            r: 0,
            c: 0,
            d: Vec::new(),
        }
    }

    /// An `r`×`c` array filled with `x`.
    pub fn filled(r: usize, c: usize, x: T) -> Arr<T> {
        Arr {
            r,
            c,
            d: vec![x; r * c],
        }
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.d.len()
    }

    /// `true` when there are no elements.
    pub fn is_empty(&self) -> bool {
        self.d.is_empty()
    }

    /// `true` for 1×1.
    pub fn is_scalar(&self) -> bool {
        self.r == 1 && self.c == 1
    }

    /// `true` when one dimension is 1 (and it is not empty).
    pub fn is_vector(&self) -> bool {
        (self.r == 1 || self.c == 1) && !self.d.is_empty()
    }

    /// Element at 0-based `(i, j)`.
    pub fn at(&self, i: usize, j: usize) -> Option<&T> {
        if i < self.r && j < self.c {
            self.d.get(j * self.r + i)
        } else {
            None
        }
    }

    /// The transpose.
    pub fn transposed(&self) -> Arr<T> {
        let mut d = Vec::with_capacity(self.d.len());
        for i in 0..self.r {
            for j in 0..self.c {
                if let Some(x) = self.d.get(j * self.r + i) {
                    d.push(x.clone());
                }
            }
        }
        Arr {
            r: self.c,
            c: self.r,
            d,
        }
    }

    /// Applies `f` to every element.
    pub fn map<U: Clone>(&self, f: impl Fn(&T) -> U) -> Arr<U> {
        Arr {
            r: self.r,
            c: self.c,
            d: self.d.iter().map(f).collect(),
        }
    }

    /// Column `j` as a vector.
    pub fn column(&self, j: usize) -> Vec<T> {
        self.d
            .get(j * self.r..(j + 1) * self.r)
            .map(<[T]>::to_vec)
            .unwrap_or_default()
    }

    /// Row `i` as a vector.
    pub fn row_vec(&self, i: usize) -> Vec<T> {
        (0..self.c).filter_map(|j| self.at(i, j).cloned()).collect()
    }
}

/// An anonymous function or a named handle.
#[derive(Clone, Debug)]
pub enum FnValue {
    /// `@name`: a function by name.
    Named(String),
    /// `@(x) expr` with the variables it captured.
    Anon {
        /// Parameter names.
        params: Vec<String>,
        /// The body.
        body: Arc<Expr>,
        /// Variables captured when it was created.
        captured: BTreeMap<String, Value>,
        /// The source text, for display.
        src: String,
    },
}

impl PartialEq for FnValue {
    fn eq(&self, other: &FnValue) -> bool {
        match (self, other) {
            (FnValue::Named(a), FnValue::Named(b)) => a == b,
            (FnValue::Anon { src: a, .. }, FnValue::Anon { src: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// A scalar struct: ordered named fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructV {
    /// Fields in creation order.
    pub fields: Vec<(String, Value)>,
}

impl StructV {
    /// The field `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Sets or adds a field.
    pub fn set(&mut self, name: &str, v: Value) {
        match self.fields.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = v,
            None => self.fields.push((name.to_string(), v)),
        }
    }

    /// Removes a field.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.fields.len();
        self.fields.retain(|(n, _)| n != name);
        self.fields.len() != before
    }
}

/// A value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Real numbers.
    Num(Arr<f64>),
    /// Complex numbers.
    Cplx(Arr<Complex64>),
    /// Logical values.
    Bool(Arr<bool>),
    /// A character row vector.
    Str(String),
    /// A cell array.
    Cell(Arr<Value>),
    /// A scalar struct.
    Struct(Box<StructV>),
    /// A function handle.
    Fn(Arc<FnValue>),
}

impl Default for Value {
    fn default() -> Value {
        Value::empty()
    }
}

impl Value {
    /// A real scalar.
    pub fn num(x: f64) -> Value {
        Value::Num(Arr::scalar(x))
    }

    /// A logical scalar.
    pub fn boolean(b: bool) -> Value {
        Value::Bool(Arr::scalar(b))
    }

    /// A string.
    pub fn text(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// The empty `[]`.
    pub fn empty() -> Value {
        Value::Num(Arr::empty())
    }

    /// A real row vector.
    pub fn row(v: Vec<f64>) -> Value {
        Value::Num(Arr::row(v))
    }

    /// A real column vector.
    pub fn col(v: Vec<f64>) -> Value {
        Value::Num(Arr::col(v))
    }

    /// A real matrix from column-major data.
    pub fn mat(r: usize, c: usize, d: Vec<f64>) -> Value {
        Value::Num(Arr::new(r, c, d))
    }

    /// A complex array, narrowed to real when every imaginary part is zero.
    pub fn cplx(a: Arr<Complex64>) -> Value {
        if a.d.iter().all(|z| z.im == 0.0) {
            Value::Num(a.map(|z| z.re))
        } else {
            Value::Cplx(a)
        }
    }

    /// A complex scalar, narrowed.
    pub fn cscalar(z: Complex64) -> Value {
        Value::cplx(Arr::scalar(z))
    }

    /// A cell row of values.
    pub fn cell_row(v: Vec<Value>) -> Value {
        Value::Cell(Arr::row(v))
    }

    /// The class name, as `class(x)` reports it.
    pub fn class(&self) -> &'static str {
        match self {
            Value::Num(_) | Value::Cplx(_) => "double",
            Value::Bool(_) => "logical",
            Value::Str(_) => "char",
            Value::Cell(_) => "cell",
            Value::Struct(_) => "struct",
            Value::Fn(_) => "function_handle",
        }
    }

    /// A phrase for messages.
    pub fn describe(&self) -> String {
        let (r, c) = self.dims();
        format!("a {r}x{c} {}", self.class())
    }

    /// Rows and columns.
    pub fn dims(&self) -> (usize, usize) {
        match self {
            Value::Num(a) => (a.r, a.c),
            Value::Cplx(a) => (a.r, a.c),
            Value::Bool(a) => (a.r, a.c),
            Value::Str(s) => {
                let n = s.chars().count();
                if n == 0 {
                    (0, 0)
                } else {
                    (1, n)
                }
            }
            Value::Cell(a) => (a.r, a.c),
            Value::Struct(_) | Value::Fn(_) => (1, 1),
        }
    }

    /// Number of elements.
    pub fn numel(&self) -> usize {
        let (r, c) = self.dims();
        r * c
    }

    /// `true` when there are no elements.
    pub fn is_empty(&self) -> bool {
        self.numel() == 0
    }

    /// `true` for numbers, booleans and text.
    pub fn is_numeric_like(&self) -> bool {
        matches!(
            self,
            Value::Num(_) | Value::Cplx(_) | Value::Bool(_) | Value::Str(_)
        )
    }

    /// The value as real numbers (booleans are 0 and 1, text is character codes).
    pub fn to_real(&self) -> Result<Arr<f64>> {
        match self {
            Value::Num(a) => Ok(a.clone()),
            Value::Bool(a) => Ok(a.map(|&b| f64::from(u8::from(b)))),
            Value::Str(s) => Ok(Arr::row(
                s.chars().map(|c| f64::from(u32::from(c))).collect(),
            ))
            .map(|a: Arr<f64>| if a.d.is_empty() { Arr::empty() } else { a }),
            Value::Cplx(_) => err(
                "Abaco:type",
                "a complex value is not allowed here; use real() or abs() first",
            ),
            other => err(
                "Abaco:type",
                format!("expected numbers, got {}", other.describe()),
            ),
        }
    }

    /// The value as complex numbers.
    pub fn to_complex(&self) -> Result<Arr<Complex64>> {
        match self {
            Value::Cplx(a) => Ok(a.clone()),
            other => Ok(other.to_real()?.map(|&x| Complex64::new(x, 0.0))),
        }
    }

    /// A real scalar, or an error naming `what`.
    pub fn scalar_f64(&self, what: &str) -> Result<f64> {
        match self {
            Value::Num(a) if a.d.len() == 1 => Ok(a.d.first().copied().unwrap_or(0.0)),
            Value::Bool(a) if a.d.len() == 1 => {
                Ok(f64::from(u8::from(a.d.first().copied().unwrap_or(false))))
            }
            other => err(
                "Abaco:type",
                format!("{what} must be a real scalar, got {}", other.describe()),
            ),
        }
    }

    /// A non-negative whole number.
    pub fn scalar_usize(&self, what: &str) -> Result<usize> {
        let x = self.scalar_f64(what)?;
        if x.is_finite() && x >= 0.0 && x.fract() == 0.0 && x <= 1e9 {
            Ok(x as usize)
        } else {
            err(
                "Abaco:type",
                format!("{what} must be a non-negative whole number, got {x}"),
            )
        }
    }

    /// The text of a string value.
    pub fn as_str(&self, what: &str) -> Result<&str> {
        match self {
            Value::Str(s) => Ok(s),
            other => err(
                "Abaco:type",
                format!("{what} must be text, got {}", other.describe()),
            ),
        }
    }

    /// Truth of a condition: non-empty and all elements non-zero.
    pub fn is_true(&self) -> Result<bool> {
        match self {
            Value::Bool(a) => Ok(!a.d.is_empty() && a.d.iter().all(|&b| b)),
            Value::Num(a) => Ok(!a.d.is_empty() && a.d.iter().all(|&x| x != 0.0)),
            Value::Cplx(a) => Ok(!a.d.is_empty() && a.d.iter().all(|z| z.re != 0.0 || z.im != 0.0)),
            Value::Str(s) => Ok(!s.is_empty() && s.chars().all(|c| c != '\0')),
            other => err(
                "Abaco:type",
                format!(
                    "a condition needs numbers or logicals, got {}",
                    other.describe()
                ),
            ),
        }
    }

    /// A struct built from `(name, value)` pairs.
    pub fn structure(fields: Vec<(&str, Value)>) -> Value {
        Value::Struct(Box::new(StructV {
            fields: fields
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect(),
        }))
    }
}

impl From<f64> for Value {
    fn from(x: f64) -> Value {
        Value::num(x)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::boolean(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::text(s)
    }
}
