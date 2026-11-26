// SPDX-License-Identifier: MIT OR Apache-2.0
//! Operators on values: scalars, elementwise broadcasting, matrix products.

use crate::ast::BinOp;
use crate::error::{ErrorKind, ExprError};
use crate::limits::Limits;
use crate::value::{Mat, Value};
use nodalia_units::{Quantity, UnitError};
use num_complex::Complex64;
use std::cmp::Ordering;

pub(crate) type R<T> = Result<T, ExprError>;

/// Per-evaluation bookkeeping shared by the operators and built-ins.
pub struct Ctx {
    pub limits: Limits,
    pub steps: u64,
    pub warnings: Vec<String>,
}

impl Ctx {
    pub fn new(limits: Limits) -> Ctx {
        Ctx {
            limits,
            steps: 0,
            warnings: Vec::new(),
        }
    }

    /// Charges `n` units of work against the step budget.
    pub fn charge(&mut self, n: u64) -> R<()> {
        self.steps = self.steps.saturating_add(n);
        if self.steps > self.limits.max_steps {
            return Err(ExprError::new(
                ErrorKind::Limit,
                format!(
                    "the evaluation exceeded the limit of {} steps; the expression does too much work",
                    self.limits.max_steps
                ),
            ));
        }
        Ok(())
    }

    /// Checks that a container of `n` elements is allowed.
    pub fn check_elements(&self, n: usize) -> R<()> {
        if n > self.limits.max_elements {
            return Err(ExprError::new(
                ErrorKind::Limit,
                format!(
                    "a vector or matrix of {n} elements exceeds the limit of {}",
                    self.limits.max_elements
                ),
            ));
        }
        Ok(())
    }

    pub fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }
}

pub(crate) fn err(kind: ErrorKind, msg: impl Into<String>) -> ExprError {
    ExprError::new(kind, msg)
}

pub(crate) fn finite_real(x: f64) -> R<Value> {
    if x.is_finite() {
        Ok(Value::Real(x))
    } else {
        Err(err(
            ErrorKind::Numeric,
            "the result is not a finite number (overflow)",
        ))
    }
}

pub(crate) fn finite_complex(z: Complex64) -> R<Value> {
    if z.re.is_finite() && z.im.is_finite() {
        Ok(Value::Complex(z))
    } else {
        Err(err(
            ErrorKind::Numeric,
            "the result is not a finite number (overflow)",
        ))
    }
}

pub(crate) fn from_q(r: Result<Quantity, UnitError>) -> R<Value> {
    r.map(Value::quantity).map_err(ExprError::from)
}

fn complex_of(v: &Value) -> Option<Complex64> {
    match v {
        Value::Real(x) => Some(Complex64::new(*x, 0.0)),
        Value::Complex(z) => Some(*z),
        _ => None,
    }
}

fn verb(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "subtract",
        BinOp::Mul | BinOp::ElemMul => "multiply",
        BinOp::Div | BinOp::ElemDiv => "divide",
        BinOp::Pow | BinOp::ElemPow => "raise",
        _ => "compare",
    }
}

fn type_error(op: BinOp, a: &Value, b: &Value) -> ExprError {
    err(
        ErrorKind::Type,
        format!(
            "cannot {} {} and {}",
            verb(op),
            a.type_name(),
            b.type_name()
        ),
    )
}

fn real_pow(x: f64, y: f64) -> R<Value> {
    if x == 0.0 && y < 0.0 {
        return Err(err(
            ErrorKind::Domain,
            "division by zero (zero raised to a negative power)",
        ));
    }
    if x < 0.0 && y.fract() != 0.0 {
        return Err(err(
            ErrorKind::Domain,
            "domain error: a negative base with a non-integer exponent is not real (use a complex base)",
        ));
    }
    let r = if y.fract() == 0.0 && y.abs() <= f64::from(i32::MAX) {
        x.powi(y as i32)
    } else {
        x.powf(y)
    };
    finite_real(r)
}

fn complex_pow(z: Complex64, w: Complex64) -> R<Value> {
    if z == Complex64::new(0.0, 0.0) {
        if w == Complex64::new(0.0, 0.0) {
            return Ok(Value::Complex(Complex64::new(1.0, 0.0)));
        }
        if w.re > 0.0 {
            return Ok(Value::Complex(Complex64::new(0.0, 0.0)));
        }
        return Err(err(
            ErrorKind::Domain,
            "division by zero (zero raised to a non-positive power)",
        ));
    }
    finite_complex(z.powc(w))
}

/// The exponent of `^` as a plain `f64`: a number or a dimensionless quantity.
fn exponent_f64(v: &Value) -> R<f64> {
    match v {
        Value::Real(x) => Ok(*x),
        Value::Quantity(q) if q.dimension().is_dimensionless() => Ok(q.si_value()),
        Value::Quantity(q) => Err(err(
            ErrorKind::Dimension,
            format!(
                "the exponent must be dimensionless, got {} ({})",
                q.display_unit()
                    .map_or_else(|| q.dimension().to_string(), ToString::to_string),
                q.dimension().describe()
            ),
        )),
        other => Err(err(
            ErrorKind::Type,
            format!("the exponent must be a number, not {}", other.type_name()),
        )),
    }
}

/// `a op b` on two scalars (`Add`, `Sub`, `Mul`, `Div`, `Pow` and their elementwise forms).
pub fn scalar_op(op: BinOp, a: &Value, b: &Value) -> R<Value> {
    let op = match op {
        BinOp::ElemMul => BinOp::Mul,
        BinOp::ElemDiv => BinOp::Div,
        BinOp::ElemPow => BinOp::Pow,
        o => o,
    };
    if op == BinOp::Pow {
        match (a, b) {
            (Value::Quantity(q), _) => return from_q(q.pow_f64(exponent_f64(b)?)),
            (Value::Real(x), Value::Quantity(_)) => return real_pow(*x, exponent_f64(b)?),
            _ => {}
        }
    }
    match (a, b) {
        (Value::Real(x), Value::Real(y)) => match op {
            BinOp::Add => finite_real(x + y),
            BinOp::Sub => finite_real(x - y),
            BinOp::Mul => finite_real(x * y),
            BinOp::Div => {
                if *y == 0.0 {
                    Err(err(ErrorKind::Domain, "division by zero"))
                } else {
                    finite_real(x / y)
                }
            }
            BinOp::Pow => real_pow(*x, *y),
            _ => Err(type_error(op, a, b)),
        },
        (Value::Quantity(_), Value::Complex(_)) | (Value::Complex(_), Value::Quantity(_)) => {
            Err(err(
                ErrorKind::Dimension,
                "complex numbers with units are not supported",
            ))
        }
        (Value::Quantity(_), _) | (_, Value::Quantity(_)) => {
            let (Some(qa), Some(qb)) = (a.as_quantity(), b.as_quantity()) else {
                return Err(type_error(op, a, b));
            };
            match op {
                BinOp::Add => from_q(qa.add(&qb)),
                BinOp::Sub => from_q(qa.sub(&qb)),
                BinOp::Mul => from_q(qa.mul(&qb)),
                BinOp::Div => from_q(qa.div(&qb)),
                _ => Err(type_error(op, a, b)),
            }
        }
        _ => {
            let (Some(z), Some(w)) = (complex_of(a), complex_of(b)) else {
                return Err(type_error(op, a, b));
            };
            match op {
                BinOp::Add => finite_complex(z + w),
                BinOp::Sub => finite_complex(z - w),
                BinOp::Mul => finite_complex(z * w),
                BinOp::Div => {
                    if w == Complex64::new(0.0, 0.0) {
                        Err(err(ErrorKind::Domain, "division by zero"))
                    } else {
                        finite_complex(z / w)
                    }
                }
                BinOp::Pow => complex_pow(z, w),
                _ => Err(type_error(op, a, b)),
            }
        }
    }
}

/// Negates a scalar.
pub fn scalar_neg(v: &Value) -> R<Value> {
    match v {
        Value::Real(x) => Ok(Value::Real(-x)),
        Value::Complex(z) => Ok(Value::Complex(-z)),
        Value::Quantity(q) => from_q(q.neg()),
        other => Err(err(
            ErrorKind::Type,
            format!("cannot negate {}", other.type_name()),
        )),
    }
}

/// Orders two scalars (real numbers and quantities of one dimension).
pub fn compare(a: &Value, b: &Value) -> R<Ordering> {
    if let (Value::Real(x), Value::Real(y)) = (a, b) {
        return x
            .partial_cmp(y)
            .ok_or_else(|| err(ErrorKind::Domain, "cannot compare NaN"));
    }
    match (a.as_quantity(), b.as_quantity()) {
        (Some(x), Some(y)) => x.compare(&y).map_err(ExprError::from),
        _ => {
            if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
                Err(err(ErrorKind::Type, "complex numbers cannot be ordered"))
            } else {
                Err(err(
                    ErrorKind::Type,
                    format!("cannot compare {} and {}", a.type_name(), b.type_name()),
                ))
            }
        }
    }
}

fn equal(a: &Value, b: &Value) -> R<bool> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(x == y),
        (Value::Str(x), Value::Str(y)) => Ok(x == y),
        (Value::Vector(x), Value::Vector(y)) => {
            if x.len() != y.len() {
                return Err(err(
                    ErrorKind::Index,
                    format!(
                        "cannot compare vectors of length {} and {}",
                        x.len(),
                        y.len()
                    ),
                ));
            }
            for (p, q) in x.iter().zip(y) {
                if !equal(p, q)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Matrix(x), Value::Matrix(y)) => {
            if x.rows != y.rows || x.cols != y.cols {
                return Err(err(
                    ErrorKind::Index,
                    format!(
                        "cannot compare matrices of shape {}x{} and {}x{}",
                        x.rows, x.cols, y.rows, y.cols
                    ),
                ));
            }
            for (p, q) in x.data.iter().zip(&y.data) {
                if !equal(p, q)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => {
            if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
                match (complex_of(a), complex_of(b)) {
                    (Some(z), Some(w)) => Ok(z == w),
                    _ => Err(err(
                        ErrorKind::Type,
                        format!("cannot compare {} and {}", a.type_name(), b.type_name()),
                    )),
                }
            } else {
                Ok(compare(a, b)? == Ordering::Equal)
            }
        }
    }
}

enum Shape {
    Scalar,
    Vec(usize),
    Mat(usize, usize),
}

fn shape(v: &Value) -> Shape {
    match v {
        Value::Vector(x) => Shape::Vec(x.len()),
        Value::Matrix(m) => Shape::Mat(m.rows, m.cols),
        _ => Shape::Scalar,
    }
}

fn elems(v: &Value) -> &[Value] {
    match v {
        Value::Vector(x) => x,
        Value::Matrix(m) => &m.data,
        other => std::slice::from_ref(other),
    }
}

fn rebuild(like: &Value, data: Vec<Value>) -> Value {
    match like {
        Value::Matrix(m) => Value::Matrix(Mat {
            rows: m.rows,
            cols: m.cols,
            data,
        }),
        _ => Value::Vector(data),
    }
}

/// Applies `f` to every scalar of `v` (or to `v` itself when it is a scalar).
pub fn map_scalars(ctx: &mut Ctx, v: &Value, f: &mut dyn FnMut(&Value) -> R<Value>) -> R<Value> {
    match v {
        Value::Vector(_) | Value::Matrix(_) => {
            let src = elems(v);
            ctx.charge(src.len() as u64)?;
            let mut out = Vec::with_capacity(src.len());
            for x in src {
                out.push(f(x)?);
            }
            Ok(rebuild(v, out))
        }
        other => f(other),
    }
}

fn mismatch_shapes(a: &Shape, b: &Shape, op: BinOp) -> ExprError {
    let d = |s: &Shape| match s {
        Shape::Scalar => "a scalar".to_string(),
        Shape::Vec(n) => format!("a vector of length {n}"),
        Shape::Mat(r, c) => format!("a {r}x{c} matrix"),
    };
    err(
        ErrorKind::Index,
        format!("cannot {} {} and {}", verb(op), d(a), d(b)),
    )
}

/// Elementwise `op` with scalar broadcasting.
fn elementwise(ctx: &mut Ctx, op: BinOp, a: &Value, b: &Value) -> R<Value> {
    match (shape(a), shape(b)) {
        (Shape::Scalar, Shape::Scalar) => scalar_op(op, a, b),
        (Shape::Scalar, _) => {
            let src = elems(b);
            ctx.charge(src.len() as u64)?;
            let mut out = Vec::with_capacity(src.len());
            for y in src {
                out.push(scalar_op(op, a, y)?);
            }
            Ok(rebuild(b, out))
        }
        (_, Shape::Scalar) => {
            let src = elems(a);
            ctx.charge(src.len() as u64)?;
            let mut out = Vec::with_capacity(src.len());
            for x in src {
                out.push(scalar_op(op, x, b)?);
            }
            Ok(rebuild(a, out))
        }
        (sa, sb) => {
            let same = match (&sa, &sb) {
                (Shape::Vec(x), Shape::Vec(y)) => x == y,
                (Shape::Mat(r1, c1), Shape::Mat(r2, c2)) => r1 == r2 && c1 == c2,
                _ => false,
            };
            if !same {
                return Err(mismatch_shapes(&sa, &sb, op));
            }
            let (xa, xb) = (elems(a), elems(b));
            ctx.charge(xa.len() as u64)?;
            let mut out = Vec::with_capacity(xa.len());
            for (x, y) in xa.iter().zip(xb) {
                out.push(scalar_op(op, x, y)?);
            }
            Ok(rebuild(a, out))
        }
    }
}

fn as_matrix(v: &Value) -> Option<Mat> {
    match v {
        Value::Matrix(m) => Some(m.clone()),
        Value::Vector(x) => Some(Mat {
            rows: x.len(),
            cols: 1,
            data: x.clone(),
        }),
        _ => None,
    }
}

/// Matrix product `a * b`.
pub fn matmul(ctx: &mut Ctx, a: &Mat, b: &Mat) -> R<Mat> {
    if a.cols != b.rows {
        return Err(err(
            ErrorKind::Index,
            format!(
                "cannot multiply a {}x{} matrix by a {}x{} matrix (inner sizes {} and {} differ)",
                a.rows, a.cols, b.rows, b.cols, a.cols, b.rows
            ),
        ));
    }
    let total = a
        .rows
        .checked_mul(b.cols)
        .ok_or_else(|| err(ErrorKind::Limit, "the matrix product is too large"))?;
    ctx.check_elements(total)?;
    ctx.charge((total as u64).saturating_mul(a.cols.max(1) as u64))?;
    let mut data = Vec::with_capacity(total);
    for i in 0..a.rows {
        for j in 0..b.cols {
            let mut acc: Option<Value> = None;
            for k in 0..a.cols {
                let (Some(x), Some(y)) = (a.get(i, k), b.get(k, j)) else {
                    return Err(err(ErrorKind::Index, "matrix index out of range"));
                };
                let t = scalar_op(BinOp::Mul, x, y)?;
                acc = Some(match acc {
                    None => t,
                    Some(s) => scalar_op(BinOp::Add, &s, &t)?,
                });
            }
            data.push(acc.unwrap_or(Value::Real(0.0)));
        }
    }
    Ok(Mat {
        rows: a.rows,
        cols: b.cols,
        data,
    })
}

fn identity(n: usize) -> Mat {
    let mut data = vec![Value::Real(0.0); n * n];
    for i in 0..n {
        if let Some(x) = data.get_mut(i * n + i) {
            *x = Value::Real(1.0);
        }
    }
    Mat {
        rows: n,
        cols: n,
        data,
    }
}

fn matrix_power(ctx: &mut Ctx, m: &Mat, n: i64) -> R<Value> {
    if m.rows != m.cols {
        return Err(err(
            ErrorKind::Index,
            format!(
                "only square matrices can be raised to a power (got {}x{})",
                m.rows, m.cols
            ),
        ));
    }
    if n.unsigned_abs() > 1_000_000 {
        return Err(err(ErrorKind::Limit, "matrix exponent is too large"));
    }
    let mut base = if n < 0 {
        match crate::linalg::inverse(ctx, &Value::Matrix(m.clone()))? {
            Value::Matrix(i) => i,
            _ => return Err(err(ErrorKind::Numeric, "inverse failed")),
        }
    } else {
        m.clone()
    };
    let mut e = n.unsigned_abs();
    let mut acc = identity(m.rows);
    while e > 0 {
        if e & 1 == 1 {
            acc = matmul(ctx, &acc, &base)?;
        }
        e >>= 1;
        if e > 0 {
            base = matmul(ctx, &base, &base)?;
        }
    }
    Ok(Value::Matrix(acc))
}

/// Evaluates a non-short-circuit binary operator on two values.
pub fn binary(ctx: &mut Ctx, op: BinOp, a: &Value, b: &Value) -> R<Value> {
    match op {
        BinOp::Eq => Ok(Value::Bool(equal(a, b)?)),
        BinOp::Ne => Ok(Value::Bool(!equal(a, b)?)),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            if !(a.is_scalar() && b.is_scalar()) {
                return Err(err(
                    ErrorKind::Type,
                    format!(
                        "cannot order {} and {}; comparison needs two numbers or quantities",
                        a.type_name(),
                        b.type_name()
                    ),
                ));
            }
            let o = compare(a, b)?;
            Ok(Value::Bool(match op {
                BinOp::Lt => o == Ordering::Less,
                BinOp::Le => o != Ordering::Greater,
                BinOp::Gt => o == Ordering::Greater,
                _ => o != Ordering::Less,
            }))
        }
        BinOp::ElemMul | BinOp::ElemDiv | BinOp::ElemPow | BinOp::Add | BinOp::Sub => {
            elementwise(ctx, op, a, b)
        }
        BinOp::Mul => match (shape(a), shape(b)) {
            (Shape::Scalar, _) | (_, Shape::Scalar) => elementwise(ctx, op, a, b),
            (Shape::Mat(..), Shape::Mat(..) | Shape::Vec(_)) => {
                let (Some(x), Some(y)) = (as_matrix(a), as_matrix(b)) else {
                    return Err(type_error(op, a, b));
                };
                let p = matmul(ctx, &x, &y)?;
                if matches!(b, Value::Vector(_)) {
                    Ok(Value::Vector(p.data))
                } else {
                    Ok(Value::Matrix(p))
                }
            }
            (Shape::Vec(_), Shape::Vec(_)) => Err(err(
                ErrorKind::Type,
                "`*` of two vectors is ambiguous: use dot(a, b) for the inner product or `.*` for elementwise",
            )),
            (Shape::Vec(_), Shape::Mat(..)) => Err(err(
                ErrorKind::Type,
                "a vector is a column: write transpose(v) * A for a row times a matrix",
            )),
        },
        BinOp::Div => match (shape(a), shape(b)) {
            (_, Shape::Scalar) => elementwise(ctx, op, a, b),
            (Shape::Scalar, _) => Err(err(
                ErrorKind::Type,
                "cannot divide a number by a vector or matrix: use `./` for elementwise, or solve(A, b)",
            )),
            _ => Err(err(
                ErrorKind::Type,
                "cannot divide two vectors or matrices: use `./` for elementwise, or solve(A, b)",
            )),
        },
        BinOp::Pow => match (shape(a), shape(b)) {
            (Shape::Scalar, Shape::Scalar) => scalar_op(op, a, b),
            (Shape::Mat(..), Shape::Scalar) => {
                let Value::Matrix(m) = a else {
                    return Err(type_error(op, a, b));
                };
                let e = match b {
                    Value::Real(x) if x.fract() == 0.0 && x.abs() < 1e9 => *x as i64,
                    _ => {
                        return Err(err(
                            ErrorKind::Domain,
                            "a matrix can only be raised to an integer power",
                        ))
                    }
                };
                matrix_power(ctx, m, e)
            }
            _ => Err(err(
                ErrorKind::Type,
                "`^` needs numbers (or a square matrix and an integer); use `.^` for elementwise powers",
            )),
        },
        BinOp::And | BinOp::Or => Err(err(ErrorKind::Type, "internal: logic operators short-circuit")),
    }
}

/// Unary minus on any value.
pub fn negate(ctx: &mut Ctx, v: &Value) -> R<Value> {
    map_scalars(ctx, v, &mut |x| scalar_neg(x))
}
