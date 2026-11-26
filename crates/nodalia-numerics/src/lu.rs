// SPDX-License-Identifier: MIT OR Apache-2.0
//! LU factorisation with partial (row) pivoting, for real and complex square matrices.
//!
//! `P A = L U`, with `L` unit lower triangular. The factorisation fails with
//! [`NumError::Singular`] only when a pivot column is exactly zero; how close to singular a matrix
//! is, is reported by [`Lu::rcond_estimate`], because "nearly singular" depends on the problem and a
//! fixed threshold would reject legitimate badly scaled systems (circuit matrices mix siemens and
//! picosiemens). Every solve checks its result is finite.

use crate::error::{NumError, Result};
use crate::matrix::Matrix;
use crate::scalar::Scalar;

/// The LU factors of a square matrix.
#[derive(Debug, Clone)]
pub struct Lu<T: Scalar = f64> {
    n: usize,
    /// `L` below the diagonal (unit diagonal implied) and `U` on and above it, row-major.
    lu: Vec<T>,
    /// `perm[i]` is the original row now at position `i`.
    perm: Vec<usize>,
    sign: i8,
    norm_1: f64,
}

impl<T: Scalar> Lu<T> {
    /// Factors `a`.
    pub fn factor(a: &Matrix<T>) -> Result<Self> {
        if !a.is_square() {
            return Err(NumError::DimensionMismatch(format!(
                "LU needs a square matrix, got {} x {}",
                a.rows(),
                a.cols()
            )));
        }
        let n = a.rows();
        let mut lu: Vec<T> = a.as_slice().to_vec();
        let mut perm: Vec<usize> = (0..n).collect();
        let mut sign = 1i8;
        let norm_1 = a.norm_1();

        for k in 0..n {
            // Pivot: the largest modulus in column k at or below the diagonal.
            let mut p = k;
            let mut best = 0.0f64;
            for i in k..n {
                let m = lu[i * n + k].modulus();
                if m > best {
                    best = m;
                    p = i;
                }
            }
            if best == 0.0 {
                return Err(NumError::Singular { column: k });
            }
            if p != k {
                for j in 0..n {
                    lu.swap(k * n + j, p * n + j);
                }
                perm.swap(k, p);
                sign = -sign;
            }
            let pivot = lu[k * n + k];
            for i in (k + 1)..n {
                let factor = lu[i * n + k] / pivot;
                lu[i * n + k] = factor;
                if factor != T::zero() {
                    for j in (k + 1)..n {
                        let u = lu[k * n + j];
                        lu[i * n + j] -= factor * u;
                    }
                }
            }
        }
        if lu.iter().any(|v| !v.is_finite_scalar()) {
            return Err(NumError::NonFinite("LU factors"));
        }
        Ok(Self {
            n,
            lu,
            perm,
            sign,
            norm_1,
        })
    }

    /// Order of the factored matrix.
    pub fn order(&self) -> usize {
        self.n
    }

    /// Solves `A x = b`.
    pub fn solve(&self, b: &[T]) -> Result<Vec<T>> {
        let n = self.n;
        if b.len() != n {
            return Err(NumError::DimensionMismatch(format!(
                "the right-hand side has {} values, the matrix has order {n}",
                b.len()
            )));
        }
        let mut x: Vec<T> = self
            .perm
            .iter()
            .filter_map(|&p| b.get(p).copied())
            .collect();
        // L y = P b
        for i in 0..n {
            let mut s = x[i];
            for j in 0..i {
                s -= self.lu[i * n + j] * x[j];
            }
            x[i] = s;
        }
        // U x = y
        for i in (0..n).rev() {
            let mut s = x[i];
            for j in (i + 1)..n {
                s -= self.lu[i * n + j] * x[j];
            }
            x[i] = s / self.lu[i * n + i];
        }
        if x.iter().any(|v| !v.is_finite_scalar()) {
            return Err(NumError::NonFinite("solution"));
        }
        Ok(x)
    }

    /// Solves `A^H x = b` with the same factors (the transpose, for real matrices).
    pub fn solve_adjoint(&self, b: &[T]) -> Result<Vec<T>> {
        let n = self.n;
        if b.len() != n {
            return Err(NumError::DimensionMismatch(format!(
                "the right-hand side has {} values, the matrix has order {n}",
                b.len()
            )));
        }
        // A = P^T L U  =>  A^H = U^H L^H P. Solve U^H y = b, L^H z = y, x = P^T z.
        let mut y = b.to_vec();
        for i in 0..n {
            let mut s = y[i];
            for j in 0..i {
                s -= self.lu[j * n + i].conj() * y[j];
            }
            y[i] = s / self.lu[i * n + i].conj();
        }
        for i in (0..n).rev() {
            let mut s = y[i];
            for j in (i + 1)..n {
                s -= self.lu[j * n + i].conj() * y[j];
            }
            y[i] = s;
        }
        let mut x = vec![T::zero(); n];
        for (i, &p) in self.perm.iter().enumerate() {
            if let Some(slot) = x.get_mut(p) {
                *slot = y[i];
            }
        }
        if x.iter().any(|v| !v.is_finite_scalar()) {
            return Err(NumError::NonFinite("adjoint solution"));
        }
        Ok(x)
    }

    /// Solves `A X = B` column by column.
    pub fn solve_matrix(&self, b: &Matrix<T>) -> Result<Matrix<T>> {
        if b.rows() != self.n {
            return Err(NumError::DimensionMismatch(format!(
                "the right-hand side has {} rows, the matrix has order {}",
                b.rows(),
                self.n
            )));
        }
        let mut out = Matrix::zeros(self.n, b.cols())?;
        for c in 0..b.cols() {
            let col = b.column(c).unwrap_or_default();
            let x = self.solve(&col)?;
            for (r, v) in x.into_iter().enumerate() {
                out.set(r, c, v)?;
            }
        }
        Ok(out)
    }

    /// The determinant.
    pub fn det(&self) -> T {
        let mut d = if self.sign < 0 { -T::one() } else { T::one() };
        for i in 0..self.n {
            d *= self.lu[i * self.n + i];
        }
        d
    }

    /// The inverse, `A^-1`.
    pub fn inverse(&self) -> Result<Matrix<T>> {
        self.solve_matrix(&Matrix::identity(self.n)?)
    }

    /// An estimate of the reciprocal condition number in the 1-norm, `1 / (‖A‖₁ ‖A⁻¹‖₁)`
    /// (Hager's method, as refined by Higham: a handful of solves, no inverse formed).
    ///
    /// Near 1 is well conditioned; below about `1e-12` the digits of a double-precision answer are
    /// mostly noise. Returns `0.0` when the estimate itself cannot be formed.
    pub fn rcond_estimate(&self) -> f64 {
        let n = self.n;
        if n == 0 {
            return 1.0;
        }
        if self.norm_1 == 0.0 {
            return 0.0;
        }
        let inv_norm = match self.inverse_norm_1_estimate() {
            Some(v) if v.is_finite() && v > 0.0 => v,
            _ => return 0.0,
        };
        1.0 / (self.norm_1 * inv_norm)
    }

    fn inverse_norm_1_estimate(&self) -> Option<f64> {
        let n = self.n;
        let mut x = vec![T::from_f64(1.0 / n as f64); n];
        let mut estimate = 0.0f64;
        let mut last_j = usize::MAX;
        for _ in 0..5 {
            let y = self.solve(&x).ok()?;
            let new_est: f64 = y.iter().map(|v| v.modulus()).sum();
            if new_est <= estimate {
                break;
            }
            estimate = new_est;
            // xi = sign(y)
            let xi: Vec<T> = y
                .iter()
                .map(|v| {
                    let m = v.modulus();
                    if m == 0.0 {
                        T::one()
                    } else {
                        *v / T::from_f64(m)
                    }
                })
                .collect();
            let z = self.solve_adjoint(&xi).ok()?;
            let (j, zmax) = z
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.modulus()))
                .fold((0, -1.0), |acc, cur| if cur.1 > acc.1 { cur } else { acc });
            let zx: f64 = z
                .iter()
                .zip(&x)
                .map(|(zi, xi)| (zi.conj() * *xi).re())
                .sum();
            if zmax <= zx || j == last_j {
                break;
            }
            last_j = j;
            x = vec![T::zero(); n];
            x[j] = T::one();
        }
        Some(estimate)
    }
}

/// Solves `A x = b` in one call.
pub fn solve<T: Scalar>(a: &Matrix<T>, b: &[T]) -> Result<Vec<T>> {
    Lu::factor(a)?.solve(b)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use num_complex::Complex64;
    use proptest::prelude::*;

    fn residual(a: &Matrix<f64>, x: &[f64], b: &[f64]) -> f64 {
        let ax = a.mul_vec(x).unwrap();
        ax.iter()
            .zip(b)
            .map(|(p, q)| (p - q).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn solves_a_known_system() {
        // x + y + z = 6; 2y + 5z = -4; 2x + 5y - z = 27  ->  (5, 3, -2)
        let a = Matrix::from_rows(&[[1.0, 1.0, 1.0], [0.0, 2.0, 5.0], [2.0, 5.0, -1.0]]).unwrap();
        let x = solve(&a, &[6.0, -4.0, 27.0]).unwrap();
        for (got, want) in x.iter().zip([5.0, 3.0, -2.0]) {
            assert!((got - want).abs() < 1e-13, "{got} vs {want}");
        }
    }

    #[test]
    fn needs_pivoting() {
        let a = Matrix::from_rows(&[[0.0, 1.0], [1.0, 0.0]]).unwrap();
        let x = solve(&a, &[2.0, 3.0]).unwrap();
        assert_eq!(x, vec![3.0, 2.0]);
        assert_eq!(Lu::factor(&a).unwrap().det(), -1.0);
    }

    #[test]
    fn singular_is_an_error_with_its_column() {
        let a = Matrix::from_rows(&[[1.0, 2.0], [2.0, 4.0]]).unwrap();
        assert_eq!(
            Lu::factor(&a).unwrap_err(),
            NumError::Singular { column: 1 }
        );
        let z = Matrix::<f64>::zeros(3, 3).unwrap();
        assert_eq!(
            Lu::factor(&z).unwrap_err(),
            NumError::Singular { column: 0 }
        );
    }

    #[test]
    fn non_square_is_refused() {
        let a = Matrix::<f64>::zeros(2, 3).unwrap();
        assert!(matches!(
            Lu::factor(&a),
            Err(NumError::DimensionMismatch(_))
        ));
    }

    #[test]
    fn determinant_and_inverse() {
        let a = Matrix::from_rows(&[[4.0, 3.0], [6.0, 3.0]]).unwrap();
        let lu = Lu::factor(&a).unwrap();
        assert!((lu.det() - -6.0).abs() < 1e-13);
        let inv = lu.inverse().unwrap();
        let p = a.matmul(&inv).unwrap();
        for (i, v) in p.as_slice().iter().enumerate() {
            let want = if i == 0 || i == 3 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-13);
        }
    }

    #[test]
    fn complex_system() {
        // (1+i) x = 2i  ->  x = 1 + i
        let a = Matrix::from_rows(&[[Complex64::new(1.0, 1.0)]]).unwrap();
        let x = solve(&a, &[Complex64::new(0.0, 2.0)]).unwrap();
        assert!((x[0] - Complex64::new(1.0, 1.0)).norm() < 1e-14);
    }

    #[test]
    fn adjoint_solve_matches_explicit_adjoint() {
        let a = Matrix::from_rows(&[
            [Complex64::new(2.0, 1.0), Complex64::new(0.0, -1.0)],
            [Complex64::new(1.0, 0.5), Complex64::new(3.0, 0.0)],
        ])
        .unwrap();
        let b = [Complex64::new(1.0, 2.0), Complex64::new(-1.0, 0.5)];
        let lu = Lu::factor(&a).unwrap();
        let x = lu.solve_adjoint(&b).unwrap();
        let back = a.adjoint().mul_vec(&x).unwrap();
        for (p, q) in back.iter().zip(&b) {
            assert!((p - q).norm() < 1e-13);
        }
    }

    #[test]
    fn hilbert_matrix_conditioning_is_reported() {
        // The 8x8 Hilbert matrix has a 1-norm condition number of about 3.4e10.
        let n = 8;
        let mut h = Matrix::<f64>::zeros(n, n).unwrap();
        for i in 0..n {
            for j in 0..n {
                h.set(i, j, 1.0 / ((i + j + 1) as f64)).unwrap();
            }
        }
        let rc = Lu::factor(&h).unwrap().rcond_estimate();
        let cond = 1.0 / rc;
        assert!(cond > 1e10 && cond < 1e11, "cond estimate {cond:e}");
    }

    #[test]
    fn well_conditioned_matrix_has_rcond_near_one() {
        let rc = Lu::factor(&Matrix::<f64>::identity(10).unwrap())
            .unwrap()
            .rcond_estimate();
        assert!((rc - 1.0).abs() < 1e-12);
    }

    #[test]
    fn badly_scaled_but_regular_systems_are_not_rejected() {
        // Conductances of 1 S next to 1e-12 S, as in circuit matrices.
        let a = Matrix::from_rows(&[[1.0, 0.0], [0.0, 1e-12]]).unwrap();
        let x = solve(&a, &[1.0, 1e-12]).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-15 && (x[1] - 1.0).abs() < 1e-12);
    }

    proptest! {
        #[test]
        fn random_diagonally_dominant_systems_have_tiny_residuals(
            n in 1usize..9,
            seed in proptest::collection::vec(-1.0f64..1.0, 81 + 9),
        ) {
            let mut a = Matrix::<f64>::zeros(n, n).unwrap();
            for i in 0..n {
                let mut row_sum = 0.0;
                for j in 0..n {
                    let v = seed[i * 9 + j];
                    a.set(i, j, v).unwrap();
                    row_sum += v.abs();
                }
                a.set(i, i, row_sum + 1.0).unwrap();
            }
            let b: Vec<f64> = (0..n).map(|i| seed[81 + i]).collect();
            let x = solve(&a, &b).unwrap();
            prop_assert!(residual(&a, &x, &b) < 1e-12);
        }
    }
}
