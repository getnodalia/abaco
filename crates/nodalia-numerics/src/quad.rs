// SPDX-License-Identifier: MIT OR Apache-2.0
//! Adaptive Gauss–Kronrod (G7, K15) quadrature, including infinite limits.
//!
//! The nodes and weights are the standard 7/15-point pair (Kronrod 1965, tabulated in QUADPACK).
//! Error control is global: the interval with the largest error estimate is bisected until the sum of
//! the estimates meets `max(abs_tol, rel_tol · |I|)`.

use crate::error::{NumError, Result};

const XGK: [f64; 8] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_4,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_47,
    0.0,
];
const WGK: [f64; 8] = [
    0.022_935_322_010_529_225,
    0.063_092_092_629_978_55,
    0.104_790_010_322_250_18,
    0.140_653_259_715_525_92,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_4,
    0.204_432_940_075_298_9,
    0.209_482_141_084_727_83,
];
const WG: [f64; 4] = [
    0.129_484_966_168_869_7,
    0.279_705_391_489_276_7,
    0.381_830_050_505_118_9,
    0.417_959_183_673_469_4,
];

/// The value of an integral and what it cost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Integral {
    /// The estimate.
    pub value: f64,
    /// An upper estimate of the absolute error.
    pub error: f64,
    /// Integrand evaluations.
    pub evaluations: usize,
    /// Number of intervals in the final subdivision.
    pub intervals: usize,
}

struct Piece {
    a: f64,
    b: f64,
    value: f64,
    error: f64,
}

fn k15(f: &mut impl FnMut(f64) -> f64, a: f64, b: f64) -> Result<Piece> {
    let centre = 0.5 * (a + b);
    let half = 0.5 * (b - a);
    let fc = f(centre);
    if !fc.is_finite() {
        return Err(NumError::NonFinite("integrand value"));
    }
    let mut kron = fc * WGK[7];
    let mut gauss = fc * WG[3];
    let mut resabs = kron.abs();
    for j in 0..7 {
        let dx = half * XGK[j];
        let (f1, f2) = (f(centre - dx), f(centre + dx));
        if !f1.is_finite() || !f2.is_finite() {
            return Err(NumError::NonFinite("integrand value"));
        }
        kron += WGK[j] * (f1 + f2);
        resabs += WGK[j] * (f1.abs() + f2.abs());
        if j % 2 == 1 {
            gauss += WG[j / 2] * (f1 + f2);
        }
    }
    let value = kron * half;
    let mut error = ((kron - gauss) * half).abs();
    let resabs = resabs * half.abs();
    // QUADPACK's error sharpening: the raw difference is very pessimistic for smooth functions.
    if resabs != 0.0 && error != 0.0 {
        error = resabs * (1.0f64).min((200.0 * error / resabs).powf(1.5));
    }
    if resabs > f64::MIN_POSITIVE / (50.0 * f64::EPSILON) {
        error = error.max(50.0 * f64::EPSILON * resabs);
    }
    Ok(Piece { a, b, value, error })
}

/// Integrates `f` over `[a, b]`; either limit may be infinite.
///
/// `max_intervals` bounds the work; running out of it is [`NumError::NoConvergence`] with the error
/// reached, never a silently inaccurate value.
pub fn integrate(
    mut f: impl FnMut(f64) -> f64,
    a: f64,
    b: f64,
    abs_tol: f64,
    rel_tol: f64,
    max_intervals: usize,
) -> Result<Integral> {
    if a.is_nan()
        || b.is_nan()
        || abs_tol.is_nan()
        || abs_tol < 0.0
        || rel_tol.is_nan()
        || rel_tol < 0.0
        || (abs_tol == 0.0 && rel_tol == 0.0)
    {
        return Err(NumError::InvalidInput(
            "the limits must not be NaN and at least one tolerance must be positive".into(),
        ));
    }
    if a == b {
        return Ok(Integral {
            value: 0.0,
            error: 0.0,
            evaluations: 0,
            intervals: 0,
        });
    }
    let (lo, hi, sign) = if a < b { (a, b, 1.0) } else { (b, a, -1.0) };
    let evals = std::cell::Cell::new(0usize);
    let mut counted = |x: f64| {
        evals.set(evals.get() + 1);
        f(x)
    };
    // Map infinite ranges onto a finite one; the Jacobian is folded into the integrand.
    let result = match (lo.is_infinite(), hi.is_infinite()) {
        (false, false) => run(&mut counted, lo, hi, abs_tol, rel_tol, max_intervals),
        (false, true) => {
            let mut g = |t: f64| {
                let x = lo + t / (1.0 - t);
                counted(x) / ((1.0 - t) * (1.0 - t))
            };
            run(&mut g, 0.0, 1.0, abs_tol, rel_tol, max_intervals)
        }
        (true, false) => {
            let mut g = |t: f64| {
                let x = hi - t / (1.0 - t);
                counted(x) / ((1.0 - t) * (1.0 - t))
            };
            run(&mut g, 0.0, 1.0, abs_tol, rel_tol, max_intervals)
        }
        (true, true) => {
            let mut g = |t: f64| {
                let d = 1.0 - t * t;
                let x = t / d;
                counted(x) * (1.0 + t * t) / (d * d)
            };
            run(&mut g, -1.0, 1.0, abs_tol, rel_tol, max_intervals)
        }
    }?;
    Ok(Integral {
        value: sign * result.value,
        evaluations: evals.get(),
        ..result
    })
}

fn run(
    f: &mut impl FnMut(f64) -> f64,
    a: f64,
    b: f64,
    abs_tol: f64,
    rel_tol: f64,
    max_intervals: usize,
) -> Result<Integral> {
    let mut pieces = vec![k15(f, a, b)?];
    loop {
        let value: f64 = pieces.iter().map(|p| p.value).sum();
        let error: f64 = pieces.iter().map(|p| p.error).sum();
        if !value.is_finite() || !error.is_finite() {
            return Err(NumError::NonFinite("integral"));
        }
        if error <= abs_tol.max(rel_tol * value.abs()) {
            return Ok(Integral {
                value,
                error,
                evaluations: 0,
                intervals: pieces.len(),
            });
        }
        if pieces.len() >= max_intervals.max(1) {
            return Err(NumError::NoConvergence {
                iterations: pieces.len(),
                residual: error,
            });
        }
        let (worst, _) = pieces
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.error.total_cmp(&y.1.error))
            .ok_or(NumError::InvalidInput("no intervals".into()))?;
        let p = pieces.swap_remove(worst);
        let m = 0.5 * (p.a + p.b);
        if m <= p.a || m >= p.b {
            return Err(NumError::NoConvergence {
                iterations: pieces.len(),
                residual: error,
            });
        }
        pieces.push(k15(f, p.a, m)?);
        pieces.push(k15(f, m, p.b)?);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn weights_sum_to_the_interval_length() {
        let k: f64 = WGK[..7].iter().sum::<f64>() * 2.0 + WGK[7];
        let g: f64 = WG[..3].iter().sum::<f64>() * 2.0 + WG[3];
        assert!(
            (k - 2.0).abs() < 1e-15 && (g - 2.0).abs() < 1e-15,
            "{k} {g}"
        );
    }

    #[test]
    fn kronrod_is_exact_to_degree_22_and_gauss_to_degree_13() {
        // Degree 12: both rules are exact, so the estimate is at the rounding floor and one
        // interval of 15 evaluations suffices.
        let r = integrate(|x| x.powi(12), 0.0, 1.0, 1e-14, 0.0, 1).unwrap();
        assert!((r.value - 1.0 / 13.0).abs() < 1e-15, "{}", r.value);
        assert_eq!((r.evaluations, r.intervals), (15, 1));
        // Degree 20: K15 is still exact (value), but the 7-point rule is not, so the error
        // estimate is honestly large and a tight tolerance is not claimed from one interval.
        let r = integrate(|x| x.powi(20), 0.0, 1.0, 1e-3, 0.0, 1).unwrap();
        assert!((r.value - 1.0 / 21.0).abs() < 1e-15, "{}", r.value);
        assert!(integrate(|x| x.powi(20), 0.0, 1.0, 1e-14, 0.0, 1).is_err());
    }

    #[test]
    fn smooth_and_oscillatory_integrals() {
        let r = integrate(f64::sin, 0.0, PI, 1e-13, 1e-13, 100).unwrap();
        assert!((r.value - 2.0).abs() < 1e-12);
        let r = integrate(|x| (50.0 * x).sin(), 0.0, PI, 1e-12, 1e-12, 500).unwrap();
        assert!((r.value - (1.0 - (50.0 * PI).cos()) / 50.0).abs() < 1e-11);
    }

    #[test]
    fn endpoint_singularity_is_resolved_by_subdivision() {
        // ∫0^1 x^-1/2 dx = 2
        let r = integrate(|x| 1.0 / x.sqrt(), 0.0, 1.0, 1e-9, 1e-9, 2000).unwrap();
        assert!((r.value - 2.0).abs() < 1e-8, "{}", r.value);
    }

    #[test]
    fn infinite_limits() {
        let r = integrate(
            |x| (-x * x).exp(),
            f64::NEG_INFINITY,
            f64::INFINITY,
            1e-12,
            1e-12,
            500,
        )
        .unwrap();
        assert!((r.value - PI.sqrt()).abs() < 1e-10, "{}", r.value);
        let r = integrate(|x| (-x).exp(), 0.0, f64::INFINITY, 1e-12, 1e-12, 500).unwrap();
        assert!((r.value - 1.0).abs() < 1e-10);
        let r = integrate(
            |x| 1.0 / (1.0 + x * x),
            f64::NEG_INFINITY,
            0.0,
            1e-12,
            1e-12,
            500,
        )
        .unwrap();
        assert!((r.value - PI / 2.0).abs() < 1e-10);
    }

    #[test]
    fn reversed_limits_negate() {
        let r = integrate(|x| x, 2.0, 0.0, 1e-13, 1e-13, 10).unwrap();
        assert!((r.value + 2.0).abs() < 1e-13);
    }

    #[test]
    fn divergent_integrals_and_bad_values_are_errors() {
        // ∫0^1 1/x diverges: the budget runs out and says so.
        assert!(integrate(|x| 1.0 / x, 0.0, 1.0, 1e-9, 1e-9, 200).is_err());
        assert!(matches!(
            integrate(|_| f64::NAN, 0.0, 1.0, 1e-9, 1e-9, 10),
            Err(NumError::NonFinite(_))
        ));
        assert!(integrate(|x| x, f64::NAN, 1.0, 1e-9, 1e-9, 10).is_err());
        assert!(integrate(|x| x, 0.0, 1.0, 0.0, 0.0, 10).is_err());
    }
}
