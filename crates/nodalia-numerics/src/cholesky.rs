// SPDX-License-Identifier: MIT OR Apache-2.0
//! Cholesky factorisation `A = L Lᵀ` of a real symmetric positive-definite matrix.

use crate::error::{NumError, Result};
use crate::matrix::Matrix;

/// The lower-triangular Cholesky factor of a symmetric positive-definite matrix.
#[derive(Debug, Clone)]
pub struct Cholesky {
    n: usize,
    l: Vec<f64>,
}

impl Cholesky {
    /// Factors `a`. Only the lower triangle is read; the upper one is not checked for symmetry
    /// beyond a relative tolerance of `1e-10`, so an asymmetric input is an error, not a guess.
    pub fn factor(a: &Matrix<f64>) -> Result<Self> {
        if !a.is_square() {
            return Err(NumError::DimensionMismatch(format!(
                "Cholesky needs a square matrix, got {} x {}",
                a.rows(),
                a.cols()
            )));
        }
        let n = a.rows();
        let scale = a.norm_inf().max(f64::MIN_POSITIVE);
        for i in 0..n {
            for j in 0..i {
                let (u, v) = (a.get(i, j).unwrap_or(0.0), a.get(j, i).unwrap_or(0.0));
                if (u - v).abs() > 1e-10 * scale {
                    return Err(NumError::InvalidInput(format!(
                        "the matrix is not symmetric at ({i}, {j})"
                    )));
                }
            }
        }
        let mut l = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..=i {
                let mut s = a.get(i, j).unwrap_or(0.0);
                for k in 0..j {
                    s -= l[i * n + k] * l[j * n + k];
                }
                if i == j {
                    if s <= 0.0 || !s.is_finite() {
                        return Err(NumError::NotPositiveDefinite { row: i });
                    }
                    l[i * n + i] = s.sqrt();
                } else {
                    l[i * n + j] = s / l[j * n + j];
                }
            }
        }
        Ok(Self { n, l })
    }

    /// Solves `A x = b`.
    pub fn solve(&self, b: &[f64]) -> Result<Vec<f64>> {
        let n = self.n;
        if b.len() != n {
            return Err(NumError::DimensionMismatch(format!(
                "the right-hand side has {} values, the matrix has order {n}",
                b.len()
            )));
        }
        let mut y = b.to_vec();
        for i in 0..n {
            let mut s = y[i];
            for k in 0..i {
                s -= self.l[i * n + k] * y[k];
            }
            y[i] = s / self.l[i * n + i];
        }
        for i in (0..n).rev() {
            let mut s = y[i];
            for k in (i + 1)..n {
                s -= self.l[k * n + i] * y[k];
            }
            y[i] = s / self.l[i * n + i];
        }
        if y.iter().any(|v| !v.is_finite()) {
            return Err(NumError::NonFinite("solution"));
        }
        Ok(y)
    }

    /// The lower-triangular factor `L`.
    pub fn l(&self) -> Result<Matrix<f64>> {
        Matrix::new(self.n, self.n, self.l.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn factors_a_known_matrix() {
        // The classic example: A = [[4,12,-16],[12,37,-43],[-16,-43,98]], L = [[2,0,0],[6,1,0],[-8,5,3]].
        let a = Matrix::from_rows(&[
            [4.0, 12.0, -16.0],
            [12.0, 37.0, -43.0],
            [-16.0, -43.0, 98.0],
        ])
        .unwrap();
        let c = Cholesky::factor(&a).unwrap();
        let l = c.l().unwrap();
        let want = [2.0, 0.0, 0.0, 6.0, 1.0, 0.0, -8.0, 5.0, 3.0];
        for (g, w) in l.as_slice().iter().zip(want) {
            assert!((g - w).abs() < 1e-13);
        }
        let x = c.solve(&[1.0, 2.0, 3.0]).unwrap();
        let back = a.mul_vec(&x).unwrap();
        for (g, w) in back.iter().zip([1.0, 2.0, 3.0]) {
            assert!((g - w).abs() < 1e-11);
        }
    }

    #[test]
    fn indefinite_and_asymmetric_are_errors() {
        let a = Matrix::from_rows(&[[1.0, 2.0], [2.0, 1.0]]).unwrap();
        assert_eq!(
            Cholesky::factor(&a).unwrap_err(),
            NumError::NotPositiveDefinite { row: 1 }
        );
        let b = Matrix::from_rows(&[[2.0, 1.0], [0.0, 2.0]]).unwrap();
        assert!(matches!(
            Cholesky::factor(&b),
            Err(NumError::InvalidInput(_))
        ));
    }
}
