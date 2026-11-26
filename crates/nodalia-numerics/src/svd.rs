// SPDX-License-Identifier: MIT OR Apache-2.0
//! Singular value decomposition of real matrices by one-sided Jacobi rotations (Hestenes 1958;
//! Demmel & Veselić 1992 show it is accurate to high relative accuracy). The decomposition is
//! the thin one: for an `m × n` matrix with `k = min(m, n)`, `A = U · diag(s) · Vᵀ` with `U` of
//! size `m × k`, `s` of length `k` in descending order and `V` of size `n × k`.

use crate::error::{NumError, Result};
use crate::matrix::Matrix;

/// Largest dimension accepted.
pub const MAX_ORDER: usize = 2048;

/// The factors of [`svd`].
#[derive(Debug, Clone, PartialEq)]
pub struct Svd {
    /// Left singular vectors, `m × k`.
    pub u: Matrix<f64>,
    /// Singular values, descending.
    pub s: Vec<f64>,
    /// Right singular vectors, `n × k`.
    pub v: Matrix<f64>,
}

/// The thin SVD of `a`.
pub fn svd(a: &Matrix<f64>) -> Result<Svd> {
    let (m, n) = (a.rows(), a.cols());
    if m.max(n) > MAX_ORDER {
        return Err(NumError::TooLarge(format!(
            "the SVD is limited to dimension {MAX_ORDER}, got {m} x {n}"
        )));
    }
    if m == 0 || n == 0 {
        return Err(NumError::InvalidInput(
            "the SVD needs a non-empty matrix".into(),
        ));
    }
    if m < n {
        // A = U S Vᵀ  ⇔  Aᵀ = V S Uᵀ
        let t = svd(&a.transpose())?;
        return Ok(Svd {
            u: t.v,
            s: t.s,
            v: t.u,
        });
    }
    // Work on the columns of a copy of A (column-major `u`), accumulating V.
    let mut u: Vec<Vec<f64>> = (0..n).map(|j| a.column(j).unwrap_or_default()).collect();
    let mut v: Vec<Vec<f64>> = (0..n)
        .map(|j| {
            let mut c = vec![0.0; n];
            if let Some(x) = c.get_mut(j) {
                *x = 1.0;
            }
            c
        })
        .collect();
    let eps = f64::EPSILON;
    let mut converged = false;
    for _sweep in 0..60 {
        let mut rotated = false;
        for p in 0..n {
            for q in (p + 1)..n {
                let (alpha, beta, gamma) = {
                    let (Some(cp), Some(cq)) = (u.get(p), u.get(q)) else {
                        continue;
                    };
                    let mut al = 0.0;
                    let mut be = 0.0;
                    let mut ga = 0.0;
                    for (x, y) in cp.iter().zip(cq) {
                        al += x * x;
                        be += y * y;
                        ga += x * y;
                    }
                    (al, be, ga)
                };
                if gamma == 0.0 || gamma.abs() <= eps * (alpha * beta).sqrt() {
                    continue;
                }
                rotated = true;
                let zeta = (beta - alpha) / (2.0 * gamma);
                let t = zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt());
                let c = 1.0 / (1.0 + t * t).sqrt();
                let s = c * t;
                rotate(&mut u, p, q, c, s);
                rotate(&mut v, p, q, c, s);
            }
        }
        if !rotated {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(NumError::NoConvergence {
            iterations: 60,
            residual: f64::NAN,
        });
    }
    let mut sv: Vec<(f64, usize)> = u
        .iter()
        .enumerate()
        .map(|(j, c)| (c.iter().map(|x| x * x).sum::<f64>().sqrt(), j))
        .collect();
    sv.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut ud = vec![0.0; m * n];
    let mut vd = vec![0.0; n * n];
    let mut s = Vec::with_capacity(n);
    let tiny = sv.first().map_or(0.0, |x| x.0) * eps * (m.max(n) as f64);
    for (k, &(sigma, j)) in sv.iter().enumerate() {
        s.push(sigma);
        let col = u.get(j).cloned().unwrap_or_default();
        let vcol = v.get(j).cloned().unwrap_or_default();
        for i in 0..m {
            let x = col.get(i).copied().unwrap_or(0.0);
            if let Some(slot) = ud.get_mut(i * n + k) {
                *slot = if sigma > tiny { x / sigma } else { 0.0 };
            }
        }
        for i in 0..n {
            if let Some(slot) = vd.get_mut(i * n + k) {
                *slot = vcol.get(i).copied().unwrap_or(0.0);
            }
        }
    }
    Ok(Svd {
        u: Matrix::new(m, n, ud)?,
        s,
        v: Matrix::new(n, n, vd)?,
    })
}

fn rotate(cols: &mut [Vec<f64>], p: usize, q: usize, c: f64, s: f64) {
    let (a, b) = if p < q { (p, q) } else { (q, p) };
    let (left, right) = cols.split_at_mut(b);
    if let (Some(cp), Some(cq)) = (left.get_mut(a), right.first_mut()) {
        for (x, y) in cp.iter_mut().zip(cq.iter_mut()) {
            let (xp, yq) = (*x, *y);
            *x = c * xp - s * yq;
            *y = s * xp + c * yq;
        }
    }
}

/// The numerical rank: the number of singular values above `tol`
/// (default `max(m, n) · σ₁ · ε`).
pub fn rank(a: &Matrix<f64>, tol: Option<f64>) -> Result<usize> {
    let d = svd(a)?;
    let smax = d.s.first().copied().unwrap_or(0.0);
    let tol = tol.unwrap_or_else(|| (a.rows().max(a.cols()) as f64) * smax * f64::EPSILON);
    Ok(d.s.iter().filter(|&&x| x > tol).count())
}

/// The Moore–Penrose pseudo-inverse.
pub fn pinv(a: &Matrix<f64>, tol: Option<f64>) -> Result<Matrix<f64>> {
    let d = svd(a)?;
    let (m, n) = (a.rows(), a.cols());
    let smax = d.s.first().copied().unwrap_or(0.0);
    let tol = tol.unwrap_or_else(|| (m.max(n) as f64) * smax * f64::EPSILON);
    let k = d.s.len();
    let mut out = vec![0.0; n * m];
    for (idx, &sigma) in d.s.iter().enumerate() {
        if sigma <= tol {
            continue;
        }
        for i in 0..n {
            let vi = d.v.get(i, idx).unwrap_or(0.0) / sigma;
            for j in 0..m {
                if let Some(slot) = out.get_mut(i * m + j) {
                    *slot += vi * d.u.get(j, idx).unwrap_or(0.0);
                }
            }
        }
    }
    let _ = k;
    Matrix::new(n, m, out)
}

/// An orthonormal basis of the null space (columns of the result).
pub fn null_space(a: &Matrix<f64>) -> Result<Matrix<f64>> {
    let (m, n) = (a.rows(), a.cols());
    // Use the full right factor: take the SVD of AᵀA's carrier by padding when m < n.
    let padded = if m < n {
        let mut data = a.as_slice().to_vec();
        data.extend(std::iter::repeat_n(0.0, (n - m) * n));
        Matrix::new(n, n, data)?
    } else {
        a.clone()
    };
    let d = svd(&padded)?;
    let smax = d.s.first().copied().unwrap_or(0.0);
    let tol = (m.max(n) as f64) * smax * f64::EPSILON;
    let keep: Vec<usize> =
        d.s.iter()
            .enumerate()
            .filter(|(_, &x)| x <= tol)
            .map(|(i, _)| i)
            .collect();
    let mut data = vec![0.0; n * keep.len()];
    for (c, &k) in keep.iter().enumerate() {
        for i in 0..n {
            if let Some(slot) = data.get_mut(i * keep.len() + c) {
                *slot = d.v.get(i, k).unwrap_or(0.0);
            }
        }
    }
    Matrix::new(n, keep.len(), data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn singular_values_of_a_known_matrix() {
        // [[3,0],[0,4]] has singular values 4 and 3.
        let a = Matrix::from_rows(&[[3.0, 0.0], [0.0, 4.0]]).unwrap();
        let d = svd(&a).unwrap();
        assert!(close(d.s[0], 4.0, 1e-14) && close(d.s[1], 3.0, 1e-14));
    }

    #[test]
    fn reconstructs_a_rectangular_matrix() {
        let a = Matrix::from_rows(&[[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]).unwrap();
        let d = svd(&a).unwrap();
        // known: 9.5255, 0.5143
        assert!(close(d.s[0], 9.525_518_091_565_107, 1e-12));
        assert!(close(d.s[1], 0.514_300_580_658_642_1, 1e-12));
        for i in 0..3 {
            for j in 0..2 {
                let mut x = 0.0;
                for k in 0..2 {
                    x += d.u.get(i, k).unwrap() * d.s[k] * d.v.get(j, k).unwrap();
                }
                assert!(close(x, a.get(i, j).unwrap(), 1e-12));
            }
        }
    }

    #[test]
    fn wide_matrices_work_and_rank_is_found() {
        let a = Matrix::from_rows(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0]]).unwrap();
        assert_eq!(rank(&a, None).unwrap(), 1);
        let n = null_space(&a).unwrap();
        assert_eq!(n.cols(), 2);
        // A * null = 0
        let z = a.matmul(&n).unwrap();
        assert!(z.as_slice().iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn pinv_solves_least_squares() {
        let a = Matrix::from_rows(&[[1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]).unwrap();
        let p = pinv(&a, None).unwrap();
        let id = p.matmul(&a).unwrap();
        assert!(
            close(id.get(0, 0).unwrap(), 1.0, 1e-12) && close(id.get(0, 1).unwrap(), 0.0, 1e-12)
        );
    }
}
