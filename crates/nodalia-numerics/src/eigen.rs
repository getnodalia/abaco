// SPDX-License-Identifier: MIT OR Apache-2.0
//! Eigenvalues of real matrices.
//!
//! * [`symmetric`]: cyclic Jacobi rotations (Rutishauser 1966, as in Golub & Van Loan §8.5). Gives
//!   eigenvalues in ascending order and orthonormal eigenvectors, accurate to a small multiple of the
//!   machine epsilon times `‖A‖`.
//! * [`general`]: balancing, Householder reduction to Hessenberg form and the Francis double-shift QR
//!   iteration with deflation (Martin, Peters and Wilkinson 1970). Eigenvalues only.

use crate::error::{NumError, Result};
use crate::matrix::Matrix;
use num_complex::Complex64;

/// Maximum size accepted by the eigenvalue routines.
pub const MAX_ORDER: usize = 4096;

/// The result of [`symmetric`].
#[derive(Debug, Clone, PartialEq)]
pub struct SymmetricEigen {
    /// Eigenvalues in ascending order.
    pub values: Vec<f64>,
    /// Orthonormal eigenvectors: column `k` belongs to `values[k]`.
    pub vectors: Matrix<f64>,
    /// Number of Jacobi sweeps it took.
    pub sweeps: usize,
}

fn check_square(a: &Matrix<f64>) -> Result<usize> {
    if !a.is_square() {
        return Err(NumError::DimensionMismatch(format!(
            "eigenvalues need a square matrix, got {} x {}",
            a.rows(),
            a.cols()
        )));
    }
    let n = a.rows();
    if n > MAX_ORDER {
        return Err(NumError::TooLarge(format!(
            "eigenvalue problems are limited to order {MAX_ORDER}, got {n}"
        )));
    }
    if a.as_slice().iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("matrix entry"));
    }
    Ok(n)
}

/// Eigenvalues and eigenvectors of a symmetric matrix by cyclic Jacobi rotations.
///
/// The matrix must be symmetric to within `1e-10 · ‖A‖`; a matrix that is not is refused rather than
/// silently symmetrised.
pub fn symmetric(a: &Matrix<f64>) -> Result<SymmetricEigen> {
    let n = check_square(a)?;
    let scale = a.norm_frobenius();
    for i in 0..n {
        for j in 0..i {
            let (x, y) = (a.get(i, j).unwrap_or(0.0), a.get(j, i).unwrap_or(0.0));
            if (x - y).abs() > 1e-10 * scale.max(f64::MIN_POSITIVE) {
                return Err(NumError::InvalidInput(format!(
                    "the matrix is not symmetric: a[{i}][{j}] = {x} but a[{j}][{i}] = {y}"
                )));
            }
        }
    }
    let mut m: Vec<f64> = a.as_slice().to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    let mut sweeps = 0;
    const MAX_SWEEPS: usize = 100;
    loop {
        let mut off = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                off += m[i * n + j] * m[i * n + j];
            }
        }
        if off.sqrt() <= f64::EPSILON * scale || off == 0.0 {
            break;
        }
        if sweeps >= MAX_SWEEPS {
            return Err(NumError::NoConvergence {
                iterations: sweeps,
                residual: off.sqrt(),
            });
        }
        sweeps += 1;
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = m[p * n + q];
                if apq == 0.0 {
                    continue;
                }
                let theta = (m[q * n + q] - m[p * n + p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (m[k * n + p], m[k * n + q]);
                    m[k * n + p] = c * akp - s * akq;
                    m[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (m[p * n + k], m[q * n + k]);
                    m[p * n + k] = c * apk - s * aqk;
                    m[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| m[i * n + i].total_cmp(&m[j * n + j]));
    let values: Vec<f64> = order.iter().map(|&i| m[i * n + i]).collect();
    let mut vecs = vec![0.0; n * n];
    for (new, &old) in order.iter().enumerate() {
        for k in 0..n {
            vecs[k * n + new] = v[k * n + old];
        }
    }
    Ok(SymmetricEigen {
        values,
        vectors: Matrix::new(n, n, vecs)?,
        sweeps,
    })
}

/// Eigenvalues of a general real square matrix, sorted by real part and then imaginary part.
pub fn general(a: &Matrix<f64>) -> Result<Vec<Complex64>> {
    let n = check_square(a)?;
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut h: Vec<Vec<f64>> = (0..n)
        .map(|i| a.row(i).map(<[f64]>::to_vec).unwrap_or_default())
        .collect();
    balance(&mut h);
    hessenberg(&mut h);
    let mut ev = hqr(&mut h)?;
    ev.sort_by(|x, y| x.re.total_cmp(&y.re).then(x.im.total_cmp(&y.im)));
    Ok(ev)
}

/// Parlett–Reinsch balancing by powers of two (does not change the eigenvalues).
fn balance(h: &mut [Vec<f64>]) {
    let n = h.len();
    for _ in 0..64 {
        let mut done = true;
        for i in 0..n {
            let mut c = 0.0;
            let mut r = 0.0;
            for j in 0..n {
                if j != i {
                    c += h[j][i].abs();
                    r += h[i][j].abs();
                }
            }
            if c == 0.0 || r == 0.0 {
                continue;
            }
            let s = c + r;
            let mut f = 1.0;
            let mut g = r / 2.0;
            while c < g {
                f *= 2.0;
                c *= 4.0;
            }
            g = r * 2.0;
            while c > g {
                f /= 2.0;
                c /= 4.0;
            }
            if (c + r) / f < 0.95 * s {
                done = false;
                for x in h[i].iter_mut() {
                    *x /= f;
                }
                for row in h.iter_mut() {
                    row[i] *= f;
                }
            }
        }
        if done {
            break;
        }
    }
}

/// Householder reduction to upper Hessenberg form (similarity transform).
fn hessenberg(h: &mut [Vec<f64>]) {
    let n = h.len();
    if n < 3 {
        return;
    }
    for k in 0..(n - 2) {
        let norm = (k + 1..n).map(|i| h[i][k] * h[i][k]).sum::<f64>().sqrt();
        if norm == 0.0 {
            continue;
        }
        let alpha = if h[k + 1][k] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (k + 1..n).map(|i| h[i][k]).collect();
        v[0] -= alpha;
        let vv: f64 = v.iter().map(|x| x * x).sum();
        if vv == 0.0 {
            continue;
        }
        for j in 0..n {
            let dot: f64 = v
                .iter()
                .enumerate()
                .map(|(t, vi)| vi * h[k + 1 + t][j])
                .sum();
            let f = 2.0 * dot / vv;
            for (t, vi) in v.iter().enumerate() {
                h[k + 1 + t][j] -= f * vi;
            }
        }
        for row in h.iter_mut() {
            let dot: f64 = v
                .iter()
                .enumerate()
                .map(|(t, vi)| vi * row[k + 1 + t])
                .sum();
            let f = 2.0 * dot / vv;
            for (t, vi) in v.iter().enumerate() {
                row[k + 1 + t] -= f * vi;
            }
        }
        for row in h.iter_mut().skip(k + 2) {
            row[k] = 0.0;
        }
    }
}

fn sign(a: f64, b: f64) -> f64 {
    if b >= 0.0 {
        a.abs()
    } else {
        -a.abs()
    }
}

/// Francis double-shift QR on an upper Hessenberg matrix; returns all eigenvalues.
fn hqr(h: &mut [Vec<f64>]) -> Result<Vec<Complex64>> {
    let n = h.len();
    let mut ev = vec![Complex64::new(0.0, 0.0); n];
    let mut anorm = 0.0;
    for i in 0..n {
        for j in i.saturating_sub(1)..n {
            anorm += h[i][j].abs();
        }
    }
    let eps = f64::EPSILON;
    let mut active = n; // rows 0..active still hold unknown eigenvalues
    let mut t = 0.0;
    while active > 0 {
        let hi = active - 1;
        let mut its = 0usize;
        loop {
            // Look for a negligible sub-diagonal element.
            let mut l = hi;
            while l >= 1 {
                let mut s = h[l - 1][l - 1].abs() + h[l][l].abs();
                if s == 0.0 {
                    s = anorm;
                }
                if h[l][l - 1].abs() <= eps * s {
                    h[l][l - 1] = 0.0;
                    break;
                }
                l -= 1;
            }
            let mut x = h[hi][hi];
            if l == hi {
                ev[hi] = Complex64::new(x + t, 0.0);
                active -= 1;
                break;
            }
            let mut y = h[hi - 1][hi - 1];
            let mut w = h[hi][hi - 1] * h[hi - 1][hi];
            if l == hi - 1 {
                let p = 0.5 * (y - x);
                let q = p * p + w;
                let z = q.abs().sqrt();
                let xt = x + t;
                if q >= 0.0 {
                    let z = p + sign(z, p);
                    let r1 = xt + z;
                    let r2 = if z != 0.0 { xt - w / z } else { r1 };
                    ev[hi - 1] = Complex64::new(r1, 0.0);
                    ev[hi] = Complex64::new(r2, 0.0);
                } else {
                    ev[hi - 1] = Complex64::new(xt + p, z);
                    ev[hi] = Complex64::new(xt + p, -z);
                }
                active -= 2;
                break;
            }
            if its >= 60 {
                return Err(NumError::NoConvergence {
                    iterations: its,
                    residual: h[hi][hi - 1].abs(),
                });
            }
            if its == 10 || its == 20 {
                // Exceptional shift.
                t += x;
                for (i, row) in h.iter_mut().enumerate().take(hi + 1) {
                    row[i] -= x;
                }
                let s = h[hi][hi - 1].abs() + h[hi - 1][hi - 2].abs();
                x = 0.75 * s;
                y = x;
                w = -0.4375 * s * s;
            }
            its += 1;
            // Look for two consecutive small sub-diagonal elements.
            let mut m = hi - 2;
            let (mut p, mut q, mut r);
            loop {
                let z = h[m][m];
                let rr = x - z;
                let ss = y - z;
                p = (rr * ss - w) / h[m + 1][m] + h[m][m + 1];
                q = h[m + 1][m + 1] - z - rr - ss;
                r = h[m + 2][m + 1];
                let s = p.abs() + q.abs() + r.abs();
                if s != 0.0 {
                    p /= s;
                    q /= s;
                    r /= s;
                }
                if m == l {
                    break;
                }
                let u = h[m][m - 1].abs() * (q.abs() + r.abs());
                let v = p.abs() * (h[m - 1][m - 1].abs() + z.abs() + h[m + 1][m + 1].abs());
                if u <= eps * v {
                    break;
                }
                m -= 1;
            }
            for i in (m + 2)..=hi {
                h[i][i - 2] = 0.0;
                if i != m + 2 {
                    h[i][i - 3] = 0.0;
                }
            }
            // Double QR step on rows l..=hi and columns m..=hi.
            for k in m..hi {
                if k != m {
                    p = h[k][k - 1];
                    q = h[k + 1][k - 1];
                    r = if k != hi - 1 { h[k + 2][k - 1] } else { 0.0 };
                    x = p.abs() + q.abs() + r.abs();
                    if x != 0.0 {
                        p /= x;
                        q /= x;
                        r /= x;
                    }
                }
                let s = sign((p * p + q * q + r * r).sqrt(), p);
                if s == 0.0 {
                    continue;
                }
                if k == m {
                    if l != m {
                        h[k][k - 1] = -h[k][k - 1];
                    }
                } else {
                    h[k][k - 1] = -s * x;
                }
                p += s;
                x = p / s;
                y = q / s;
                let z = r / s;
                q /= p;
                r /= p;
                for j in k..=hi {
                    let mut pp = h[k][j] + q * h[k + 1][j];
                    if k != hi - 1 {
                        pp += r * h[k + 2][j];
                        h[k + 2][j] -= pp * z;
                    }
                    h[k + 1][j] -= pp * y;
                    h[k][j] -= pp * x;
                }
                let mmin = if hi < k + 3 { hi } else { k + 3 };
                for row in h.iter_mut().take(mmin + 1).skip(l) {
                    let mut pp = x * row[k] + y * row[k + 1];
                    if k != hi - 1 {
                        pp += z * row[k + 2];
                        row[k + 2] -= pp * r;
                    }
                    row[k + 1] -= pp * q;
                    row[k] -= pp;
                }
            }
        }
    }
    if ev.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
        return Err(NumError::NonFinite("eigenvalue"));
    }
    Ok(ev)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn second_difference(n: usize) -> Matrix<f64> {
        let mut m = Matrix::zeros(n, n).unwrap();
        for i in 0..n {
            m.set(i, i, 2.0).unwrap();
            if i + 1 < n {
                m.set(i, i + 1, -1.0).unwrap();
                m.set(i + 1, i, -1.0).unwrap();
            }
        }
        m
    }

    #[test]
    fn symmetric_2x2() {
        let a = Matrix::from_rows(&[[2.0, 1.0], [1.0, 2.0]]).unwrap();
        let e = symmetric(&a).unwrap();
        assert!((e.values[0] - 1.0).abs() < 1e-14 && (e.values[1] - 3.0).abs() < 1e-14);
    }

    #[test]
    fn second_difference_matrix_matches_the_closed_form() {
        // Eigenvalues of tridiag(-1, 2, -1) of order n are 2 - 2 cos(k pi / (n + 1)).
        let n = 12;
        let a = second_difference(n);
        let e = symmetric(&a).unwrap();
        for (k, v) in e.values.iter().enumerate() {
            let exact =
                2.0 - 2.0 * (((k + 1) as f64) * std::f64::consts::PI / (n as f64 + 1.0)).cos();
            assert!((v - exact).abs() < 1e-13, "k={k}: {v} vs {exact}");
        }
        // A v = lambda v and V orthonormal.
        for k in 0..n {
            let col = e.vectors.column(k).unwrap();
            let av = a.mul_vec(&col).unwrap();
            for i in 0..n {
                assert!((av[i] - e.values[k] * col[i]).abs() < 1e-12);
            }
        }
        let vtv = e.vectors.transpose().matmul(&e.vectors).unwrap();
        for i in 0..n {
            for j in 0..n {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((vtv.get(i, j).unwrap() - want).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn non_symmetric_input_is_refused_by_symmetric() {
        let a = Matrix::from_rows(&[[1.0, 2.0], [0.0, 1.0]]).unwrap();
        assert!(matches!(symmetric(&a), Err(NumError::InvalidInput(_))));
    }

    #[test]
    fn rotation_has_conjugate_imaginary_pair() {
        let a = Matrix::from_rows(&[[0.0, -1.0], [1.0, 0.0]]).unwrap();
        let ev = general(&a).unwrap();
        assert!(ev[0].re.abs() < 1e-14 && (ev[0].im + 1.0).abs() < 1e-14);
        assert!(ev[1].re.abs() < 1e-14 && (ev[1].im - 1.0).abs() < 1e-14);
    }

    #[test]
    fn companion_of_1_2_3_has_those_roots() {
        // x^3 - 6x^2 + 11x - 6
        let a = Matrix::from_rows(&[[6.0, -11.0, 6.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]).unwrap();
        let ev = general(&a).unwrap();
        for (got, want) in ev.iter().zip([1.0, 2.0, 3.0]) {
            assert!(
                (got.re - want).abs() < 1e-12 && got.im.abs() < 1e-12,
                "{got}"
            );
        }
    }

    #[test]
    fn general_agrees_with_symmetric_on_a_symmetric_matrix() {
        let a = second_difference(9);
        let s = symmetric(&a).unwrap();
        let g = general(&a).unwrap();
        for (x, y) in s.values.iter().zip(&g) {
            assert!((x - y.re).abs() < 1e-11 && y.im.abs() < 1e-11);
        }
    }

    #[test]
    fn trace_and_determinant_are_preserved_on_pseudo_random_matrices() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for n in [3usize, 5, 8, 13, 20] {
            let data: Vec<f64> = (0..n * n).map(|_| next()).collect();
            let a = Matrix::new(n, n, data).unwrap();
            let ev = general(&a).unwrap();
            let tr: Complex64 = ev.iter().sum();
            assert!((tr.re - a.trace().unwrap()).abs() < 1e-10, "n={n}");
            assert!(tr.im.abs() < 1e-10);
            let det: Complex64 = ev.iter().product();
            let want = crate::lu::Lu::factor(&a).unwrap().det();
            assert!(
                (det.re - want).abs() < 1e-9 * (1.0 + want.abs()),
                "n={n}: {} vs {want}",
                det.re
            );
        }
    }

    #[test]
    fn defective_and_triangular_matrices_work() {
        // Jordan block: eigenvalue 2 with multiplicity 3 (accuracy limited to eps^(1/3)).
        let a = Matrix::from_rows(&[[2.0, 1.0, 0.0], [0.0, 2.0, 1.0], [0.0, 0.0, 2.0]]).unwrap();
        for z in general(&a).unwrap() {
            assert!((z.re - 2.0).abs() < 1e-4 && z.im.abs() < 1e-4);
        }
        let t = Matrix::from_rows(&[[1.0, 5.0, 7.0], [0.0, 4.0, 9.0], [0.0, 0.0, -3.0]]).unwrap();
        let ev = general(&t).unwrap();
        assert!(
            (ev[0].re + 3.0).abs() < 1e-13
                && (ev[1].re - 1.0).abs() < 1e-13
                && (ev[2].re - 4.0).abs() < 1e-13
        );
    }

    #[test]
    fn non_finite_and_non_square_are_errors() {
        // Matrices refuse non-finite entries at construction, so the guard in `check_square` is
        // defence in depth.
        assert!(Matrix::from_rows(&[[f64::NAN, 0.0], [0.0, 1.0]]).is_err());
        let b = Matrix::from_rows(&[[1.0, 2.0, 3.0]]).unwrap();
        assert!(general(&b).is_err() && symmetric(&b).is_err());
    }
}
