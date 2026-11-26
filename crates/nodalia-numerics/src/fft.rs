// SPDX-License-Identifier: MIT OR Apache-2.0
//! The discrete Fourier transform of any length.
//!
//! Powers of two use an iterative radix-2 Cooley–Tukey transform. Every other length goes
//! through Bluestein's chirp-z algorithm, which expresses the transform as a convolution that
//! a power-of-two FFT evaluates, so the cost is `O(n log n)` for every `n` (never `O(n²)`).
//! The forward transform is `X[k] = Σ x[j] exp(-2πi jk/n)`; the inverse divides by `n`.

use crate::error::{NumError, Result};
use num_complex::Complex64;
use std::f64::consts::PI;

/// The longest transform accepted (2²⁴ points).
pub const MAX_LEN: usize = 1 << 24;

fn check_len(n: usize) -> Result<()> {
    if n > MAX_LEN {
        return Err(NumError::TooLarge(format!(
            "a transform of {n} points (the limit is {MAX_LEN})"
        )));
    }
    Ok(())
}

/// Forward DFT of `x`.
pub fn fft(x: &[Complex64]) -> Result<Vec<Complex64>> {
    transform(x, false)
}

/// Inverse DFT of `x` (includes the `1/n` factor).
pub fn ifft(x: &[Complex64]) -> Result<Vec<Complex64>> {
    transform(x, true)
}

/// Forward DFT of a real signal.
pub fn fft_real(x: &[f64]) -> Result<Vec<Complex64>> {
    let c: Vec<Complex64> = x.iter().map(|&v| Complex64::new(v, 0.0)).collect();
    fft(&c)
}

fn transform(x: &[Complex64], inverse: bool) -> Result<Vec<Complex64>> {
    let n = x.len();
    check_len(n)?;
    if x.iter().any(|v| !v.re.is_finite() || !v.im.is_finite()) {
        return Err(NumError::NonFinite("the signal"));
    }
    if n <= 1 {
        return Ok(x.to_vec());
    }
    let mut out = if inverse {
        // ifft(x) = conj(fft(conj(x))) / n
        let conj: Vec<Complex64> = x.iter().map(|v| v.conj()).collect();
        let mut y = forward(&conj);
        let s = 1.0 / n as f64;
        for v in &mut y {
            *v = v.conj() * s;
        }
        y
    } else {
        forward(x)
    };
    out.shrink_to_fit();
    Ok(out)
}

fn forward(x: &[Complex64]) -> Vec<Complex64> {
    let n = x.len();
    if n.is_power_of_two() {
        let mut a = x.to_vec();
        radix2(&mut a);
        a
    } else {
        bluestein(x)
    }
}

/// In-place iterative radix-2 transform; `a.len()` is a power of two.
fn radix2(a: &mut [Complex64]) {
    let n = a.len();
    if n <= 1 {
        return;
    }
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            a.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        // Twiddle factors for this stage, computed directly (not by repeated products) so the
        // error does not grow with the transform length.
        let tw: Vec<Complex64> = (0..half)
            .map(|k| Complex64::from_polar(1.0, -2.0 * PI * k as f64 / len as f64))
            .collect();
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (i, j) = (start + k, start + k + half);
                let (u, v) = match (a.get(i), a.get(j), tw.get(k)) {
                    (Some(&u), Some(&v), Some(&w)) => (u, v * w),
                    _ => continue,
                };
                if let Some(s) = a.get_mut(i) {
                    *s = u + v;
                }
                if let Some(s) = a.get_mut(j) {
                    *s = u - v;
                }
            }
        }
        len *= 2;
    }
}

/// Bluestein's algorithm for arbitrary `n`.
fn bluestein(x: &[Complex64]) -> Vec<Complex64> {
    let n = x.len();
    let m = (2 * n - 1).next_power_of_two();
    // chirp[k] = exp(-iπ k²/n); k² is reduced modulo 2n so the angle stays small and exact.
    let chirp: Vec<Complex64> = (0..n)
        .map(|k| {
            let kk = (k as u128 * k as u128 % (2 * n as u128)) as f64;
            Complex64::from_polar(1.0, -PI * kk / n as f64)
        })
        .collect();
    let mut a = vec![Complex64::new(0.0, 0.0); m];
    for (k, slot) in a.iter_mut().take(n).enumerate() {
        if let (Some(&xv), Some(&c)) = (x.get(k), chirp.get(k)) {
            *slot = xv * c;
        }
    }
    let mut b = vec![Complex64::new(0.0, 0.0); m];
    if let Some(first) = b.first_mut() {
        *first = Complex64::new(1.0, 0.0);
    }
    for k in 1..n {
        if let Some(&c) = chirp.get(k) {
            let cc = c.conj();
            if let Some(s) = b.get_mut(k) {
                *s = cc;
            }
            if let Some(s) = b.get_mut(m - k) {
                *s = cc;
            }
        }
    }
    radix2(&mut a);
    radix2(&mut b);
    for (u, v) in a.iter_mut().zip(&b) {
        *u *= *v;
    }
    // Inverse transform of the product (via conjugation).
    for v in &mut a {
        *v = v.conj();
    }
    radix2(&mut a);
    let s = 1.0 / m as f64;
    (0..n)
        .map(|k| match (a.get(k), chirp.get(k)) {
            (Some(&v), Some(&c)) => v.conj() * s * c,
            _ => Complex64::new(0.0, 0.0),
        })
        .collect()
}

/// Linear convolution of two real sequences through the FFT (length `a.len() + b.len() - 1`).
pub fn convolve(a: &[f64], b: &[f64]) -> Result<Vec<f64>> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    let n = a.len() + b.len() - 1;
    let m = n.next_power_of_two();
    check_len(m)?;
    let mut fa = vec![Complex64::new(0.0, 0.0); m];
    let mut fb = vec![Complex64::new(0.0, 0.0); m];
    for (s, &v) in fa.iter_mut().zip(a) {
        *s = Complex64::new(v, 0.0);
    }
    for (s, &v) in fb.iter_mut().zip(b) {
        *s = Complex64::new(v, 0.0);
    }
    radix2(&mut fa);
    radix2(&mut fb);
    for (u, v) in fa.iter_mut().zip(&fb) {
        *u *= *v;
    }
    let r = ifft(&fa)?;
    Ok(r.into_iter().take(n).map(|c| c.re).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(x: &[Complex64]) -> Vec<Complex64> {
        let n = x.len();
        (0..n)
            .map(|k| {
                x.iter()
                    .enumerate()
                    .map(|(j, &v)| {
                        v * Complex64::from_polar(1.0, -2.0 * PI * ((j * k) % n) as f64 / n as f64)
                    })
                    .sum()
            })
            .collect()
    }

    fn sample(n: usize) -> Vec<Complex64> {
        (0..n)
            .map(|i| Complex64::new((i as f64 * 0.7).sin() + 0.1 * i as f64, (i as f64).cos()))
            .collect()
    }

    #[test]
    fn matches_the_definition_for_every_length_up_to_40() {
        for n in 1..=40 {
            let x = sample(n);
            let got = fft(&x).unwrap();
            let want = naive(&x);
            for (g, w) in got.iter().zip(&want) {
                assert!(
                    (g - w).norm() < 1e-9 * (1.0 + w.norm()),
                    "n={n}: {g} vs {w}"
                );
            }
        }
    }

    #[test]
    fn inverse_undoes_forward() {
        for &n in &[1usize, 2, 7, 16, 100, 127, 1000] {
            let x = sample(n);
            let back = ifft(&fft(&x).unwrap()).unwrap();
            for (a, b) in x.iter().zip(&back) {
                assert!((a - b).norm() < 1e-10, "n={n}");
            }
        }
    }

    #[test]
    fn a_pure_tone_lands_in_one_bin() {
        let n = 64;
        let x: Vec<f64> = (0..n)
            .map(|j| (2.0 * PI * 5.0 * j as f64 / n as f64).cos())
            .collect();
        let s = fft_real(&x).unwrap();
        for (k, v) in s.iter().enumerate() {
            let want = if k == 5 || k == n - 5 {
                n as f64 / 2.0
            } else {
                0.0
            };
            assert!((v.norm() - want).abs() < 1e-9, "bin {k}");
        }
    }

    #[test]
    fn parseval_holds() {
        let x = sample(97);
        let s = fft(&x).unwrap();
        let et: f64 = x.iter().map(|v| v.norm_sqr()).sum();
        let ef: f64 = s.iter().map(|v| v.norm_sqr()).sum::<f64>() / 97.0;
        assert!((et - ef).abs() < 1e-8 * et);
    }

    #[test]
    fn convolution_matches_the_direct_sum() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [0.5, -1.0, 2.0];
        let got = convolve(&a, &b).unwrap();
        let mut want = vec![0.0; a.len() + b.len() - 1];
        for (i, &x) in a.iter().enumerate() {
            for (j, &y) in b.iter().enumerate() {
                want[i + j] += x * y;
            }
        }
        for (g, w) in got.iter().zip(&want) {
            assert!((g - w).abs() < 1e-12);
        }
    }

    #[test]
    fn refuses_hostile_input() {
        assert!(fft(&[Complex64::new(f64::NAN, 0.0), Complex64::new(1.0, 0.0)]).is_err());
        assert_eq!(fft(&[]).unwrap().len(), 0);
    }
}
