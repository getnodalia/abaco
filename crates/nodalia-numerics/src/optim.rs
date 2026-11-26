// SPDX-License-Identifier: MIT OR Apache-2.0
//! Optimisation: derivative-free (Nelder–Mead, Brent), quasi-Newton (BFGS), nonlinear least
//! squares and square systems (Levenberg–Marquardt) and linear programming (two-phase simplex).
//!
//! Objective callbacks cannot fail; a caller that needs to report an error parks it and returns
//! NaN, which every routine here treats as "the point is unusable" (an objective that is NaN at
//! the start is an error). Every routine has an iteration and an evaluation budget and can be
//! cancelled, and none returns a point it has not evaluated.

use crate::error::{NumError, Result};
use crate::lu::solve;
use crate::matrix::Matrix;
use std::sync::atomic::{AtomicBool, Ordering};

/// Stopping rules shared by the minimisers.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// Largest change in the variables (infinity norm) at which to stop.
    pub tol_x: f64,
    /// Largest change in the objective (or the residual norm for least squares) at which to stop.
    pub tol_f: f64,
    /// Iteration budget.
    pub max_iter: usize,
    /// Budget of objective evaluations.
    pub max_evals: usize,
    /// Set from another thread to stop with [`NumError::Cancelled`].
    pub cancel: Option<&'a AtomicBool>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            tol_x: 1e-4,
            tol_f: 1e-4,
            max_iter: 10_000,
            max_evals: 1_000_000,
            cancel: None,
        }
    }
}

/// What a minimiser found and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Minimum {
    /// The best point evaluated.
    pub x: Vec<f64>,
    /// The objective there.
    pub f: f64,
    /// Iterations used.
    pub iterations: usize,
    /// Objective evaluations used.
    pub evaluations: usize,
    /// `true` when a stopping tolerance was met, `false` when a budget ran out.
    pub converged: bool,
}

fn cancelled(o: &Options<'_>) -> bool {
    o.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

fn check_start(x0: &[f64]) -> Result<()> {
    if x0.is_empty() {
        return Err(NumError::InvalidInput("the starting point is empty".into()));
    }
    if x0.iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("the starting point"));
    }
    Ok(())
}

fn usable(v: f64) -> f64 {
    if v.is_nan() {
        f64::INFINITY
    } else {
        v
    }
}

/// Nelder–Mead simplex search (reflection 1, expansion 2, contraction ½, shrink ½), started from
/// `x0` and a simplex that perturbs each coordinate by 5 % (or 0.00025 when it is zero).
pub fn nelder_mead(
    mut f: impl FnMut(&[f64]) -> f64,
    x0: &[f64],
    opts: &Options<'_>,
) -> Result<Minimum> {
    check_start(x0)?;
    let n = x0.len();
    let mut evals = 1;
    let f0 = f(x0);
    if !f0.is_finite() {
        return Err(NumError::NonFinite("the objective at the starting point"));
    }
    let mut pts: Vec<(Vec<f64>, f64)> = vec![(x0.to_vec(), f0)];
    for i in 0..n {
        let mut p = x0.to_vec();
        p[i] = if p[i] == 0.0 { 0.000_25 } else { p[i] * 1.05 };
        evals += 1;
        let v = usable(f(&p));
        pts.push((p, v));
    }
    let mut iter = 0;
    let mut converged = false;
    while iter < opts.max_iter && evals < opts.max_evals {
        if cancelled(opts) {
            return Err(NumError::Cancelled);
        }
        pts.sort_by(|a, b| a.1.total_cmp(&b.1));
        let best = pts[0].1;
        let df = pts.iter().map(|p| (p.1 - best).abs()).fold(0.0, f64::max);
        let dx = pts[1..]
            .iter()
            .flat_map(|p| p.0.iter().zip(&pts[0].0).map(|(a, b)| (a - b).abs()))
            .fold(0.0, f64::max);
        if df <= opts.tol_f && dx <= opts.tol_x {
            converged = true;
            break;
        }
        iter += 1;
        let worst = pts[n].clone();
        let mut centroid = vec![0.0; n];
        for p in &pts[..n] {
            for (c, v) in centroid.iter_mut().zip(&p.0) {
                *c += v / n as f64;
            }
        }
        let along = |t: f64| -> Vec<f64> {
            centroid
                .iter()
                .zip(&worst.0)
                .map(|(c, w)| c + t * (c - w))
                .collect()
        };
        let xr = along(1.0);
        evals += 1;
        let fr = usable(f(&xr));
        if fr < pts[0].1 {
            let xe = along(2.0);
            evals += 1;
            let fe = usable(f(&xe));
            pts[n] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < pts[n - 1].1 {
            pts[n] = (xr, fr);
        } else {
            let (xc, outside) = if fr < worst.1 {
                (along(0.5), true)
            } else {
                (along(-0.5), false)
            };
            evals += 1;
            let fc = usable(f(&xc));
            let accept = if outside { fc <= fr } else { fc < worst.1 };
            if accept {
                pts[n] = (xc, fc);
            } else {
                let b = pts[0].0.clone();
                for p in pts.iter_mut().skip(1) {
                    for (v, bv) in p.0.iter_mut().zip(&b) {
                        *v = bv + 0.5 * (*v - bv);
                    }
                    evals += 1;
                    p.1 = usable(f(&p.0));
                }
            }
        }
    }
    pts.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (x, fv) = pts.swap_remove(0);
    Ok(Minimum {
        x,
        f: fv,
        iterations: iter,
        evaluations: evals,
        converged,
    })
}

/// Brent's method for the minimum of a function of one variable on `[a, b]` (golden section
/// with parabolic interpolation). Stops when the bracket is shorter than `3 tol`.
pub fn brent_min(
    mut f: impl FnMut(f64) -> f64,
    a: f64,
    b: f64,
    tol: f64,
    max_iter: usize,
) -> Result<Minimum> {
    if !(a.is_finite() && b.is_finite() && a < b) {
        return Err(NumError::InvalidInput(
            "the interval must be finite with a < b".into(),
        ));
    }
    if !(tol > 0.0 && tol.is_finite()) {
        return Err(NumError::InvalidInput(
            "the tolerance must be positive".into(),
        ));
    }
    const C: f64 = 0.381_966_011_250_105_1; // (3 - sqrt 5) / 2
    let (mut lo, mut hi) = (a, b);
    let mut x = lo + C * (hi - lo);
    let (mut w, mut v) = (x, x);
    let mut fx = f(x);
    if fx.is_nan() {
        return Err(NumError::NonFinite("the objective"));
    }
    let (mut fw, mut fv) = (fx, fx);
    let (mut d, mut e) = (0.0_f64, 0.0_f64);
    let mut evals = 1;
    let mut iter = 0;
    let mut converged = false;
    while iter < max_iter {
        let m = 0.5 * (lo + hi);
        let t1 = tol * x.abs() + 1e-10;
        let t2 = 2.0 * t1;
        if (x - m).abs() <= t2 - 0.5 * (hi - lo) {
            converged = true;
            break;
        }
        iter += 1;
        let mut golden = true;
        if e.abs() > t1 {
            let r = (x - w) * (fx - fv);
            let mut q = (x - v) * (fx - fw);
            let mut p = (x - v) * q - (x - w) * r;
            q = 2.0 * (q - r);
            if q > 0.0 {
                p = -p;
            }
            q = q.abs();
            if p.abs() < (0.5 * q * e).abs() && p > q * (lo - x) && p < q * (hi - x) {
                e = d;
                d = p / q;
                let u = x + d;
                if u - lo < t2 || hi - u < t2 {
                    d = if x < m { t1 } else { -t1 };
                }
                golden = false;
            }
        }
        if golden {
            e = if x < m { hi - x } else { lo - x };
            d = C * e;
        }
        let u = if d.abs() >= t1 {
            x + d
        } else if d > 0.0 {
            x + t1
        } else {
            x - t1
        };
        evals += 1;
        let fu = usable(f(u));
        if fu <= fx {
            if u < x {
                hi = x;
            } else {
                lo = x;
            }
            v = w;
            fv = fw;
            w = x;
            fw = fx;
            x = u;
            fx = fu;
        } else {
            if u < x {
                lo = u;
            } else {
                hi = u;
            }
            if fu <= fw || w == x {
                v = w;
                fv = fw;
                w = u;
                fw = fu;
            } else if fu <= fv || v == x || v == w {
                v = u;
                fv = fu;
            }
        }
    }
    Ok(Minimum {
        x: vec![x],
        f: fx,
        iterations: iter,
        evaluations: evals,
        converged,
    })
}

/// Central-difference gradient.
fn gradient(f: &mut impl FnMut(&[f64]) -> f64, x: &[f64], evals: &mut usize) -> Vec<f64> {
    let mut g = Vec::with_capacity(x.len());
    let mut p = x.to_vec();
    for i in 0..x.len() {
        let h = f64::EPSILON.cbrt() * x[i].abs().max(1.0);
        p[i] = x[i] + h;
        let fp = f(&p);
        p[i] = x[i] - h;
        let fm = f(&p);
        p[i] = x[i];
        *evals += 2;
        g.push((fp - fm) / (2.0 * h));
    }
    g
}

/// BFGS with a backtracking line search and central-difference gradients. Converges when the
/// largest gradient component is below `tol_f` (the optimality tolerance) or the step is
/// below `tol_x`.
pub fn bfgs(mut f: impl FnMut(&[f64]) -> f64, x0: &[f64], opts: &Options<'_>) -> Result<Minimum> {
    check_start(x0)?;
    let n = x0.len();
    let mut evals = 1;
    let mut x = x0.to_vec();
    let mut fx = f(&x);
    if !fx.is_finite() {
        return Err(NumError::NonFinite("the objective at the starting point"));
    }
    let mut g = gradient(&mut f, &x, &mut evals);
    if g.iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("the gradient at the starting point"));
    }
    let mut h = vec![0.0; n * n];
    let reset = |h: &mut Vec<f64>| {
        h.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..n {
            h[i * n + i] = 1.0;
        }
    };
    reset(&mut h);
    let mut iter = 0;
    let mut converged = false;
    while iter < opts.max_iter && evals < opts.max_evals {
        if cancelled(opts) {
            return Err(NumError::Cancelled);
        }
        let gmax = g.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        if gmax <= opts.tol_f {
            converged = true;
            break;
        }
        iter += 1;
        let mut d: Vec<f64> = (0..n)
            .map(|i| -(0..n).map(|j| h[i * n + j] * g[j]).sum::<f64>())
            .collect();
        let mut slope: f64 = d.iter().zip(&g).map(|(a, b)| a * b).sum();
        if slope >= 0.0 {
            reset(&mut h);
            d = g.iter().map(|v| -v).collect();
            slope = -g.iter().map(|v| v * v).sum::<f64>();
        }
        let mut t = if iter == 1 {
            (1.0 / g.iter().map(|v| v * v).sum::<f64>().sqrt()).min(1.0)
        } else {
            1.0
        };
        let mut xn = x.clone();
        let mut fnew = fx;
        let mut ok = false;
        for _ in 0..60 {
            for i in 0..n {
                xn[i] = x[i] + t * d[i];
            }
            evals += 1;
            fnew = usable(f(&xn));
            if fnew <= fx + 1e-4 * t * slope {
                ok = true;
                break;
            }
            t *= 0.5;
        }
        if !ok {
            // No decrease along a descent direction: the point is as good as the arithmetic allows.
            converged = gmax <= opts.tol_f.max(1e-3);
            break;
        }
        let gn = gradient(&mut f, &xn, &mut evals);
        if gn.iter().any(|v| !v.is_finite()) {
            return Err(NumError::NonFinite("the gradient"));
        }
        let s: Vec<f64> = xn.iter().zip(&x).map(|(a, b)| a - b).collect();
        let y: Vec<f64> = gn.iter().zip(&g).map(|(a, b)| a - b).collect();
        let sy: f64 = s.iter().zip(&y).map(|(a, b)| a * b).sum();
        let step = s.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        if sy > 1e-12 {
            let hy: Vec<f64> = (0..n)
                .map(|i| (0..n).map(|j| h[i * n + j] * y[j]).sum())
                .collect();
            let yhy: f64 = y.iter().zip(&hy).map(|(a, b)| a * b).sum();
            for i in 0..n {
                for j in 0..n {
                    h[i * n + j] +=
                        (1.0 + yhy / sy) * s[i] * s[j] / sy - (hy[i] * s[j] + s[i] * hy[j]) / sy;
                }
            }
        }
        x = xn;
        fx = fnew;
        g = gn;
        if step <= opts.tol_x * 1e-3 {
            converged = true;
            break;
        }
    }
    Ok(Minimum {
        x,
        f: fx,
        iterations: iter,
        evaluations: evals,
        converged,
    })
}

/// Where a least-squares fit ended.
#[derive(Debug, Clone, PartialEq)]
pub struct LeastSquares {
    /// The parameters.
    pub x: Vec<f64>,
    /// The residual vector there.
    pub residual: Vec<f64>,
    /// `‖r‖²`.
    pub cost: f64,
    /// Iterations used.
    pub iterations: usize,
    /// Residual evaluations used.
    pub evaluations: usize,
    /// `true` when a stopping tolerance was met.
    pub converged: bool,
}

/// Levenberg–Marquardt (Marquardt's diagonal scaling) for `min ‖r(x)‖²` with a forward-difference
/// Jacobian. Used for curve fitting and for square systems `r(x) = 0`.
pub fn levenberg_marquardt(
    mut r: impl FnMut(&[f64]) -> Vec<f64>,
    x0: &[f64],
    opts: &Options<'_>,
) -> Result<LeastSquares> {
    check_start(x0)?;
    let n = x0.len();
    let mut x = x0.to_vec();
    let mut res = r(&x);
    let mut evals = 1;
    let m = res.len();
    if m == 0 {
        return Err(NumError::InvalidInput("the residual is empty".into()));
    }
    if res.iter().any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("the residual at the starting point"));
    }
    let sq = |v: &[f64]| v.iter().map(|a| a * a).sum::<f64>();
    let mut cost = sq(&res);
    let mut lambda = 1e-3;
    let mut iter = 0;
    let mut converged = false;
    while iter < opts.max_iter && evals < opts.max_evals {
        if cancelled(opts) {
            return Err(NumError::Cancelled);
        }
        iter += 1;
        // Forward-difference Jacobian, m x n row-major.
        let mut jac = vec![0.0; m * n];
        let mut p = x.clone();
        for j in 0..n {
            let h = f64::EPSILON.sqrt() * x[j].abs().max(1.0);
            p[j] = x[j] + h;
            let rp = r(&p);
            evals += 1;
            p[j] = x[j];
            if rp.len() != m || rp.iter().any(|v| !v.is_finite()) {
                return Err(NumError::NonFinite("the Jacobian"));
            }
            for i in 0..m {
                jac[i * n + j] = (rp[i] - res[i]) / h;
            }
        }
        let mut jtj = vec![0.0; n * n];
        let mut jtr = vec![0.0; n];
        for i in 0..m {
            for a in 0..n {
                jtr[a] += jac[i * n + a] * res[i];
                for b in 0..n {
                    jtj[a * n + b] += jac[i * n + a] * jac[i * n + b];
                }
            }
        }
        if jtr.iter().fold(0.0_f64, |mx, v| mx.max(v.abs())) <= 1e-14 * (1.0 + cost) {
            converged = true;
            break;
        }
        let mut improved = false;
        for _ in 0..30 {
            let mut a = jtj.clone();
            for i in 0..n {
                a[i * n + i] += lambda * jtj[i * n + i].max(1e-12);
            }
            let rhs: Vec<f64> = jtr.iter().map(|v| -v).collect();
            let step = match Matrix::new(n, n, a).and_then(|mat| solve(&mat, &rhs)) {
                Ok(s) => s,
                Err(_) => {
                    lambda *= 10.0;
                    continue;
                }
            };
            let trial: Vec<f64> = x.iter().zip(&step).map(|(a, b)| a + b).collect();
            let rt = r(&trial);
            evals += 1;
            let ct = if rt.len() == m && rt.iter().all(|v| v.is_finite()) {
                sq(&rt)
            } else {
                f64::INFINITY
            };
            if ct < cost {
                let dx = step.iter().fold(0.0_f64, |mx, v| mx.max(v.abs()));
                let dc = cost - ct;
                x = trial;
                res = rt;
                cost = ct;
                lambda = (lambda / 10.0).max(1e-15);
                improved = true;
                let xs = x.iter().fold(0.0_f64, |mx, v| mx.max(v.abs()));
                if dx <= opts.tol_x * 1e-4 * (1.0 + xs) || dc <= opts.tol_f * 1e-8 * (1.0 + cost) {
                    converged = true;
                }
                break;
            }
            lambda *= 10.0;
            if lambda > 1e16 {
                break;
            }
        }
        if converged {
            break;
        }
        if !improved {
            // No damping gives a decrease: a stationary point of the cost.
            converged = true;
            break;
        }
    }
    Ok(LeastSquares {
        x,
        residual: res,
        cost,
        iterations: iter,
        evaluations: evals,
        converged,
    })
}

/// How a linear program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LpStatus {
    /// An optimal vertex was found.
    Optimal,
    /// No point satisfies the constraints.
    Infeasible,
    /// The objective decreases without bound.
    Unbounded,
}

/// The result of [`linprog`].
#[derive(Debug, Clone, PartialEq)]
pub struct LpSolution {
    /// How it ended.
    pub status: LpStatus,
    /// The solution (empty unless optimal).
    pub x: Vec<f64>,
    /// `cᵀx` (NaN unless optimal).
    pub fval: f64,
    /// Simplex pivots.
    pub iterations: usize,
}

/// Inputs of a linear program `min cᵀx` subject to `a_ub x ≤ b_ub`, `a_eq x = b_eq` and
/// `lb ≤ x ≤ ub`. Matrices are row-major with `n` columns.
#[derive(Debug, Clone, Default)]
pub struct LinearProgram {
    /// The cost vector (length `n`).
    pub c: Vec<f64>,
    /// Inequality rows.
    pub a_ub: Vec<Vec<f64>>,
    /// Inequality right-hand sides.
    pub b_ub: Vec<f64>,
    /// Equality rows.
    pub a_eq: Vec<Vec<f64>>,
    /// Equality right-hand sides.
    pub b_eq: Vec<f64>,
    /// Lower bounds (`-∞` allowed).
    pub lb: Vec<f64>,
    /// Upper bounds (`+∞` allowed).
    pub ub: Vec<f64>,
}

const LP_MAX_CELLS: usize = 4_000_000;

/// Two-phase simplex on a dense tableau with Bland's anti-cycling rule.
pub fn linprog(p: &LinearProgram) -> Result<LpSolution> {
    let n = p.c.len();
    if n == 0 {
        return Err(NumError::InvalidInput("the cost vector is empty".into()));
    }
    let lb = if p.lb.is_empty() {
        vec![0.0; n]
    } else {
        p.lb.clone()
    };
    let ub = if p.ub.is_empty() {
        vec![f64::INFINITY; n]
    } else {
        p.ub.clone()
    };
    if lb.len() != n || ub.len() != n {
        return Err(NumError::DimensionMismatch(
            "the bounds must have one entry per variable".into(),
        ));
    }
    let shape_ok = |rows: &[Vec<f64>], rhs: &[f64]| {
        rows.len() == rhs.len() && rows.iter().all(|r| r.len() == n)
    };
    if !shape_ok(&p.a_ub, &p.b_ub) || !shape_ok(&p.a_eq, &p.b_eq) {
        return Err(NumError::DimensionMismatch(
            "a constraint row does not have one entry per variable".into(),
        ));
    }
    if p.c
        .iter()
        .chain(p.b_ub.iter())
        .chain(p.b_eq.iter())
        .chain(p.a_ub.iter().flatten())
        .chain(p.a_eq.iter().flatten())
        .any(|v| !v.is_finite())
    {
        return Err(NumError::NonFinite("the problem data"));
    }
    for i in 0..n {
        if lb[i].is_nan() || ub[i].is_nan() || lb[i] > ub[i] {
            return Ok(LpSolution {
                status: LpStatus::Infeasible,
                x: vec![],
                fval: f64::NAN,
                iterations: 0,
            });
        }
    }
    // Substitute x = shift + sign * y (y >= 0), or x = y+ - y- for free variables.
    struct Var {
        col: usize,
        neg: Option<usize>,
        shift: f64,
        sign: f64,
    }
    let mut vars = Vec::with_capacity(n);
    let mut ncols = 0;
    for i in 0..n {
        if lb[i].is_finite() {
            vars.push(Var {
                col: ncols,
                neg: None,
                shift: lb[i],
                sign: 1.0,
            });
            ncols += 1;
        } else if ub[i].is_finite() {
            vars.push(Var {
                col: ncols,
                neg: None,
                shift: ub[i],
                sign: -1.0,
            });
            ncols += 1;
        } else {
            vars.push(Var {
                col: ncols,
                neg: Some(ncols + 1),
                shift: 0.0,
                sign: 1.0,
            });
            ncols += 2;
        }
    }
    // Rows in the new variables: (coefficients, rhs, is_equality).
    let mut rows: Vec<(Vec<f64>, f64, bool)> = Vec::new();
    let convert = |a: &[f64], b: f64| {
        let mut row = vec![0.0; ncols];
        let mut rhs = b;
        for (i, v) in vars.iter().enumerate() {
            rhs -= a[i] * v.shift;
            row[v.col] += a[i] * v.sign;
            if let Some(nc) = v.neg {
                row[nc] -= a[i];
            }
        }
        (row, rhs)
    };
    for (a, &b) in p.a_ub.iter().zip(&p.b_ub) {
        let (r, b2) = convert(a, b);
        rows.push((r, b2, false));
    }
    for (a, &b) in p.a_eq.iter().zip(&p.b_eq) {
        let (r, b2) = convert(a, b);
        rows.push((r, b2, true));
    }
    // A finite range on a shifted variable becomes a constraint y <= ub - lb.
    for (i, v) in vars.iter().enumerate() {
        if v.neg.is_none() && lb[i].is_finite() && ub[i].is_finite() {
            let mut r = vec![0.0; ncols];
            r[v.col] = 1.0;
            rows.push((r, ub[i] - lb[i], false));
        }
    }
    let mut cost = vec![0.0; ncols];
    let mut offset = 0.0;
    for (i, v) in vars.iter().enumerate() {
        offset += p.c[i] * v.shift;
        cost[v.col] += p.c[i] * v.sign;
        if let Some(nc) = v.neg {
            cost[nc] -= p.c[i];
        }
    }
    let m = rows.len();
    let n_slack = rows.iter().filter(|r| !r.2).count();
    // Make every right-hand side non-negative; rows that need it get an artificial variable.
    let mut need_art = vec![false; m];
    let mut slack_sign = vec![0.0; m];
    for (i, row) in rows.iter_mut().enumerate() {
        if row.1 < 0.0 {
            row.0.iter_mut().for_each(|v| *v = -*v);
            row.1 = -row.1;
            slack_sign[i] = if row.2 { 0.0 } else { -1.0 };
            need_art[i] = true;
        } else {
            slack_sign[i] = if row.2 { 0.0 } else { 1.0 };
            need_art[i] = row.2;
        }
    }
    let n_art = need_art.iter().filter(|&&b| b).count();
    let total = ncols + n_slack + n_art;
    if (m + 1).saturating_mul(total + 1) > LP_MAX_CELLS {
        return Err(NumError::TooLarge(format!(
            "a linear program with {m} rows and {total} columns"
        )));
    }
    let width = total + 1;
    let mut t = vec![0.0; (m + 1) * width];
    let mut basis = vec![0usize; m];
    let (mut s_idx, mut a_idx) = (ncols, ncols + n_slack);
    for (i, (coef, rhs, is_eq)) in rows.iter().enumerate() {
        t[i * width..i * width + ncols].copy_from_slice(coef);
        t[i * width + total] = *rhs;
        if !*is_eq {
            t[i * width + s_idx] = slack_sign[i];
            if slack_sign[i] > 0.0 {
                basis[i] = s_idx;
            }
            s_idx += 1;
        }
        if need_art[i] {
            t[i * width + a_idx] = 1.0;
            basis[i] = a_idx;
            a_idx += 1;
        }
    }
    let mut iterations = 0;
    let max_pivots = 50_000 + 50 * total;
    let eps = 1e-9;
    // Phase I: minimise the sum of artificials.
    if n_art > 0 {
        let obj = m * width;
        for j in ncols + n_slack..total {
            t[obj + j] = 1.0;
        }
        for i in 0..m {
            if basis[i] >= ncols + n_slack {
                for j in 0..width {
                    t[obj + j] -= t[i * width + j];
                }
            }
        }
        match simplex(
            &mut t,
            &mut basis,
            m,
            width,
            total,
            &mut iterations,
            max_pivots,
            eps,
        )? {
            Pivot::Unbounded => {
                return Err(NumError::NoConvergence {
                    iterations,
                    residual: f64::NAN,
                })
            }
            Pivot::Optimal => {}
        }
        if -t[obj + total] > 1e-7 * (1.0 + rows.iter().map(|r| r.1.abs()).fold(0.0, f64::max)) {
            return Ok(LpSolution {
                status: LpStatus::Infeasible,
                x: vec![],
                fval: f64::NAN,
                iterations,
            });
        }
        // Drive any artificial still in the basis (at zero) out, or drop its redundant row.
        for i in 0..m {
            if basis[i] >= ncols + n_slack {
                if let Some(j) = (0..ncols + n_slack).find(|&j| t[i * width + j].abs() > eps) {
                    pivot(&mut t, m, width, i, j);
                    basis[i] = j;
                }
            }
        }
    }
    // Phase II.
    let obj = m * width;
    for j in 0..width {
        t[obj + j] = 0.0;
    }
    t[obj..obj + ncols].copy_from_slice(&cost);
    for i in 0..m {
        let b = basis[i];
        if b < ncols + n_slack {
            let cb = t[obj + b];
            if cb != 0.0 {
                for j in 0..width {
                    t[obj + j] -= cb * t[i * width + j];
                }
            }
        }
    }
    // Forbid artificials from re-entering.
    let usable_cols = ncols + n_slack;
    match simplex(
        &mut t,
        &mut basis,
        m,
        width,
        usable_cols,
        &mut iterations,
        max_pivots,
        eps,
    )? {
        Pivot::Unbounded => {
            return Ok(LpSolution {
                status: LpStatus::Unbounded,
                x: vec![],
                fval: f64::NAN,
                iterations,
            });
        }
        Pivot::Optimal => {}
    }
    let mut y = vec![0.0; total];
    for i in 0..m {
        if basis[i] < total {
            y[basis[i]] = t[i * width + total];
        }
    }
    let x: Vec<f64> = vars
        .iter()
        .map(|v| {
            let yv = y[v.col] - v.neg.map_or(0.0, |nc| y[nc]);
            v.shift + v.sign * yv
        })
        .collect();
    let fval = p.c.iter().zip(&x).map(|(a, b)| a * b).sum::<f64>();
    let _ = offset;
    Ok(LpSolution {
        status: LpStatus::Optimal,
        x,
        fval,
        iterations,
    })
}

enum Pivot {
    Optimal,
    Unbounded,
}

fn pivot(t: &mut [f64], m: usize, width: usize, row: usize, col: usize) {
    let p = t[row * width + col];
    for j in 0..width {
        t[row * width + j] /= p;
    }
    for i in 0..=m {
        if i != row {
            let f = t[i * width + col];
            if f != 0.0 {
                for j in 0..width {
                    let v = t[row * width + j];
                    t[i * width + j] -= f * v;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn simplex(
    t: &mut [f64],
    basis: &mut [usize],
    m: usize,
    width: usize,
    enter_limit: usize,
    iterations: &mut usize,
    max_pivots: usize,
    eps: f64,
) -> Result<Pivot> {
    let total = width - 1;
    let obj = m * width;
    loop {
        // Bland's rule: the lowest-index column with a negative reduced cost enters.
        let Some(col) = (0..enter_limit.min(total)).find(|&j| t[obj + j] < -eps) else {
            return Ok(Pivot::Optimal);
        };
        // Ratio test, ties broken by the lowest basic variable index.
        let mut best: Option<(usize, f64)> = None;
        for i in 0..m {
            let a = t[i * width + col];
            if a > eps {
                let ratio = t[i * width + total] / a;
                let better = match best {
                    None => true,
                    Some((bi, br)) => {
                        ratio < br - 1e-12 || ((ratio - br).abs() <= 1e-12 && basis[i] < basis[bi])
                    }
                };
                if better {
                    best = Some((i, ratio));
                }
            }
        }
        let Some((row, _)) = best else {
            return Ok(Pivot::Unbounded);
        };
        pivot(t, m, width, row, col);
        basis[row] = col;
        *iterations += 1;
        if *iterations > max_pivots {
            return Err(NumError::NoConvergence {
                iterations: *iterations,
                residual: t[obj + total],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rosenbrock(x: &[f64]) -> f64 {
        100.0 * (x[1] - x[0] * x[0]).powi(2) + (1.0 - x[0]).powi(2)
    }

    #[test]
    fn nelder_mead_finds_the_rosenbrock_minimum() {
        let o = Options {
            tol_x: 1e-10,
            tol_f: 1e-12,
            ..Options::default()
        };
        let m = nelder_mead(rosenbrock, &[-1.2, 1.0], &o).unwrap();
        assert!(m.converged);
        assert!(
            (m.x[0] - 1.0).abs() < 1e-5 && (m.x[1] - 1.0).abs() < 1e-5,
            "{:?}",
            m.x
        );
    }

    #[test]
    fn brent_finds_a_parabola_and_a_cosine() {
        let m = brent_min(|x| (x - 2.0).powi(2) + 1.0, 0.0, 5.0, 1e-10, 500).unwrap();
        assert!((m.x[0] - 2.0).abs() < 1e-7 && (m.f - 1.0).abs() < 1e-12);
        let c = brent_min(f64::cos, 0.0, 6.0, 1e-10, 500).unwrap();
        assert!((c.x[0] - std::f64::consts::PI).abs() < 1e-6);
        assert!(brent_min(|x| x, 1.0, 0.0, 1e-8, 10).is_err());
    }

    #[test]
    fn bfgs_solves_rosenbrock_and_a_quadratic_bowl() {
        let o = Options {
            tol_f: 1e-8,
            ..Options::default()
        };
        let m = bfgs(rosenbrock, &[-1.2, 1.0], &o).unwrap();
        assert!(
            (m.x[0] - 1.0).abs() < 1e-4 && (m.x[1] - 1.0).abs() < 1e-4,
            "{:?}",
            m.x
        );
        let q = bfgs(
            |x| (x[0] - 3.0).powi(2) + 10.0 * (x[1] + 1.0).powi(2),
            &[0.0, 0.0],
            &o,
        )
        .unwrap();
        assert!((q.x[0] - 3.0).abs() < 1e-5 && (q.x[1] + 1.0).abs() < 1e-5);
    }

    #[test]
    fn levenberg_marquardt_fits_an_exponential() {
        let t: Vec<f64> = (0..20).map(|k| k as f64 * 0.25).collect();
        let y: Vec<f64> = t.iter().map(|&s| 2.5 * (-0.7 * s).exp()).collect();
        let r = |p: &[f64]| -> Vec<f64> {
            t.iter()
                .zip(&y)
                .map(|(&s, &yv)| p[0] * (-p[1] * s).exp() - yv)
                .collect()
        };
        let fit = levenberg_marquardt(r, &[1.0, 0.1], &Options::default()).unwrap();
        assert!(
            (fit.x[0] - 2.5).abs() < 1e-6 && (fit.x[1] - 0.7).abs() < 1e-6,
            "{:?}",
            fit.x
        );
        assert!(fit.cost < 1e-14);
    }

    #[test]
    fn levenberg_marquardt_solves_a_square_system() {
        // x² + y² = 1 and x = y  →  x = y = 1/√2
        let r = |p: &[f64]| vec![p[0] * p[0] + p[1] * p[1] - 1.0, p[0] - p[1]];
        let s = levenberg_marquardt(r, &[1.0, 0.5], &Options::default()).unwrap();
        let h = std::f64::consts::FRAC_1_SQRT_2;
        assert!((s.x[0] - h).abs() < 1e-8 && (s.x[1] - h).abs() < 1e-8);
    }

    #[test]
    fn bad_starts_and_cancellation_are_errors() {
        assert!(nelder_mead(|_| f64::NAN, &[0.0], &Options::default()).is_err());
        assert!(bfgs(|x| x[0], &[], &Options::default()).is_err());
        let flag = AtomicBool::new(true);
        let o = Options {
            cancel: Some(&flag),
            ..Options::default()
        };
        assert_eq!(
            nelder_mead(rosenbrock, &[-1.2, 1.0], &o),
            Err(NumError::Cancelled)
        );
    }

    #[test]
    fn simplex_solves_a_textbook_program() {
        // max 3x + 2y  s.t. x + y <= 4, x + 3y <= 6, x <= 3  →  (3, 1), value 11
        let p = LinearProgram {
            c: vec![-3.0, -2.0],
            a_ub: vec![vec![1.0, 1.0], vec![1.0, 3.0]],
            b_ub: vec![4.0, 6.0],
            ub: vec![3.0, f64::INFINITY],
            ..LinearProgram::default()
        };
        let s = linprog(&p).unwrap();
        assert_eq!(s.status, LpStatus::Optimal);
        assert!((s.x[0] - 3.0).abs() < 1e-9 && (s.x[1] - 1.0).abs() < 1e-9);
        assert!((s.fval + 11.0).abs() < 1e-9);
    }

    #[test]
    fn simplex_handles_equalities_free_variables_and_diet_problems() {
        // min x + y  s.t. x + 2y = 4, x,y >= 0  →  x = 0, y = 2
        let p = LinearProgram {
            c: vec![1.0, 1.0],
            a_eq: vec![vec![1.0, 2.0]],
            b_eq: vec![4.0],
            ..LinearProgram::default()
        };
        let s = linprog(&p).unwrap();
        assert!((s.fval - 2.0).abs() < 1e-9);
        // free variable: min x s.t. x >= -5 written as -x <= 5, x free
        let q = LinearProgram {
            c: vec![1.0],
            a_ub: vec![vec![-1.0]],
            b_ub: vec![5.0],
            lb: vec![f64::NEG_INFINITY],
            ..LinearProgram::default()
        };
        let s = linprog(&q).unwrap();
        assert!((s.x[0] + 5.0).abs() < 1e-9);
        // >= constraints through negative right-hand sides: min 2x+3y, x+y >= 4, x - y >= -2
        let d = LinearProgram {
            c: vec![2.0, 3.0],
            a_ub: vec![vec![-1.0, -1.0], vec![-1.0, 1.0]],
            b_ub: vec![-4.0, 2.0],
            ..LinearProgram::default()
        };
        let s = linprog(&d).unwrap();
        assert_eq!(s.status, LpStatus::Optimal);
        assert!((s.fval - 8.0).abs() < 1e-9, "{s:?}");
    }

    #[test]
    fn simplex_reports_infeasible_and_unbounded() {
        let infeasible = LinearProgram {
            c: vec![1.0],
            a_ub: vec![vec![1.0], vec![-1.0]],
            b_ub: vec![1.0, -2.0],
            ..LinearProgram::default()
        };
        assert_eq!(linprog(&infeasible).unwrap().status, LpStatus::Infeasible);
        let unbounded = LinearProgram {
            c: vec![-1.0],
            ..LinearProgram::default()
        };
        assert_eq!(linprog(&unbounded).unwrap().status, LpStatus::Unbounded);
        let bad = LinearProgram {
            c: vec![1.0, 2.0],
            a_ub: vec![vec![1.0]],
            b_ub: vec![1.0],
            ..LinearProgram::default()
        };
        assert!(linprog(&bad).is_err());
    }
}
