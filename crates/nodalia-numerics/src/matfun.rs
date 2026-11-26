// SPDX-License-Identifier: MIT OR Apache-2.0
//! Matrix functions: the exponential by scaling and squaring with a degree-13 Padé
//! approximant (Higham 2005, "The scaling and squaring method for the matrix exponential
//! revisited"), and eigenvectors of a general real matrix by inverse iteration.

use crate::eigen;
use crate::error::{NumError, Result};
use crate::lu::Lu;
use crate::matrix::Matrix;
use num_complex::Complex64;

/// Padé(13) coefficients.
const B: [f64; 14] = [
    64_764_752_532_480_000.0,
    32_382_376_266_240_000.0,
    7_771_770_303_897_600.0,
    1_187_353_796_428_800.0,
    129_060_195_264_000.0,
    10_559_470_521_600.0,
    670_442_572_800.0,
    33_522_128_640.0,
    1_323_241_920.0,
    40_840_800.0,
    960_960.0,
    16_380.0,
    182.0,
    1.0,
];

/// `exp(A)` for a real square matrix.
pub fn expm(a: &Matrix<f64>) -> Result<Matrix<f64>> {
    if !a.is_square() {
        return Err(NumError::DimensionMismatch(format!(
            "expm needs a square matrix, got {} x {}",
            a.rows(),
            a.cols()
        )));
    }
    let n = a.rows();
    if n == 0 {
        return Ok(a.clone());
    }
    // θ13 from Higham's table
    let theta13 = 5.371_920_351_148_152;
    let norm = a.norm_1();
    let s = if norm > theta13 {
        (norm / theta13).log2().ceil().max(0.0) as i32
    } else {
        0
    };
    if s > 200 {
        return Err(NumError::InvalidInput(
            "the matrix is too large for expm".into(),
        ));
    }
    let a = a.scale(0.5f64.powi(s))?;
    let id = Matrix::identity(n)?;
    let a2 = a.matmul(&a)?;
    let a4 = a2.matmul(&a2)?;
    let a6 = a4.matmul(&a2)?;
    let comb = |terms: &[(f64, &Matrix<f64>)]| -> Result<Matrix<f64>> {
        let mut out = Matrix::zeros(n, n)?;
        for (c, m) in terms {
            out = out.add(&m.scale(*c)?)?;
        }
        Ok(out)
    };
    let u_inner = comb(&[(B[13], &a6), (B[11], &a4), (B[9], &a2)])?;
    let u_inner = a6.matmul(&u_inner)?;
    let u_rest = comb(&[(B[7], &a6), (B[5], &a4), (B[3], &a2), (B[1], &id)])?;
    let u = a.matmul(&u_inner.add(&u_rest)?)?;
    let v_inner = comb(&[(B[12], &a6), (B[10], &a4), (B[8], &a2)])?;
    let v_inner = a6.matmul(&v_inner)?;
    let v_rest = comb(&[(B[6], &a6), (B[4], &a4), (B[2], &a2), (B[0], &id)])?;
    let v = v_inner.add(&v_rest)?;
    let p = v.add(&u)?;
    let q = v.sub(&u)?;
    let lu = Lu::factor(&q)?;
    let mut r = lu.solve_matrix(&p)?;
    for _ in 0..s {
        r = r.matmul(&r)?;
    }
    Ok(r)
}

/// Eigenvalues and unit-norm eigenvectors of a general real square matrix.
///
/// Eigenvectors come from inverse iteration with a complex shift just off each eigenvalue, so
/// they are accurate for simple eigenvalues. For repeated eigenvalues the returned vectors may
/// coincide when the matrix is defective, which is a property of the matrix.
pub fn eig_general(a: &Matrix<f64>) -> Result<(Vec<Complex64>, Matrix<Complex64>)> {
    let values = eigen::general(a)?;
    let n = a.rows();
    let mut cols: Vec<Vec<Complex64>> = Vec::with_capacity(n);
    let scale = a.norm_frobenius().max(f64::MIN_POSITIVE);
    for (k, &lam) in values.iter().enumerate() {
        // Shift slightly so the system is solvable but nearly singular along the eigenvector.
        let mut shift = lam + Complex64::new(scale * 1e-10, scale * 1e-10);
        // Separate repeated eigenvalues a little so successive vectors differ.
        let repeats = values
            .iter()
            .take(k)
            .filter(|m| (**m - lam).norm() < 1e-8 * scale)
            .count();
        shift += Complex64::new(scale * 1e-9 * repeats as f64, 0.0);
        let mut m = Matrix::<Complex64>::zeros(n, n)?;
        for i in 0..n {
            for j in 0..n {
                let mut x = Complex64::new(a.get(i, j).unwrap_or(0.0), 0.0);
                if i == j {
                    x -= shift;
                }
                m.set(i, j, x)?;
            }
        }
        let lu = Lu::factor(&m)?;
        // start vector with distinct components to avoid being orthogonal to the eigenvector
        let mut x: Vec<Complex64> = (0..n)
            .map(|i| Complex64::new(1.0 + 0.1 * i as f64, 0.0))
            .collect();
        for _ in 0..4 {
            x = lu.solve(&x)?;
            let nrm = x.iter().map(|z| z.norm_sqr()).sum::<f64>().sqrt();
            if !(nrm.is_finite() && nrm > 0.0) {
                return Err(NumError::NonFinite("eigenvector iteration"));
            }
            for z in &mut x {
                *z /= nrm;
            }
        }
        // normalise the phase: largest component real and positive
        let big = x
            .iter()
            .copied()
            .max_by(|p, q| p.norm().total_cmp(&q.norm()))
            .unwrap_or(Complex64::new(1.0, 0.0));
        let phase = big / big.norm();
        for z in &mut x {
            *z /= phase;
            if lam.im == 0.0 {
                z.im = 0.0;
            }
        }
        cols.push(x);
    }
    let mut data = vec![Complex64::new(0.0, 0.0); n * n];
    for (j, col) in cols.iter().enumerate() {
        for (i, z) in col.iter().enumerate() {
            if let Some(slot) = data.get_mut(i * n + j) {
                *slot = *z;
            }
        }
    }
    Ok((values, Matrix::new(n, n, data)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expm_of_a_diagonal_matrix() {
        let a = Matrix::from_rows(&[[1.0, 0.0], [0.0, 2.0]]).unwrap();
        let e = expm(&a).unwrap();
        assert!((e.get(0, 0).unwrap() - 1f64.exp()).abs() < 1e-12);
        assert!((e.get(1, 1).unwrap() - 2f64.exp()).abs() < 1e-12);
    }

    #[test]
    fn expm_of_a_rotation_generator() {
        // exp([[0,-t],[t,0]]) is a rotation by t
        let t = 0.7f64;
        let a = Matrix::from_rows(&[[0.0, -t], [t, 0.0]]).unwrap();
        let e = expm(&a).unwrap();
        assert!((e.get(0, 0).unwrap() - t.cos()).abs() < 1e-13);
        assert!((e.get(1, 0).unwrap() - t.sin()).abs() < 1e-13);
    }

    #[test]
    fn expm_handles_a_large_norm() {
        let a = Matrix::from_rows(&[[10.0, 0.0], [0.0, -10.0]]).unwrap();
        let e = expm(&a).unwrap();
        assert!((e.get(0, 0).unwrap() / 10f64.exp() - 1.0).abs() < 1e-12);
        assert!((e.get(1, 1).unwrap() / (-10f64).exp() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn eigenvectors_satisfy_the_equation() {
        let a = Matrix::from_rows(&[[2.0, 1.0, 0.0], [0.0, 3.0, 1.0], [1.0, 0.0, 4.0]]).unwrap();
        let (vals, vecs) = eig_general(&a).unwrap();
        for (k, lam) in vals.iter().enumerate() {
            for i in 0..3 {
                let mut av = Complex64::new(0.0, 0.0);
                for j in 0..3 {
                    av += Complex64::new(a.get(i, j).unwrap(), 0.0) * vecs.get(j, k).unwrap();
                }
                let lv = lam * vecs.get(i, k).unwrap();
                assert!(
                    (av - lv).norm() < 1e-8,
                    "residual {} for {lam}",
                    (av - lv).norm()
                );
            }
        }
    }
}
