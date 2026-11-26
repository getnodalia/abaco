// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dense row-major matrices over [`Scalar`].

use crate::error::{NumError, Result};
use crate::scalar::Scalar;

/// Largest number of elements a matrix may hold (64 Mi elements; 512 MiB of `f64`).
pub const MAX_ELEMENTS: usize = 1 << 26;

/// A dense matrix stored row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Matrix<T: Scalar = f64> {
    rows: usize,
    cols: usize,
    data: Vec<T>,
}

fn checked_len(rows: usize, cols: usize) -> Result<usize> {
    let len = rows
        .checked_mul(cols)
        .ok_or_else(|| NumError::TooLarge(format!("{rows} x {cols} matrix")))?;
    if len > MAX_ELEMENTS {
        return Err(NumError::TooLarge(format!(
            "{rows} x {cols} matrix has {len} elements (limit {MAX_ELEMENTS})"
        )));
    }
    Ok(len)
}

impl<T: Scalar> Matrix<T> {
    /// A matrix of the given shape from row-major data.
    pub fn new(rows: usize, cols: usize, data: Vec<T>) -> Result<Self> {
        let len = checked_len(rows, cols)?;
        if data.len() != len {
            return Err(NumError::DimensionMismatch(format!(
                "{rows} x {cols} needs {len} values, got {}",
                data.len()
            )));
        }
        if data.iter().any(|v| !v.is_finite_scalar()) {
            return Err(NumError::NonFinite("matrix data"));
        }
        Ok(Self { rows, cols, data })
    }

    /// An all-zero matrix.
    pub fn zeros(rows: usize, cols: usize) -> Result<Self> {
        let len = checked_len(rows, cols)?;
        Ok(Self {
            rows,
            cols,
            data: vec![T::zero(); len],
        })
    }

    /// The identity matrix of order `n`.
    pub fn identity(n: usize) -> Result<Self> {
        let mut m = Self::zeros(n, n)?;
        for i in 0..n {
            m.data[i * n + i] = T::one();
        }
        Ok(m)
    }

    /// A matrix from rows; every row must have the same length.
    pub fn from_rows<R: AsRef<[T]>>(rows: &[R]) -> Result<Self> {
        let cols = rows.first().map_or(0, |r| r.as_ref().len());
        let mut data = Vec::new();
        checked_len(rows.len(), cols)?;
        for (i, r) in rows.iter().enumerate() {
            let r = r.as_ref();
            if r.len() != cols {
                return Err(NumError::DimensionMismatch(format!(
                    "row {i} has {} values, row 0 has {cols}",
                    r.len()
                )));
            }
            data.extend_from_slice(r);
        }
        Self::new(rows.len(), cols, data)
    }

    /// Number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// True when the matrix has as many rows as columns.
    pub fn is_square(&self) -> bool {
        self.rows == self.cols
    }

    /// The element at `(r, c)`, or `None` outside the matrix.
    pub fn get(&self, r: usize, c: usize) -> Option<T> {
        if r < self.rows && c < self.cols {
            self.data.get(r * self.cols + c).copied()
        } else {
            None
        }
    }

    /// Sets the element at `(r, c)`.
    pub fn set(&mut self, r: usize, c: usize, v: T) -> Result<()> {
        let cols = self.cols;
        let rows = self.rows;
        match self.slot(r, c) {
            Some(x) => {
                *x = v;
                Ok(())
            }
            None => Err(out_of_bounds(r, c, rows, cols)),
        }
    }

    /// Adds `v` to the element at `(r, c)`: the operation circuit and finite-element stamps need.
    pub fn add_at(&mut self, r: usize, c: usize, v: T) -> Result<()> {
        let cols = self.cols;
        let rows = self.rows;
        match self.slot(r, c) {
            Some(x) => {
                *x += v;
                Ok(())
            }
            None => Err(out_of_bounds(r, c, rows, cols)),
        }
    }

    fn slot(&mut self, r: usize, c: usize) -> Option<&mut T> {
        if r < self.rows && c < self.cols {
            let cols = self.cols;
            self.data.get_mut(r * cols + c)
        } else {
            None
        }
    }

    /// One row as a slice.
    pub fn row(&self, r: usize) -> Option<&[T]> {
        if r < self.rows {
            self.data.get(r * self.cols..(r + 1) * self.cols)
        } else {
            None
        }
    }

    /// One column copied into a vector.
    pub fn column(&self, c: usize) -> Option<Vec<T>> {
        if c >= self.cols {
            return None;
        }
        Some((0..self.rows).filter_map(|r| self.get(r, c)).collect())
    }

    /// All elements, row by row.
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// The transpose.
    pub fn transpose(&self) -> Self {
        let mut data = Vec::with_capacity(self.data.len());
        for c in 0..self.cols {
            for r in 0..self.rows {
                if let Some(v) = self.get(r, c) {
                    data.push(v);
                }
            }
        }
        Self {
            rows: self.cols,
            cols: self.rows,
            data,
        }
    }

    /// The conjugate transpose (the transpose, for real matrices).
    pub fn adjoint(&self) -> Self {
        let mut t = self.transpose();
        for v in &mut t.data {
            *v = v.conj();
        }
        t
    }

    /// Element-wise sum.
    pub fn add(&self, other: &Self) -> Result<Self> {
        self.zip(other, |a, b| a + b, "add")
    }

    /// Element-wise difference.
    pub fn sub(&self, other: &Self) -> Result<Self> {
        self.zip(other, |a, b| a - b, "subtract")
    }

    fn zip(&self, other: &Self, f: impl Fn(T, T) -> T, what: &str) -> Result<Self> {
        if self.rows != other.rows || self.cols != other.cols {
            return Err(NumError::DimensionMismatch(format!(
                "cannot {what} {} x {} and {} x {}",
                self.rows, self.cols, other.rows, other.cols
            )));
        }
        let data: Vec<T> = self
            .data
            .iter()
            .zip(&other.data)
            .map(|(&a, &b)| f(a, b))
            .collect();
        Self::checked(self.rows, self.cols, data)
    }

    fn checked(rows: usize, cols: usize, data: Vec<T>) -> Result<Self> {
        if data.iter().any(|v| !v.is_finite_scalar()) {
            return Err(NumError::NonFinite("matrix result"));
        }
        Ok(Self { rows, cols, data })
    }

    /// Every element multiplied by `k`.
    pub fn scale(&self, k: T) -> Result<Self> {
        let data: Vec<T> = self.data.iter().map(|&a| a * k).collect();
        Self::checked(self.rows, self.cols, data)
    }

    /// The matrix product `self * other`.
    pub fn matmul(&self, other: &Self) -> Result<Self> {
        if self.cols != other.rows {
            return Err(NumError::DimensionMismatch(format!(
                "cannot multiply {} x {} by {} x {}",
                self.rows, self.cols, other.rows, other.cols
            )));
        }
        let mut out = Self::zeros(self.rows, other.cols)?;
        for i in 0..self.rows {
            for k in 0..self.cols {
                let a = self.data[i * self.cols + k];
                if a == T::zero() {
                    continue;
                }
                for j in 0..other.cols {
                    out.data[i * other.cols + j] += a * other.data[k * other.cols + j];
                }
            }
        }
        Self::checked(out.rows, out.cols, out.data)
    }

    /// The product `self * x` with a vector.
    pub fn mul_vec(&self, x: &[T]) -> Result<Vec<T>> {
        if x.len() != self.cols {
            return Err(NumError::DimensionMismatch(format!(
                "a {} x {} matrix cannot multiply a vector of {}",
                self.rows,
                self.cols,
                x.len()
            )));
        }
        let mut y = Vec::with_capacity(self.rows);
        for i in 0..self.rows {
            let mut acc = T::zero();
            for j in 0..self.cols {
                acc += self.data[i * self.cols + j] * x[j];
            }
            if !acc.is_finite_scalar() {
                return Err(NumError::NonFinite("matrix-vector product"));
            }
            y.push(acc);
        }
        Ok(y)
    }

    /// The 1-norm: largest absolute column sum.
    pub fn norm_1(&self) -> f64 {
        (0..self.cols)
            .map(|c| {
                (0..self.rows)
                    .filter_map(|r| self.get(r, c))
                    .map(|v| v.modulus())
                    .sum::<f64>()
            })
            .fold(0.0, f64::max)
    }

    /// The infinity norm: largest absolute row sum.
    pub fn norm_inf(&self) -> f64 {
        (0..self.rows)
            .map(|r| {
                self.row(r)
                    .map_or(0.0, |row| row.iter().map(|v| v.modulus()).sum::<f64>())
            })
            .fold(0.0, f64::max)
    }

    /// The Frobenius norm.
    pub fn norm_frobenius(&self) -> f64 {
        let scale = self.data.iter().map(|v| v.modulus()).fold(0.0, f64::max);
        if scale == 0.0 {
            return 0.0;
        }
        let s: f64 = self
            .data
            .iter()
            .map(|v| (v.modulus() / scale).powi(2))
            .sum();
        scale * s.sqrt()
    }

    /// The trace of a square matrix.
    pub fn trace(&self) -> Result<T> {
        if !self.is_square() {
            return Err(NumError::DimensionMismatch(format!(
                "the trace needs a square matrix, got {} x {}",
                self.rows, self.cols
            )));
        }
        let mut t = T::zero();
        for i in 0..self.rows {
            t += self.data[i * self.cols + i];
        }
        Ok(t)
    }
}

fn out_of_bounds(r: usize, c: usize, rows: usize, cols: usize) -> NumError {
    NumError::DimensionMismatch(format!(
        "element ({r}, {c}) is outside a {rows} x {cols} matrix"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use num_complex::Complex64;

    #[test]
    fn builds_and_reads() {
        let m = Matrix::from_rows(&[[1.0, 2.0], [3.0, 4.0]]).unwrap();
        assert_eq!(m.get(1, 0), Some(3.0));
        assert_eq!(m.get(2, 0), None);
        assert_eq!(m.row(0).unwrap(), &[1.0, 2.0]);
        assert_eq!(m.column(1).unwrap(), vec![2.0, 4.0]);
    }

    #[test]
    fn ragged_rows_are_rejected() {
        let rows: Vec<Vec<f64>> = vec![vec![1.0, 2.0], vec![3.0]];
        assert!(matches!(
            Matrix::from_rows(&rows),
            Err(NumError::DimensionMismatch(_))
        ));
    }

    #[test]
    fn non_finite_is_rejected() {
        assert!(matches!(
            Matrix::new(1, 1, vec![f64::NAN]),
            Err(NumError::NonFinite(_))
        ));
        assert!(matches!(
            Matrix::new(1, 2, vec![1.0, f64::INFINITY]),
            Err(NumError::NonFinite(_))
        ));
    }

    #[test]
    fn huge_shapes_are_refused_without_allocating() {
        assert!(matches!(
            Matrix::<f64>::zeros(usize::MAX, 2),
            Err(NumError::TooLarge(_))
        ));
        assert!(matches!(
            Matrix::<f64>::zeros(1 << 20, 1 << 20),
            Err(NumError::TooLarge(_))
        ));
    }

    #[test]
    fn stamping_accumulates_and_checks_bounds() {
        let mut m = Matrix::<f64>::zeros(2, 2).unwrap();
        m.add_at(0, 0, 1.5).unwrap();
        m.add_at(0, 0, 2.5).unwrap();
        assert_eq!(m.get(0, 0), Some(4.0));
        assert!(m.add_at(2, 0, 1.0).is_err());
        assert!(m.set(0, 2, 1.0).is_err());
    }

    #[test]
    fn product_and_transpose() {
        let a = Matrix::from_rows(&[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]).unwrap();
        let b = a.transpose();
        let p = a.matmul(&b).unwrap();
        assert_eq!(p.as_slice(), &[14.0, 32.0, 32.0, 77.0]);
        assert!(a.matmul(&a).is_err());
        assert_eq!(a.mul_vec(&[1.0, 0.0, -1.0]).unwrap(), vec![-2.0, -2.0]);
    }

    #[test]
    fn complex_adjoint_conjugates() {
        let m = Matrix::from_rows(&[[Complex64::new(1.0, 2.0), Complex64::new(0.0, 1.0)]]).unwrap();
        let h = m.adjoint();
        assert_eq!(h.get(0, 0), Some(Complex64::new(1.0, -2.0)));
        assert_eq!(h.get(1, 0), Some(Complex64::new(0.0, -1.0)));
    }

    #[test]
    fn norms() {
        let m = Matrix::from_rows(&[[1.0, -2.0], [3.0, 4.0]]).unwrap();
        assert_eq!(m.norm_1(), 6.0);
        assert_eq!(m.norm_inf(), 7.0);
        assert!((m.norm_frobenius() - 30f64.sqrt()).abs() < 1e-14);
        assert_eq!(m.trace().unwrap(), 5.0);
    }

    #[test]
    fn overflow_in_a_product_is_reported() {
        let a = Matrix::new(1, 1, vec![1e200]).unwrap();
        assert!(matches!(a.matmul(&a), Err(NumError::NonFinite(_))));
    }
}
