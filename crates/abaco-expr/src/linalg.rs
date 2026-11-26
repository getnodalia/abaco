// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dense linear algebra on matrix values, through `nodalia-numerics`.

use crate::error::{ErrorKind, ExprError};
use crate::ops::{err, Ctx, R};
use crate::value::{Mat, Value};
use nodalia_numerics::{Lu, Matrix, NumError};
use num_complex::Complex64;

enum Num {
    Real(Matrix<f64>),
    Complex(Matrix<Complex64>),
}

pub(crate) fn num_error(e: NumError) -> ExprError {
    match e {
        NumError::Singular { column } => err(
            ErrorKind::Numeric,
            format!("the matrix is singular (no pivot in column {})", column + 1),
        ),
        other => err(ErrorKind::Numeric, other.to_string()),
    }
}

fn units_hint() -> ExprError {
    err(
        ErrorKind::Dimension,
        "linear algebra on matrices with units is not supported; divide by the unit first to get plain numbers (for example `A / N`)",
    )
}

fn to_num(v: &Value) -> R<Num> {
    let (rows, cols, data): (usize, usize, Vec<&Value>) = match v {
        Value::Matrix(m) => (m.rows, m.cols, m.data.iter().collect()),
        Value::Vector(x) => (x.len(), 1, x.iter().collect()),
        s if s.is_scalar() => (1, 1, vec![s]),
        other => {
            return Err(err(
                ErrorKind::Type,
                format!("expected a matrix, got {}", other.type_name()),
            ))
        }
    };
    let mut any_complex = false;
    for x in &data {
        match x {
            Value::Real(_) => {}
            Value::Complex(_) => any_complex = true,
            _ => return Err(units_hint()),
        }
    }
    if any_complex {
        let d: Vec<Complex64> = data
            .iter()
            .map(|x| match x {
                Value::Real(r) => Complex64::new(*r, 0.0),
                Value::Complex(z) => *z,
                _ => Complex64::new(0.0, 0.0),
            })
            .collect();
        Ok(Num::Complex(Matrix::new(rows, cols, d).map_err(num_error)?))
    } else {
        let d: Vec<f64> = data
            .iter()
            .map(|x| if let Value::Real(r) = x { *r } else { 0.0 })
            .collect();
        Ok(Num::Real(Matrix::new(rows, cols, d).map_err(num_error)?))
    }
}

fn real_to_value<T>(m: &Matrix<T>, f: impl Fn(&T) -> Value) -> Value
where
    T: nodalia_numerics::Scalar,
{
    Value::Matrix(Mat {
        rows: m.rows(),
        cols: m.cols(),
        data: m.as_slice().iter().map(f).collect(),
    })
}

fn condition<T: nodalia_numerics::Scalar>(ctx: &mut Ctx, lu: &Lu<T>) -> R<()> {
    let rcond = lu.rcond_estimate();
    if rcond < 1e-15 {
        return Err(err(
            ErrorKind::Numeric,
            format!(
                "the matrix is singular to working precision (reciprocal condition number estimate {rcond:.1e})"
            ),
        ));
    }
    if rcond < 1e-10 {
        ctx.warn(format!(
            "the matrix is ill-conditioned (reciprocal condition number estimate {rcond:.1e}); about {} digits of the result may be wrong",
            (-rcond.log10()).floor() as i64
        ));
    }
    Ok(())
}

fn check_square(v: &Value) -> R<()> {
    match v {
        Value::Matrix(m) if m.rows != m.cols => Err(err(
            ErrorKind::Index,
            format!("the matrix must be square (got {}x{})", m.rows, m.cols),
        )),
        Value::Vector(_) => Err(err(ErrorKind::Index, "a vector is not a square matrix")),
        _ => Ok(()),
    }
}

/// Determinant. A matrix with an exactly zero pivot has determinant 0.
pub fn det(ctx: &mut Ctx, v: &Value) -> R<Value> {
    check_square(v)?;
    if let Value::Matrix(m) = v {
        ctx.charge((m.rows as u64).saturating_pow(3))?;
    }
    match to_num(v)? {
        Num::Real(m) => match Lu::factor(&m) {
            Ok(lu) => crate::ops::finite_real(lu.det()),
            Err(NumError::Singular { .. }) => Ok(Value::Real(0.0)),
            Err(e) => Err(num_error(e)),
        },
        Num::Complex(m) => match Lu::factor(&m) {
            Ok(lu) => crate::ops::finite_complex(lu.det()),
            Err(NumError::Singular { .. }) => Ok(Value::Complex(Complex64::new(0.0, 0.0))),
            Err(e) => Err(num_error(e)),
        },
    }
}

/// Matrix inverse; singular or numerically singular matrices are errors.
pub fn inverse(ctx: &mut Ctx, v: &Value) -> R<Value> {
    check_square(v)?;
    if let Value::Matrix(m) = v {
        ctx.charge((m.rows as u64).saturating_pow(3))?;
    }
    match to_num(v)? {
        Num::Real(m) => {
            let lu = Lu::factor(&m).map_err(num_error)?;
            condition(ctx, &lu)?;
            let inv = lu.inverse().map_err(num_error)?;
            Ok(real_to_value(&inv, |x| Value::Real(*x)))
        }
        Num::Complex(m) => {
            let lu = Lu::factor(&m).map_err(num_error)?;
            condition(ctx, &lu)?;
            let inv = lu.inverse().map_err(num_error)?;
            Ok(real_to_value(&inv, |x| Value::Complex(*x)))
        }
    }
}

/// Solves `A x = b` for a vector or matrix right-hand side.
pub fn solve(ctx: &mut Ctx, a: &Value, b: &Value) -> R<Value> {
    check_square(a)?;
    if !matches!(a, Value::Matrix(_)) {
        return Err(err(
            ErrorKind::Type,
            "solve needs a square matrix as its first argument",
        ));
    }
    if let Value::Matrix(m) = a {
        ctx.charge((m.rows as u64).saturating_pow(3))?;
    }
    let rows = match a {
        Value::Matrix(m) => m.rows,
        _ => 0,
    };
    let b_len = match b {
        Value::Vector(x) => x.len(),
        Value::Matrix(m) => m.rows,
        _ => {
            return Err(err(
                ErrorKind::Type,
                "solve needs a vector or matrix as its second argument",
            ))
        }
    };
    if b_len != rows {
        return Err(err(
            ErrorKind::Index,
            format!("the right-hand side has {b_len} rows but the matrix has {rows}"),
        ));
    }
    let a_num = to_num(a)?;
    let b_num = to_num(b)?;
    let b_is_vec = matches!(b, Value::Vector(_));
    // promote both to complex if either is
    match (a_num, b_num) {
        (Num::Real(am), Num::Real(bm)) => {
            let lu = Lu::factor(&am).map_err(num_error)?;
            condition(ctx, &lu)?;
            let x = lu.solve_matrix(&bm).map_err(num_error)?;
            finish_solve(x, b_is_vec, |v| Value::Real(*v))
        }
        (an, bn) => {
            let ac = promote(an)?;
            let bc = promote(bn)?;
            let lu = Lu::factor(&ac).map_err(num_error)?;
            condition(ctx, &lu)?;
            let x = lu.solve_matrix(&bc).map_err(num_error)?;
            finish_solve(x, b_is_vec, |v| Value::Complex(*v))
        }
    }
}

fn promote(n: Num) -> R<Matrix<Complex64>> {
    match n {
        Num::Complex(m) => Ok(m),
        Num::Real(m) => {
            let data: Vec<Complex64> = m
                .as_slice()
                .iter()
                .map(|x| Complex64::new(*x, 0.0))
                .collect();
            Matrix::new(m.rows(), m.cols(), data).map_err(num_error)
        }
    }
}

fn finish_solve<T: nodalia_numerics::Scalar>(
    x: Matrix<T>,
    vector: bool,
    f: impl Fn(&T) -> Value,
) -> R<Value> {
    if vector {
        Ok(Value::Vector(x.as_slice().iter().map(f).collect()))
    } else {
        Ok(real_to_value(&x, f))
    }
}
