// SPDX-License-Identifier: MIT OR Apache-2.0
//! Initial-value problems `y' = f(t, y)` by the Dormand–Prince 5(4) embedded Runge–Kutta pair
//! (Dormand & Prince 1980) with adaptive step-size control.
//!
//! Steps are clipped so the solver lands exactly on every requested output time; there is no
//! interpolation, so the reported values carry the full accuracy of the method.

use crate::error::{NumError, Result};
use std::sync::atomic::{AtomicBool, Ordering};

/// Solver settings.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// Relative tolerance per component.
    pub rtol: f64,
    /// Absolute tolerance per component.
    pub atol: f64,
    /// First step, or `None` to estimate it.
    pub initial_step: Option<f64>,
    /// Largest step.
    pub max_step: f64,
    /// Budget of accepted plus rejected steps.
    pub max_steps: usize,
    /// Set from another thread to stop with [`NumError::Cancelled`].
    pub cancel: Option<&'a AtomicBool>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            rtol: 1e-6,
            atol: 1e-9,
            initial_step: None,
            max_step: f64::INFINITY,
            max_steps: 1_000_000,
            cancel: None,
        }
    }
}

/// The solution at the requested times and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    /// The output times, as requested.
    pub t: Vec<f64>,
    /// `y[k]` is the state at `t[k]`.
    pub y: Vec<Vec<f64>>,
    /// Accepted steps.
    pub accepted: usize,
    /// Rejected steps.
    pub rejected: usize,
    /// Evaluations of `f`.
    pub evaluations: usize,
}

const C: [f64; 7] = [0.0, 0.2, 0.3, 0.8, 8.0 / 9.0, 1.0, 1.0];
const A: [[f64; 6]; 7] = [
    [0.0; 6],
    [0.2, 0.0, 0.0, 0.0, 0.0, 0.0],
    [3.0 / 40.0, 9.0 / 40.0, 0.0, 0.0, 0.0, 0.0],
    [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0, 0.0, 0.0, 0.0],
    [
        19372.0 / 6561.0,
        -25360.0 / 2187.0,
        64448.0 / 6561.0,
        -212.0 / 729.0,
        0.0,
        0.0,
    ],
    [
        9017.0 / 3168.0,
        -355.0 / 33.0,
        46732.0 / 5247.0,
        49.0 / 176.0,
        -5103.0 / 18656.0,
        0.0,
    ],
    [
        35.0 / 384.0,
        0.0,
        500.0 / 1113.0,
        125.0 / 192.0,
        -2187.0 / 6784.0,
        11.0 / 84.0,
    ],
];
const B5: [f64; 7] = [
    35.0 / 384.0,
    0.0,
    500.0 / 1113.0,
    125.0 / 192.0,
    -2187.0 / 6784.0,
    11.0 / 84.0,
    0.0,
];
const B4: [f64; 7] = [
    5179.0 / 57600.0,
    0.0,
    7571.0 / 16695.0,
    393.0 / 640.0,
    -92097.0 / 339200.0,
    187.0 / 2100.0,
    1.0 / 40.0,
];

/// Integrates from `t0` with state `y0` and reports the state at each time of `t_out`
/// (non-decreasing, none before `t0`). `f(t, y, dy)` writes the derivative into `dy`.
pub fn solve(
    mut f: impl FnMut(f64, &[f64], &mut [f64]),
    t0: f64,
    y0: &[f64],
    t_out: &[f64],
    opts: &Options<'_>,
) -> Result<Solution> {
    let n = y0.len();
    if !t0.is_finite() || y0.iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("initial condition"));
    }
    if !(opts.rtol > 0.0 && opts.rtol.is_finite() && opts.atol >= 0.0 && opts.atol.is_finite()) {
        return Err(NumError::InvalidInput(
            "rtol must be positive and atol non-negative".into(),
        ));
    }
    let mut prev = t0;
    for &t in t_out {
        if !t.is_finite() || t < prev {
            return Err(NumError::InvalidInput(
                "output times must be finite, non-decreasing and not before the start".into(),
            ));
        }
        prev = t;
    }
    let mut out = Solution {
        t: t_out.to_vec(),
        y: Vec::with_capacity(t_out.len()),
        accepted: 0,
        rejected: 0,
        evaluations: 0,
    };
    let mut t = t0;
    let mut y = y0.to_vec();
    let mut k: Vec<Vec<f64>> = vec![vec![0.0; n]; 7];
    let mut tmp = vec![0.0; n];
    let mut ynew = vec![0.0; n];

    let mut call = |t: f64, y: &[f64], dy: &mut [f64], out: &mut Solution| -> Result<()> {
        f(t, y, dy);
        out.evaluations += 1;
        if dy.iter().any(|v| !v.is_finite()) {
            return Err(NumError::NonFinite("derivative"));
        }
        Ok(())
    };

    call(t, &y, &mut k[0], &mut out)?;
    let scale = |y: &[f64], i: usize| opts.atol + opts.rtol * y[i].abs();
    let mut h = match opts.initial_step {
        Some(h) if h > 0.0 && h.is_finite() => h,
        Some(_) => {
            return Err(NumError::InvalidInput(
                "the initial step must be positive".into(),
            ))
        }
        None => {
            // Hairer, Nørsett & Wanner, II.4: estimate from the sizes of y and f.
            let d0 = rms((0..n).map(|i| y[i] / scale(&y, i)));
            let d1 = rms((0..n).map(|i| k[0][i] / scale(&y, i)));
            let h0 = if d0 < 1e-5 || d1 < 1e-5 {
                1e-6
            } else {
                0.01 * d0 / d1
            };
            let y1: Vec<f64> = (0..n).map(|i| y[i] + h0 * k[0][i]).collect();
            let mut f1 = vec![0.0; n];
            call(t + h0, &y1, &mut f1, &mut out)?;
            let d2 = rms((0..n).map(|i| (f1[i] - k[0][i]) / scale(&y, i))) / h0;
            let h1 = if d1.max(d2) <= 1e-15 {
                (h0 * 1e-3).max(1e-6)
            } else {
                (0.01 / d1.max(d2)).powf(0.2)
            };
            (100.0 * h0).min(h1)
        }
    }
    .min(opts.max_step);

    let mut steps = 0usize;
    for &target in t_out {
        while t < target {
            if let Some(c) = opts.cancel {
                if c.load(Ordering::Relaxed) {
                    return Err(NumError::Cancelled);
                }
            }
            steps += 1;
            if steps > opts.max_steps {
                return Err(NumError::NoConvergence {
                    iterations: steps,
                    residual: (target - t).abs(),
                });
            }
            let mut h_try = h.min(target - t);
            // Avoid a vanishing last step: stretch this one to the target if close.
            if target - t - h_try < 1e-3 * h_try {
                h_try = target - t;
            }
            if h_try < 16.0 * f64::EPSILON * t.abs().max(1e-300) {
                return Err(NumError::StepTooSmall { t });
            }
            for s in 1..7 {
                for i in 0..n {
                    let mut acc = 0.0;
                    for (j, kj) in k.iter().enumerate().take(s) {
                        acc += A[s][j] * kj[i];
                    }
                    tmp[i] = y[i] + h_try * acc;
                }
                let ts = t + C[s] * h_try;
                let (head, tail) = k.split_at_mut(s);
                let _ = head;
                call(ts, &tmp, &mut tail[0], &mut out)?;
            }
            for i in 0..n {
                let mut acc = 0.0;
                for (s, ks) in k.iter().enumerate().take(6) {
                    acc += B5[s] * ks[i];
                }
                ynew[i] = y[i] + h_try * acc;
            }
            // The 7th stage of the pair is f(t+h, ynew) (first-same-as-last), already in k[6].
            let err = rms((0..n).map(|i| {
                let e: f64 = (0..7).map(|s| (B5[s] - B4[s]) * k[s][i]).sum::<f64>() * h_try;
                e / (opts.atol + opts.rtol * y[i].abs().max(ynew[i].abs()))
            }));
            if !err.is_finite() {
                return Err(NumError::NonFinite("error estimate"));
            }
            if err <= 1.0 {
                t = if h_try == target - t {
                    target
                } else {
                    t + h_try
                };
                y.copy_from_slice(&ynew);
                let last = k[6].clone();
                k[0] = last;
                out.accepted += 1;
                let factor = if err == 0.0 {
                    10.0
                } else {
                    (0.9 * err.powf(-0.2)).clamp(0.2, 10.0)
                };
                h = (h_try * factor).min(opts.max_step);
            } else {
                out.rejected += 1;
                h = h_try * (0.9 * err.powf(-0.2)).clamp(0.1, 0.9);
            }
        }
        out.y.push(y.clone());
    }
    Ok(out)
}

fn rms(it: impl Iterator<Item = f64>) -> f64 {
    let (mut s, mut c) = (0.0, 0usize);
    for v in it {
        s += v * v;
        c += 1;
    }
    if c == 0 {
        0.0
    } else {
        (s / c as f64).sqrt()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn tight() -> Options<'static> {
        Options {
            rtol: 1e-10,
            atol: 1e-12,
            ..Options::default()
        }
    }

    #[test]
    fn coefficient_rows_sum_to_c() {
        for s in 1..7 {
            let sum: f64 = A[s].iter().sum();
            assert!((sum - C[s]).abs() < 1e-14, "row {s}: {sum} vs {}", C[s]);
        }
        assert!((B5.iter().sum::<f64>() - 1.0).abs() < 1e-14);
        assert!((B4.iter().sum::<f64>() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn exponential_decay() {
        let s = solve(
            |_, y, dy| dy[0] = -y[0],
            0.0,
            &[1.0],
            &[1.0, 2.0, 5.0],
            &tight(),
        )
        .unwrap();
        for (t, y) in s.t.iter().zip(&s.y) {
            assert!((y[0] - (-t).exp()).abs() < 1e-9, "t={t}: {}", y[0]);
        }
    }

    #[test]
    fn harmonic_oscillator_returns_to_its_start_after_a_period() {
        let s = solve(
            |_, y, dy| {
                dy[0] = y[1];
                dy[1] = -y[0];
            },
            0.0,
            &[1.0, 0.0],
            &[2.0 * PI],
            &tight(),
        )
        .unwrap();
        assert!(
            (s.y[0][0] - 1.0).abs() < 1e-8 && s.y[0][1].abs() < 1e-8,
            "{:?}",
            s.y[0]
        );
    }

    #[test]
    fn time_dependent_forcing() {
        // y' = 2t, y(0)=0 => y = t^2 (exactly representable by the method).
        let s = solve(
            |t, _, dy| dy[0] = 2.0 * t,
            0.0,
            &[0.0],
            &[3.0],
            &Options::default(),
        )
        .unwrap();
        assert!((s.y[0][0] - 9.0).abs() < 1e-8);
    }

    #[test]
    fn lands_exactly_on_each_output_time_and_includes_the_start() {
        let s = solve(
            |_, y, dy| dy[0] = -y[0],
            0.0,
            &[1.0],
            &[0.0, 0.5, 0.5, 1.0],
            &tight(),
        )
        .unwrap();
        assert_eq!(s.y.len(), 4);
        assert_eq!(s.y[0][0], 1.0);
        assert_eq!(s.y[1], s.y[2]);
    }

    #[test]
    fn finite_time_blow_up_is_an_error_not_a_number() {
        // y' = y^2, y(0)=1 blows up at t=1.
        let r = solve(
            |_, y, dy| dy[0] = y[0] * y[0],
            0.0,
            &[1.0],
            &[2.0],
            &Options::default(),
        );
        assert!(r.is_err(), "{r:?}");
    }

    #[test]
    fn bad_input_and_cancellation() {
        let f = |_: f64, y: &[f64], dy: &mut [f64]| dy[0] = -y[0];
        assert!(solve(f, 0.0, &[f64::NAN], &[1.0], &Options::default()).is_err());
        assert!(solve(f, 0.0, &[1.0], &[-1.0], &Options::default()).is_err());
        assert!(solve(
            f,
            0.0,
            &[1.0],
            &[1.0],
            &Options {
                rtol: 0.0,
                ..Options::default()
            }
        )
        .is_err());
        let stop = AtomicBool::new(true);
        let r = solve(
            f,
            0.0,
            &[1.0],
            &[1.0],
            &Options {
                cancel: Some(&stop),
                ..Options::default()
            },
        );
        assert!(matches!(r, Err(NumError::Cancelled)));
        let r = solve(
            f,
            0.0,
            &[1.0],
            &[1.0],
            &Options {
                max_steps: 2,
                rtol: 1e-12,
                ..Options::default()
            },
        );
        assert!(matches!(r, Err(NumError::NoConvergence { .. })));
    }

    #[test]
    fn tighter_tolerance_gives_a_smaller_error() {
        let run = |rtol: f64| {
            let o = Options {
                rtol,
                atol: rtol * 1e-3,
                ..Options::default()
            };
            let s = solve(|_, y, dy| dy[0] = -y[0], 0.0, &[1.0], &[3.0], &o).unwrap();
            (s.y[0][0] - (-3.0f64).exp()).abs()
        };
        assert!(run(1e-10) < run(1e-4) / 100.0);
    }
}
