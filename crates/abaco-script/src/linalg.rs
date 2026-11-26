// SPDX-License-Identifier: MIT OR Apache-2.0
//! Glue between values and the dense solvers of `nodalia-numerics`.

use crate::error::{err, Error, Result};
use crate::value::{Arr, Value};
use nodalia_numerics::{matfun, svd, Lu, Matrix, NumError};
use num_complex::Complex64;

/// Maps a numerical error to a script error.
pub fn num_err(e: NumError) -> Error {
    let id = match &e {
        NumError::Singular { .. } => "Abaco:singular",
        NumError::NoConvergence { .. } => "Abaco:convergence",
        NumError::DimensionMismatch(_) => "Abaco:size",
        _ => "Abaco:numeric",
    };
    Error::new(id, e.to_string())
}

/// A real column-major array as a row-major matrix.
pub fn to_matrix(a: &Arr<f64>) -> Result<Matrix<f64>> {
    let mut d = Vec::with_capacity(a.d.len());
    for i in 0..a.r {
        for j in 0..a.c {
            d.push(a.d.get(j * a.r + i).copied().unwrap_or(0.0));
        }
    }
    if d.iter().any(|x| !x.is_finite()) {
        return err("Abaco:numeric", "the matrix contains NaN or Inf");
    }
    Matrix::new(a.r, a.c, d).map_err(num_err)
}

/// A complex column-major array as a row-major matrix.
pub fn to_cmatrix(a: &Arr<Complex64>) -> Result<Matrix<Complex64>> {
    let mut d = Vec::with_capacity(a.d.len());
    for i in 0..a.r {
        for j in 0..a.c {
            d.push(
                a.d.get(j * a.r + i)
                    .copied()
                    .unwrap_or(Complex64::new(0.0, 0.0)),
            );
        }
    }
    if d.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
        return err("Abaco:numeric", "the matrix contains NaN or Inf");
    }
    Matrix::new(a.r, a.c, d).map_err(num_err)
}

/// A row-major matrix as a column-major array.
pub fn from_matrix(m: &Matrix<f64>) -> Arr<f64> {
    let (r, c) = (m.rows(), m.cols());
    let mut d = Vec::with_capacity(r * c);
    for j in 0..c {
        for i in 0..r {
            d.push(m.get(i, j).unwrap_or(0.0));
        }
    }
    Arr { r, c, d }
}

/// A complex row-major matrix as a column-major array.
pub fn from_cmatrix(m: &Matrix<Complex64>) -> Arr<Complex64> {
    let (r, c) = (m.rows(), m.cols());
    let mut d = Vec::with_capacity(r * c);
    for j in 0..c {
        for i in 0..r {
            d.push(m.get(i, j).unwrap_or(Complex64::new(0.0, 0.0)));
        }
    }
    Arr { r, c, d }
}

fn is_complex(v: &Value) -> bool {
    matches!(v, Value::Cplx(_))
}

/// `A \ B`: exact for square systems, the minimum-norm least-squares solution otherwise.
pub fn solve(a: &Value, b: &Value) -> Result<Value> {
    let (ar, ac) = a.dims();
    let (br, _) = b.dims();
    if ar != br {
        return err(
            "Abaco:size",
            format!("`\\` needs the same number of rows in both operands, got {ar} and {br}"),
        );
    }
    if is_complex(a) || is_complex(b) {
        if ar != ac {
            return err(
                "Abaco:type",
                "least squares with complex numbers is not supported; use a square system",
            );
        }
        let am = to_cmatrix(&a.to_complex()?)?;
        let bm = to_cmatrix(&b.to_complex()?)?;
        let lu = Lu::factor(&am).map_err(num_err)?;
        let x = lu.solve_matrix(&bm).map_err(num_err)?;
        return Ok(Value::cplx(from_cmatrix(&x)));
    }
    let am = to_matrix(&a.to_real()?)?;
    let bm = to_matrix(&b.to_real()?)?;
    if ar == ac {
        let lu = Lu::factor(&am).map_err(num_err)?;
        let x = lu.solve_matrix(&bm).map_err(num_err)?;
        return Ok(Value::Num(from_matrix(&x)));
    }
    let p = svd::pinv(&am, None).map_err(num_err)?;
    let x = p.matmul(&bm).map_err(num_err)?;
    Ok(Value::Num(from_matrix(&x)))
}

/// The inverse of a square matrix.
pub fn inv(a: &Value) -> Result<Value> {
    let (r, c) = a.dims();
    if r != c {
        return err(
            "Abaco:size",
            format!("`inv` needs a square matrix, got {r}x{c}"),
        );
    }
    if r == 0 {
        return Ok(Value::empty());
    }
    if is_complex(a) {
        let m = to_cmatrix(&a.to_complex()?)?;
        let i = Lu::factor(&m)
            .map_err(num_err)?
            .inverse()
            .map_err(num_err)?;
        return Ok(Value::cplx(from_cmatrix(&i)));
    }
    let m = to_matrix(&a.to_real()?)?;
    let i = Lu::factor(&m)
        .map_err(num_err)?
        .inverse()
        .map_err(num_err)?;
    Ok(Value::Num(from_matrix(&i)))
}

/// The determinant.
pub fn det(a: &Value) -> Result<Value> {
    let (r, c) = a.dims();
    if r != c {
        return err(
            "Abaco:size",
            format!("`det` needs a square matrix, got {r}x{c}"),
        );
    }
    if r == 0 {
        return Ok(Value::num(1.0));
    }
    if is_complex(a) {
        let m = to_cmatrix(&a.to_complex()?)?;
        return match Lu::factor(&m) {
            Ok(lu) => Ok(Value::cscalar(lu.det())),
            Err(NumError::Singular { .. }) => Ok(Value::num(0.0)),
            Err(e) => Err(num_err(e)),
        };
    }
    let m = to_matrix(&a.to_real()?)?;
    match Lu::factor(&m) {
        Ok(lu) => Ok(Value::num(lu.det())),
        Err(NumError::Singular { .. }) => Ok(Value::num(0.0)),
        Err(e) => Err(num_err(e)),
    }
}

/// `A ^ B` for matrices (a square matrix to a whole power, or a scalar to a matrix power).
pub fn mpower(a: &Value, b: &Value) -> Result<Value> {
    let (ar, ac) = a.dims();
    if b.numel() == 1 {
        if ar != ac {
            return err(
                "Abaco:size",
                format!("`^` needs a square matrix, got {ar}x{ac}"),
            );
        }
        let p = b.scalar_f64("the exponent")?;
        if p.fract() != 0.0 || p.abs() > 1e6 {
            return err("Abaco:type", "a matrix can only be raised to a whole-number power; use `.^` for element-wise powers");
        }
        let mut n = p.abs() as u64;
        let mut base = if p < 0.0 { inv(a)? } else { a.clone() };
        let mut acc = identity_like(ar)?;
        while n > 0 {
            if n & 1 == 1 {
                acc = crate::ops::matmul(&acc, &base)?;
            }
            n >>= 1;
            if n > 0 {
                base = crate::ops::matmul(&base, &base)?;
            }
        }
        return Ok(acc);
    }
    if a.numel() == 1 {
        // s ^ A = expm(log(s) * A)
        let (br, bc) = b.dims();
        if br != bc {
            return err("Abaco:size", "`^` needs a square matrix as the exponent");
        }
        let s = a.scalar_f64("the base")?;
        if s <= 0.0 {
            return err(
                "Abaco:type",
                "a non-positive scalar cannot be raised to a matrix power",
            );
        }
        let scaled = crate::ops::binary(crate::ast::BinOp::ElemMul, &Value::num(s.ln()), b)?;
        return expm(&scaled);
    }
    err(
        "Abaco:size",
        "`^` needs a scalar on one side; use `.^` for element-wise powers",
    )
}

/// The identity matrix of order `n`.
pub fn identity_like(n: usize) -> Result<Value> {
    let mut d = vec![0.0; n * n];
    for i in 0..n {
        if let Some(x) = d.get_mut(i * n + i) {
            *x = 1.0;
        }
    }
    Ok(Value::Num(Arr::new(n, n, d)))
}

/// The matrix exponential.
pub fn expm(a: &Value) -> Result<Value> {
    let (r, c) = a.dims();
    if r != c {
        return err(
            "Abaco:size",
            format!("`expm` needs a square matrix, got {r}x{c}"),
        );
    }
    let m = to_matrix(&a.to_real()?)?;
    let e = matfun::expm(&m).map_err(num_err)?;
    Ok(Value::Num(from_matrix(&e)))
}
