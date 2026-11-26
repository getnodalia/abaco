// SPDX-License-Identifier: MIT OR Apache-2.0
//! Householder QR and linear least squares for real matrices with at least as many rows as columns.

use crate::error::{NumError, Result};
use crate::matrix::Matrix;

/// The solution of a least-squares problem.
#[derive(Debug, Clone, PartialEq)]
pub struct LeastSquares {
    /// The minimiser of `‖A x − b‖₂`.
    pub x: Vec<f64>,
    /// `‖A x − b‖₂` at the minimiser.
    pub residual_norm: f64,
}

/// Solves `min ‖A x − b‖₂` by Householder QR.
///
/// Fails with [`NumError::RankDeficient`] when a column of `A` is (numerically) a combination of the
/// earlier ones, because the minimiser is then not unique and returning one would hide that.
pub fn least_squares(a: &Matrix<f64>, b: &[f64]) -> Result<LeastSquares> {
    let (m, n) = (a.rows(), a.cols());
    if m < n {
        return Err(NumError::DimensionMismatch(format!(
            "least squares needs at least as many rows as columns, got {m} x {n}"
        )));
    }
    if b.len() != m {
        return Err(NumError::DimensionMismatch(format!(
            "the right-hand side has {} values, the matrix has {m} rows",
            b.len()
        )));
    }
    if n == 0 {
        return Ok(LeastSquares {
            x: Vec::new(),
            residual_norm: b.iter().map(|v| v * v).sum::<f64>().sqrt(),
        });
    }
    let mut r: Vec<f64> = a.as_slice().to_vec();
    let mut qtb = b.to_vec();
    let mut first_diag = 0.0f64;

    for k in 0..n {
        let mut norm = 0.0f64;
        let scale = (k..m).map(|i| r[i * n + k].abs()).fold(0.0, f64::max);
        if scale == 0.0 {
            return Err(NumError::RankDeficient { column: k });
        }
        for i in k..m {
            norm += (r[i * n + k] / scale).powi(2);
        }
        let norm = scale * norm.sqrt();
        let alpha = if r[k * n + k] > 0.0 { -norm } else { norm };
        // v = x - alpha e1, stored in a scratch vector.
        let mut v: Vec<f64> = (k..m).map(|i| r[i * n + k]).collect();
        v[0] -= alpha;
        let vnorm2: f64 = v.iter().map(|x| x * x).sum();
        if vnorm2 > 0.0 {
            for j in k..n {
                let dot: f64 = v
                    .iter()
                    .enumerate()
                    .map(|(t, vi)| vi * r[(k + t) * n + j])
                    .sum();
                let f = 2.0 * dot / vnorm2;
                for (t, vi) in v.iter().enumerate() {
                    r[(k + t) * n + j] -= f * vi;
                }
            }
            let dot: f64 = v.iter().enumerate().map(|(t, vi)| vi * qtb[k + t]).sum();
            let f = 2.0 * dot / vnorm2;
            for (t, vi) in v.iter().enumerate() {
                qtb[k + t] -= f * vi;
            }
        }
        let diag = r[k * n + k].abs();
        if k == 0 {
            first_diag = diag;
        }
        if diag <= (m.max(n) as f64) * f64::EPSILON * first_diag {
            return Err(NumError::RankDeficient { column: k });
        }
    }

    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = qtb[i];
        for j in (i + 1)..n {
            s -= r[i * n + j] * x[j];
        }
        x[i] = s / r[i * n + i];
    }
    if x.iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("least-squares solution"));
    }
    let residual_norm = qtb[n..].iter().map(|v| v * v).sum::<f64>().sqrt();
    Ok(LeastSquares { x, residual_norm })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn fits_a_line_exactly_when_the_data_is_linear() {
        // y = 2x + 1
        let a = Matrix::from_rows(&[[1.0, 0.0], [1.0, 1.0], [1.0, 2.0], [1.0, 3.0]]).unwrap();
        let r = least_squares(&a, &[1.0, 3.0, 5.0, 7.0]).unwrap();
        assert!((r.x[0] - 1.0).abs() < 1e-12 && (r.x[1] - 2.0).abs() < 1e-12);
        assert!(r.residual_norm < 1e-12);
    }

    #[test]
    fn matches_the_normal_equations_on_noisy_data() {
        // Points (0,1), (1,2), (2,2), (3,5): normal equations give slope 1.2, intercept 0.7.
        let a = Matrix::from_rows(&[[1.0, 0.0], [1.0, 1.0], [1.0, 2.0], [1.0, 3.0]]).unwrap();
        let r = least_squares(&a, &[1.0, 2.0, 2.0, 5.0]).unwrap();
        assert!((r.x[0] - 0.7).abs() < 1e-12, "{:?}", r.x);
        assert!((r.x[1] - 1.2).abs() < 1e-12);
        // residuals: 0.3, 0.1, -1.1, 0.7 -> sqrt(0.09+0.01+1.21+0.49) = sqrt(1.8)
        assert!((r.residual_norm - 1.8f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn dependent_columns_are_reported() {
        let a = Matrix::from_rows(&[[1.0, 2.0], [2.0, 4.0], [3.0, 6.0]]).unwrap();
        assert!(matches!(
            least_squares(&a, &[1.0, 2.0, 3.0]),
            Err(NumError::RankDeficient { .. })
        ));
    }

    #[test]
    fn underdetermined_is_refused() {
        let a = Matrix::from_rows(&[[1.0, 2.0, 3.0]]).unwrap();
        assert!(least_squares(&a, &[1.0]).is_err());
    }
}
