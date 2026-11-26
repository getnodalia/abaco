// SPDX-License-Identifier: MIT OR Apache-2.0
//! Initial-value problems `M y' = f(t, y)` with dense output and event location.
//!
//! * [`Method::Dopri5`] is the Dormand–Prince 5(4) pair (Dormand & Prince 1980) with the
//!   fourth-order continuous extension of Hairer, Nørsett & Wanner (II.6) — for non-stiff
//!   problems with `M = I`.
//! * [`Method::Rosenbrock`] is the Rosenbrock (2,3) pair of Shampine & Reichelt (1997, "The
//!   MATLAB ODE suite", `ode23s`) with a numerical Jacobian — for stiff problems and, through a
//!   constant mass matrix, for index-1 differential-algebraic systems (a singular `M`).
//!
//! Output is either at requested times (read from the continuous extension, so the step size is
//! never forced by the output grid) or at every accepted step. Events `g(t, y)` are located by
//! bisection on the continuous extension; an event may stop the integration. Integration may run
//! backwards in time.

use crate::error::{NumError, Result};
use crate::lu::Lu;
use crate::matrix::Matrix;
use crate::ode::{A, B4, B5, C};
use std::sync::atomic::{AtomicBool, Ordering};

/// The integration method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Dormand–Prince 5(4); non-stiff, identity mass matrix.
    Dopri5,
    /// Rosenbrock (2,3); stiff and differential-algebraic.
    Rosenbrock,
}

/// Solver settings.
#[derive(Debug, Clone)]
pub struct Options<'a> {
    /// Relative tolerance per component.
    pub rtol: f64,
    /// Absolute tolerance per component.
    pub atol: f64,
    /// First step magnitude, or `None` to estimate it.
    pub initial_step: Option<f64>,
    /// Largest step magnitude.
    pub max_step: f64,
    /// Budget of accepted plus rejected steps.
    pub max_steps: usize,
    /// The method.
    pub method: Method,
    /// Constant mass matrix `M`, row-major `n × n`; `None` is the identity.
    pub mass: Option<Vec<f64>>,
    /// Set from another thread to stop with [`NumError::Cancelled`].
    pub cancel: Option<&'a AtomicBool>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            rtol: 1e-3,
            atol: 1e-6,
            initial_step: None,
            max_step: f64::INFINITY,
            max_steps: 1_000_000,
            method: Method::Dopri5,
            mass: None,
            cancel: None,
        }
    }
}

/// What to watch for: `g_i(t, y)` crossing zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSpec {
    /// Stop the integration at this event.
    pub terminal: bool,
    /// `1` rising only, `-1` falling only, `0` both.
    pub direction: i8,
}

/// A located event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventHit {
    /// Index of the event function.
    pub index: usize,
    /// Time of the crossing.
    pub t: f64,
    /// State at the crossing.
    pub y: Vec<f64>,
}

/// The integration result.
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    /// Times of the output rows.
    pub t: Vec<f64>,
    /// `y[k]` is the state at `t[k]`.
    pub y: Vec<Vec<f64>>,
    /// Events found, in time order.
    pub events: Vec<EventHit>,
    /// `true` when a terminal event stopped the integration before the end of the span.
    pub stopped: bool,
    /// Accepted steps.
    pub accepted: usize,
    /// Rejected steps.
    pub rejected: usize,
    /// Evaluations of `f`.
    pub evaluations: usize,
}

type Rhs<'f> = dyn FnMut(f64, &[f64], &mut [f64]) + 'f;
type EventFn<'f> = dyn FnMut(f64, &[f64]) -> Vec<f64> + 'f;

/// The continuous extension over one accepted step.
enum Dense {
    /// Hairer's five-vector form.
    Dopri([Vec<f64>; 5]),
    /// Shampine & Reichelt's `ntrp23s`.
    Rosen {
        y: Vec<f64>,
        k1: Vec<f64>,
        k2: Vec<f64>,
        h: f64,
    },
}

const RD: f64 = 1.0 / (2.0 + std::f64::consts::SQRT_2);

impl Dense {
    fn eval(&self, theta: f64) -> Vec<f64> {
        match self {
            Dense::Dopri(r) => {
                let s1 = 1.0 - theta;
                (0..r[0].len())
                    .map(|i| {
                        r[0][i]
                            + theta * (r[1][i] + s1 * (r[2][i] + theta * (r[3][i] + s1 * r[4][i])))
                    })
                    .collect()
            }
            Dense::Rosen { y, k1, k2, h } => {
                let c = 1.0 - 2.0 * RD;
                let a = theta * (1.0 - theta) / c;
                let b = theta * (theta - 2.0 * RD) / c;
                (0..y.len())
                    .map(|i| y[i] + h * (k1[i] * a + k2[i] * b))
                    .collect()
            }
        }
    }
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

/// Integrates over `[t0, tf]` (either order). `t_out`, when given, lists the output times
/// (monotone in the direction of integration, inside the span); otherwise every accepted step is
/// reported, with the first row at `t0`.
pub fn solve(
    f: &mut Rhs<'_>,
    mut events: Option<(&mut EventFn<'_>, &[EventSpec])>,
    (t0, tf): (f64, f64),
    y0: &[f64],
    t_out: Option<&[f64]>,
    opts: &Options<'_>,
) -> Result<Solution> {
    let n = y0.len();
    if !t0.is_finite() || !tf.is_finite() || y0.iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("the span or the initial condition"));
    }
    if n == 0 {
        return Err(NumError::InvalidInput("the state is empty".into()));
    }
    if !(opts.rtol > 0.0 && opts.rtol.is_finite() && opts.atol >= 0.0 && opts.atol.is_finite()) {
        return Err(NumError::InvalidInput(
            "rtol must be positive and atol non-negative".into(),
        ));
    }
    let mass = match &opts.mass {
        Some(m) => {
            if m.len() != n * n || m.iter().any(|v| !v.is_finite()) {
                return Err(NumError::DimensionMismatch(format!(
                    "the mass matrix must be {n} x {n} and finite"
                )));
            }
            if opts.method == Method::Dopri5 {
                return Err(NumError::InvalidInput(
                    "a mass matrix needs the stiff solver".into(),
                ));
            }
            Some(Matrix::new(n, n, m.clone())?)
        }
        None => None,
    };
    let sgn = if tf < t0 { -1.0 } else { 1.0 };
    let span = (tf - t0).abs();
    // Work in s = sgn·t so the integration always runs forward.
    let s0 = sgn * t0;
    let sf = sgn * tf;
    let outs: Option<Vec<f64>> = match t_out {
        Some(ts) => {
            let mut prev = s0;
            let mut v = Vec::with_capacity(ts.len());
            for &t in ts {
                let s = sgn * t;
                if !t.is_finite() || s < prev || s > sf + 1e-12 * span.max(1.0) {
                    return Err(NumError::InvalidInput(
                        "output times must be finite, monotone and inside the span".into(),
                    ));
                }
                prev = s;
                v.push(s.min(sf));
            }
            Some(v)
        }
        None => None,
    };

    let mut out = Solution {
        t: Vec::new(),
        y: Vec::new(),
        events: Vec::new(),
        stopped: false,
        accepted: 0,
        rejected: 0,
        evaluations: 0,
    };
    let mut fcall = |s: f64, y: &[f64], dy: &mut [f64], out: &mut Solution| -> Result<()> {
        f(sgn * s, y, dy);
        if sgn < 0.0 {
            dy.iter_mut().for_each(|v| *v = -*v);
        }
        out.evaluations += 1;
        if dy.iter().any(|v| !v.is_finite()) {
            return Err(NumError::NonFinite("derivative"));
        }
        Ok(())
    };

    let mut s = s0;
    let mut y = y0.to_vec();
    let mut next_out = 0usize;
    // Rows at the start.
    match &outs {
        Some(o) => {
            while next_out < o.len() && o[next_out] <= s0 {
                out.t.push(sgn * o[next_out]);
                out.y.push(y.clone());
                next_out += 1;
            }
        }
        None => {
            out.t.push(t0);
            out.y.push(y.clone());
        }
    }
    let mut g_prev: Vec<f64> = match events.as_mut() {
        Some((g, specs)) => {
            let v = g(t0, &y);
            if v.len() != specs.len() || v.iter().any(|x| !x.is_finite()) {
                return Err(NumError::InvalidInput(
                    "the event function returned an unexpected or non-finite result".into(),
                ));
            }
            v
        }
        None => Vec::new(),
    };
    if s0 == sf {
        return Ok(out);
    }

    let mut f0 = vec![0.0; n];
    fcall(s, &y, &mut f0, &mut out)?;
    let scale = |y: &[f64], i: usize| opts.atol + opts.rtol * y[i].abs();
    let hmax = opts.max_step.min(span);
    let mut h = match opts.initial_step {
        Some(h) if h > 0.0 && h.is_finite() => h,
        Some(_) => {
            return Err(NumError::InvalidInput(
                "the initial step must be positive".into(),
            ))
        }
        None => {
            let d0 = rms((0..n).map(|i| y[i] / scale(&y, i)));
            let d1 = rms((0..n).map(|i| f0[i] / scale(&y, i)));
            let h0 = if d0 < 1e-5 || d1 < 1e-5 {
                1e-6
            } else {
                0.01 * d0 / d1
            };
            let y1: Vec<f64> = (0..n).map(|i| y[i] + h0 * f0[i]).collect();
            let mut f1 = vec![0.0; n];
            fcall(s + h0, &y1, &mut f1, &mut out)?;
            let d2 = rms((0..n).map(|i| (f1[i] - f0[i]) / scale(&y, i))) / h0;
            let h1 = if d1.max(d2) <= 1e-15 {
                (h0 * 1e-3).max(1e-6)
            } else {
                (0.01 / d1.max(d2)).powf(0.2)
            };
            (100.0 * h0).min(h1)
        }
    }
    .min(hmax);
    h = h.max(1e-14 * span.max(1.0));

    let mut k: Vec<Vec<f64>> = vec![vec![0.0; n]; 7];
    k[0].copy_from_slice(&f0);
    let mut steps = 0usize;
    while s < sf {
        if let Some(c) = opts.cancel {
            if c.load(Ordering::Relaxed) {
                return Err(NumError::Cancelled);
            }
        }
        steps += 1;
        if steps > opts.max_steps {
            return Err(NumError::NoConvergence {
                iterations: steps,
                residual: sf - s,
            });
        }
        let mut h_try = h.min(sf - s);
        if sf - s - h_try < 1e-3 * h_try {
            h_try = sf - s;
        }
        if h_try < 16.0 * f64::EPSILON * s.abs().max(1e-300) {
            return Err(NumError::StepTooSmall { t: sgn * s });
        }
        // One attempt: the new state, the scaled error and the continuous extension.
        let attempt = match opts.method {
            Method::Dopri5 => dopri_step(&mut fcall, &mut out, s, &y, &mut k, h_try, opts),
            Method::Rosenbrock => {
                rosen_step(&mut fcall, &mut out, s, &y, &f0, mass.as_ref(), h_try, opts)
            }
        };
        let (ynew, err, dense, fnew) = match attempt {
            Ok(a) => a,
            Err(NumError::Singular { .. }) => {
                out.rejected += 1;
                h = h_try * 0.25;
                continue;
            }
            Err(e) => return Err(e),
        };
        if !err.is_finite() {
            return Err(NumError::NonFinite("error estimate"));
        }
        let order_exp = match opts.method {
            Method::Dopri5 => 0.2,
            Method::Rosenbrock => 1.0 / 3.0,
        };
        if err > 1.0 {
            out.rejected += 1;
            h = h_try * (0.9 * err.powf(-order_exp)).clamp(0.1, 0.9);
            continue;
        }
        out.accepted += 1;
        let s1 = if h_try == sf - s { sf } else { s + h_try };
        let step_h = s1 - s;

        // Events on this step.
        let mut hits: Vec<(f64, usize)> = Vec::new();
        let mut g_new = Vec::new();
        if let Some((g, specs)) = events.as_mut() {
            g_new = g(sgn * s1, &ynew);
            if g_new.len() != specs.len() || g_new.iter().any(|x| !x.is_finite()) {
                return Err(NumError::InvalidInput(
                    "the event function returned an unexpected or non-finite result".into(),
                ));
            }
            for (i, spec) in specs.iter().enumerate() {
                let (a, b) = (g_prev[i], g_new[i]);
                let rising = a < 0.0 && b >= 0.0;
                let falling = a > 0.0 && b <= 0.0;
                let ok = match spec.direction {
                    1 => rising,
                    -1 => falling,
                    _ => rising || falling,
                };
                if !ok {
                    continue;
                }
                // Bisection on theta in (0, 1].
                let (mut lo, mut hi) = (0.0f64, 1.0f64);
                for _ in 0..80 {
                    let mid = 0.5 * (lo + hi);
                    let ym = dense.eval(mid);
                    let gm = g(sgn * (s + mid * step_h), &ym);
                    let gi = gm.get(i).copied().unwrap_or(f64::NAN);
                    if (gi < 0.0) == (a < 0.0) && gi != 0.0 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                    if hi - lo < 1e-14 {
                        break;
                    }
                }
                hits.push((hi, i));
            }
            hits.sort_by(|p, q| p.0.total_cmp(&q.0));
        }
        let mut stop_at: Option<f64> = None;
        for &(theta, i) in &hits {
            let st = s + theta * step_h;
            let hit = EventHit {
                index: i,
                t: sgn * st,
                y: if theta >= 1.0 {
                    ynew.clone()
                } else {
                    dense.eval(theta)
                },
            };
            out.events.push(hit);
            if events
                .as_ref()
                .and_then(|(_, sp)| sp.get(i))
                .is_some_and(|sp| sp.terminal)
            {
                stop_at = Some(theta);
                break;
            }
        }
        let limit = stop_at.map_or(s1, |th| s + th * step_h);

        // Output rows inside (s, limit].
        match &outs {
            Some(o) => {
                while next_out < o.len() && o[next_out] <= limit {
                    let theta = ((o[next_out] - s) / step_h).clamp(0.0, 1.0);
                    out.t.push(sgn * o[next_out]);
                    out.y.push(if theta >= 1.0 {
                        ynew.clone()
                    } else {
                        dense.eval(theta)
                    });
                    next_out += 1;
                }
            }
            None => {
                let yl = match stop_at {
                    Some(th) => dense.eval(th),
                    None => ynew.clone(),
                };
                out.t.push(sgn * limit);
                out.y.push(yl);
            }
        }
        if let Some(th) = stop_at {
            out.stopped = true;
            if outs.is_some() {
                // Close the record at the event so the last row is where it stopped.
                if out.t.last().copied() != Some(sgn * limit) {
                    out.t.push(sgn * limit);
                    out.y.push(dense.eval(th));
                }
            }
            return Ok(out);
        }
        g_prev = g_new;
        s = s1;
        y = ynew;
        match opts.method {
            Method::Dopri5 => {
                let last = k[6].clone();
                k[0] = last;
                f0.copy_from_slice(&k[0]);
            }
            Method::Rosenbrock => f0 = fnew,
        }
        let factor = if err == 0.0 {
            10.0
        } else {
            (0.9 * err.powf(-order_exp)).clamp(0.2, 10.0)
        };
        h = (h_try * factor).min(hmax);
    }
    Ok(out)
}

type Attempt = (Vec<f64>, f64, Dense, Vec<f64>);

fn dopri_step(
    fcall: &mut impl FnMut(f64, &[f64], &mut [f64], &mut Solution) -> Result<()>,
    out: &mut Solution,
    s: f64,
    y: &[f64],
    k: &mut [Vec<f64>],
    h: f64,
    opts: &Options<'_>,
) -> Result<Attempt> {
    let n = y.len();
    let mut tmp = vec![0.0; n];
    for st in 1..7 {
        for i in 0..n {
            let mut acc = 0.0;
            for (j, kj) in k.iter().enumerate().take(st) {
                acc += A[st][j] * kj[i];
            }
            tmp[i] = y[i] + h * acc;
        }
        let (_, tail) = k.split_at_mut(st);
        fcall(s + C[st] * h, &tmp, &mut tail[0], out)?;
    }
    let ynew: Vec<f64> = (0..n)
        .map(|i| {
            let acc: f64 = (0..6).map(|st| B5[st] * k[st][i]).sum();
            y[i] + h * acc
        })
        .collect();
    let err = rms((0..n).map(|i| {
        let e: f64 = (0..7).map(|st| (B5[st] - B4[st]) * k[st][i]).sum::<f64>() * h;
        e / (opts.atol + opts.rtol * y[i].abs().max(ynew[i].abs()))
    }));
    // Hairer's dense output (dopri5.f, contd5).
    const D1: f64 = -12715105075.0 / 11282082432.0;
    const D3: f64 = 87487479700.0 / 32700410799.0;
    const D4: f64 = -10690763975.0 / 1880347072.0;
    const D5: f64 = 701980252875.0 / 199316789632.0;
    const D6: f64 = -1453857185.0 / 822651844.0;
    const D7: f64 = 69997945.0 / 29380423.0;
    let mut r: [Vec<f64>; 5] = Default::default();
    r[0] = y.to_vec();
    r[1] = (0..n).map(|i| ynew[i] - y[i]).collect();
    r[2] = (0..n).map(|i| h * k[0][i] - r[1][i]).collect();
    r[3] = (0..n).map(|i| r[1][i] - h * k[6][i] - r[2][i]).collect();
    r[4] = (0..n)
        .map(|i| {
            h * (D1 * k[0][i]
                + D3 * k[2][i]
                + D4 * k[3][i]
                + D5 * k[4][i]
                + D6 * k[5][i]
                + D7 * k[6][i])
        })
        .collect();
    let fnew = k[6].clone();
    Ok((ynew, err, Dense::Dopri(r), fnew))
}

#[allow(clippy::too_many_arguments)]
fn rosen_step(
    fcall: &mut impl FnMut(f64, &[f64], &mut [f64], &mut Solution) -> Result<()>,
    out: &mut Solution,
    s: f64,
    y: &[f64],
    f0: &[f64],
    mass: Option<&Matrix<f64>>,
    h: f64,
    opts: &Options<'_>,
) -> Result<Attempt> {
    let n = y.len();
    let e32 = 6.0 + std::f64::consts::SQRT_2;
    // Jacobian and time derivative by forward differences.
    let mut jac = vec![0.0; n * n];
    let mut yp = y.to_vec();
    let mut fp = vec![0.0; n];
    for j in 0..n {
        let d = 1.5e-8 * y[j].abs().max(1e-3);
        yp[j] = y[j] + d;
        fcall(s, &yp, &mut fp, out)?;
        yp[j] = y[j];
        for i in 0..n {
            jac[i * n + j] = (fp[i] - f0[i]) / d;
        }
    }
    let dt = 1.5e-8 * s.abs().max(1.0);
    fcall(s + dt, y, &mut fp, out)?;
    let tt: Vec<f64> = (0..n).map(|i| (fp[i] - f0[i]) / dt).collect();
    let mut w = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            let m = mass.and_then(|m| m.get(i, j)).unwrap_or(f64::from(i == j));
            w[i * n + j] = m - h * RD * jac[i * n + j];
        }
    }
    let lu = Lu::factor(&Matrix::new(n, n, w)?)?;
    let mulm = |v: &[f64]| -> Vec<f64> {
        match mass {
            Some(m) => m.mul_vec(v).unwrap_or_else(|_| v.to_vec()),
            None => v.to_vec(),
        }
    };
    let rhs1: Vec<f64> = (0..n).map(|i| f0[i] + h * RD * tt[i]).collect();
    let k1 = lu.solve(&rhs1)?;
    let y1: Vec<f64> = (0..n).map(|i| y[i] + 0.5 * h * k1[i]).collect();
    let mut f1 = vec![0.0; n];
    fcall(s + 0.5 * h, &y1, &mut f1, out)?;
    let mk1 = mulm(&k1);
    let rhs2: Vec<f64> = (0..n).map(|i| f1[i] - mk1[i]).collect();
    let d2 = lu.solve(&rhs2)?;
    let k2: Vec<f64> = (0..n).map(|i| k1[i] + d2[i]).collect();
    let ynew: Vec<f64> = (0..n).map(|i| y[i] + h * k2[i]).collect();
    let mut f2 = vec![0.0; n];
    fcall(s + h, &ynew, &mut f2, out)?;
    let mk2 = mulm(&k2);
    let rhs3: Vec<f64> = (0..n)
        .map(|i| f2[i] - e32 * (mk2[i] - f1[i]) - 2.0 * (mk1[i] - f0[i]) + h * RD * tt[i])
        .collect();
    let k3 = lu.solve(&rhs3)?;
    let err = rms((0..n).map(|i| {
        let e = h / 6.0 * (k1[i] - 2.0 * k2[i] + k3[i]);
        e / (opts.atol + opts.rtol * y[i].abs().max(ynew[i].abs()))
    }));
    let dense = Dense::Rosen {
        y: y.to_vec(),
        k1,
        k2,
        h,
    };
    Ok((ynew, err, dense, f2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        method: Method,
        f: impl Fn(f64, &[f64], &mut [f64]),
        span: (f64, f64),
        y0: &[f64],
        t_out: Option<&[f64]>,
        rtol: f64,
    ) -> Solution {
        let opts = Options {
            rtol,
            atol: rtol * 1e-3,
            method,
            ..Default::default()
        };
        let mut g = |t: f64, y: &[f64], dy: &mut [f64]| f(t, y, dy);
        solve(&mut g, None, span, y0, t_out, &opts).unwrap()
    }

    #[test]
    fn dopri_dense_output_is_accurate_between_steps() {
        let ts: Vec<f64> = (0..=40).map(|i| i as f64 * 0.1).collect();
        let sol = run(
            Method::Dopri5,
            |_, y, dy| dy[0] = -y[0],
            (0.0, 4.0),
            &[1.0],
            Some(&ts),
            1e-8,
        );
        for (t, y) in sol.t.iter().zip(&sol.y) {
            assert!((y[0] - (-t).exp()).abs() < 2e-8, "t={t}");
        }
    }

    #[test]
    fn harmonic_oscillator_returns_to_its_start() {
        let sol = run(
            Method::Dopri5,
            |_, y, dy| {
                dy[0] = y[1];
                dy[1] = -y[0];
            },
            (0.0, 2.0 * std::f64::consts::PI),
            &[1.0, 0.0],
            None,
            1e-9,
        );
        let last = sol.y.last().unwrap();
        assert!((last[0] - 1.0).abs() < 1e-7 && last[1].abs() < 1e-7);
        assert!(sol.t.len() > 10 && sol.t[0] == 0.0);
    }

    #[test]
    fn backward_integration_matches_the_closed_form() {
        let sol = run(
            Method::Dopri5,
            |_, y, dy| dy[0] = y[0],
            (1.0, 0.0),
            &[std::f64::consts::E],
            Some(&[1.0, 0.5, 0.0]),
            1e-9,
        );
        assert!((sol.y[1][0] - 0.5f64.exp()).abs() < 1e-7);
        assert!((sol.y[2][0] - 1.0).abs() < 1e-7);
        assert_eq!(sol.t, vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn rosenbrock_handles_a_stiff_linear_problem() {
        // y' = -1000 (y - cos t): stiff; the explicit pair would need thousands of steps.
        let f = |t: f64, y: &[f64], dy: &mut [f64]| dy[0] = -1000.0 * (y[0] - t.cos());
        let stiff = run(Method::Rosenbrock, f, (0.0, 2.0), &[0.0], None, 1e-4);
        let last = stiff.y.last().unwrap()[0];
        // Slow manifold: y ≈ cos t + sin t / 1000.
        let want = 2.0f64.cos() + 2.0f64.sin() / 1000.0;
        assert!((last - want).abs() < 2e-3, "{last} vs {want}");
        let explicit = run(Method::Dopri5, f, (0.0, 2.0), &[0.0], None, 1e-4);
        assert!(
            stiff.accepted * 3 < explicit.accepted,
            "{} vs {}",
            stiff.accepted,
            explicit.accepted
        );
    }

    #[test]
    fn van_der_pol_stiff_stays_on_its_limit_cycle() {
        let mu = 1000.0;
        let f = |_: f64, y: &[f64], dy: &mut [f64]| {
            dy[0] = y[1];
            dy[1] = mu * (1.0 - y[0] * y[0]) * y[1] - y[0];
        };
        let sol = run(
            Method::Rosenbrock,
            f,
            (0.0, 3000.0),
            &[2.0, 0.0],
            None,
            1e-4,
        );
        let end = sol.y.last().unwrap();
        // The relaxation oscillation keeps |y1| between roughly 1 and 2.1.
        assert!(end[0].abs() > 0.5 && end[0].abs() < 2.2);
    }

    #[test]
    fn robertson_conserves_mass_and_reaches_its_known_state() {
        let f = |_: f64, y: &[f64], dy: &mut [f64]| {
            dy[0] = -0.04 * y[0] + 1e4 * y[1] * y[2];
            dy[1] = 0.04 * y[0] - 1e4 * y[1] * y[2] - 3e7 * y[1] * y[1];
            dy[2] = 3e7 * y[1] * y[1];
        };
        let sol = run(
            Method::Rosenbrock,
            f,
            (0.0, 40.0),
            &[1.0, 0.0, 0.0],
            Some(&[40.0]),
            1e-6,
        );
        let y = &sol.y[0];
        // Published reference values at t = 40 (Hairer & Wanner, Table 1.1 of IV.1).
        assert!((y[0] - 0.7158).abs() < 2e-4, "{y:?}");
        assert!((y[1] - 9.185e-6).abs() < 3e-7, "{y:?}");
        assert!((y.iter().sum::<f64>() - 1.0).abs() < 1e-8);
    }

    #[test]
    fn a_singular_mass_matrix_solves_an_index_1_dae() {
        // y1' = -y1 + y2 ; 0 = y2 - 2 y1  =>  y1' = y1, y2 = 2 y1.
        let opts = Options {
            rtol: 1e-6,
            atol: 1e-9,
            method: Method::Rosenbrock,
            mass: Some(vec![1.0, 0.0, 0.0, 0.0]),
            ..Default::default()
        };
        let mut f = |_: f64, y: &[f64], dy: &mut [f64]| {
            dy[0] = -y[0] + y[1];
            dy[1] = y[1] - 2.0 * y[0];
        };
        let sol = solve(&mut f, None, (0.0, 1.0), &[1.0, 2.0], Some(&[1.0]), &opts).unwrap();
        let y = &sol.y[0];
        assert!((y[0] - 1f64.exp()).abs() < 5e-3, "{y:?}");
        assert!((y[1] - 2.0 * y[0]).abs() < 1e-6, "{y:?}");
    }

    #[test]
    fn a_terminal_event_stops_a_falling_ball_at_the_ground() {
        // y'' = -9.81, released from 10 m: it lands at t = sqrt(2·10/9.81).
        let opts = Options {
            rtol: 1e-9,
            atol: 1e-11,
            ..Default::default()
        };
        let mut f = |_: f64, y: &[f64], dy: &mut [f64]| {
            dy[0] = y[1];
            dy[1] = -9.81;
        };
        let mut g = |_: f64, y: &[f64]| vec![y[0]];
        let specs = [EventSpec {
            terminal: true,
            direction: -1,
        }];
        let sol = solve(
            &mut f,
            Some((&mut g, &specs)),
            (0.0, 10.0),
            &[10.0, 0.0],
            None,
            &opts,
        )
        .unwrap();
        let want = (20.0f64 / 9.81).sqrt();
        assert!(sol.stopped);
        assert_eq!(sol.events.len(), 1);
        assert!((sol.events[0].t - want).abs() < 1e-9, "{}", sol.events[0].t);
        assert!((sol.t.last().unwrap() - want).abs() < 1e-9);
        assert!(sol.events[0].y[0].abs() < 1e-8);
    }

    #[test]
    fn non_terminal_events_count_every_crossing() {
        // y = sin t crosses zero at k·pi.
        let opts = Options {
            rtol: 1e-9,
            atol: 1e-11,
            ..Default::default()
        };
        let mut f = |_: f64, y: &[f64], dy: &mut [f64]| {
            dy[0] = y[1];
            dy[1] = -y[0];
        };
        let mut g = |_: f64, y: &[f64]| vec![y[0]];
        let specs = [EventSpec {
            terminal: false,
            direction: 0,
        }];
        let sol = solve(
            &mut f,
            Some((&mut g, &specs)),
            (0.0, 10.0),
            &[0.0, 1.0],
            None,
            &opts,
        )
        .unwrap();
        assert!(!sol.stopped);
        let ts: Vec<f64> = sol.events.iter().map(|e| e.t).collect();
        assert_eq!(ts.len(), 3, "{ts:?}");
        for (k, t) in ts.iter().enumerate() {
            assert!((t - (k as f64 + 1.0) * std::f64::consts::PI).abs() < 1e-7);
        }
    }

    #[test]
    fn hostile_input_is_refused() {
        let mut f = |_: f64, _: &[f64], dy: &mut [f64]| dy[0] = f64::NAN;
        let o = Options::default();
        assert!(solve(&mut f, None, (0.0, 1.0), &[1.0], None, &o).is_err());
        let mut g = |_: f64, _: &[f64], dy: &mut [f64]| dy[0] = 0.0;
        assert!(solve(&mut g, None, (0.0, f64::INFINITY), &[1.0], None, &o).is_err());
        assert!(solve(&mut g, None, (0.0, 1.0), &[], None, &o).is_err());
        assert!(solve(&mut g, None, (0.0, 1.0), &[1.0], Some(&[2.0]), &o).is_err());
        let bad = Options {
            mass: Some(vec![1.0, 2.0]),
            method: Method::Rosenbrock,
            ..Default::default()
        };
        assert!(solve(&mut g, None, (0.0, 1.0), &[1.0], None, &bad).is_err());
    }
}
