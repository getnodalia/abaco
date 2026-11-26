// SPDX-License-Identifier: MIT OR Apache-2.0
//! Numerical calculus on function handles: quadrature and root finding.

use super::{arg, arity, num, scalar_arg, Bf, Out};
use crate::error::{err, Error, Result};
use crate::interp::Interp;
use crate::linalg::num_err;
use crate::value::Value;
use nodalia_numerics::{quad, roots, NumError};

/// Calls `f(x)` and reads a real scalar. A failure is parked in `failure` and NaN returned,
/// because the numerical callbacks cannot return errors.
pub(crate) fn eval_scalar(
    interp: &mut Interp,
    f: &Value,
    x: f64,
    failure: &mut Option<Error>,
) -> f64 {
    if failure.is_some() {
        return f64::NAN;
    }
    if let Err(e) = interp.charge(1).and_then(|()| interp.check_cancel()) {
        *failure = Some(e);
        return f64::NAN;
    }
    match interp
        .call1(f, vec![Value::num(x)])
        .and_then(|v| v.scalar_f64("the function value"))
    {
        Ok(v) => v,
        Err(e) => {
            *failure = Some(e);
            f64::NAN
        }
    }
}

fn check_fn(v: &Value, name: &str) -> Result<()> {
    match v {
        Value::Fn(_) | Value::Str(_) => Ok(()),
        other => err(
            "Abaco:type",
            format!("`{name}` needs a function handle, got {}", other.describe()),
        ),
    }
}

fn tolerances(args: &[Value], from: usize) -> Result<(f64, f64)> {
    let (mut abs, mut rel) = (1e-10, 1e-6);
    let mut i = from;
    while i + 1 < args.len() {
        if let Value::Str(k) = &args[i] {
            match k.to_ascii_lowercase().as_str() {
                "abstol" => abs = args[i + 1].scalar_f64("AbsTol")?,
                "reltol" => rel = args[i + 1].scalar_f64("RelTol")?,
                _ => {}
            }
        }
        i += 2;
    }
    Ok((abs, rel))
}

/// ∫ f over [a, b], with infinite limits mapped to a finite interval.
fn integrate_with(
    interp: &mut Interp,
    f: &Value,
    a: f64,
    b: f64,
    abs: f64,
    rel: f64,
) -> Result<f64> {
    let mut failure: Option<Error> = None;
    let result = {
        let mut call = |x: f64| eval_scalar(interp, f, x, &mut failure);
        if a == b {
            return Ok(0.0);
        }
        let (lo, hi, sign) = if a < b { (a, b, 1.0) } else { (b, a, -1.0) };
        let r = if lo.is_infinite() && hi.is_infinite() {
            // x = t / (1 - t^2)
            quad::integrate(
                |t| {
                    let d = 1.0 - t * t;
                    let x = t / d;
                    call(x) * (1.0 + t * t) / (d * d)
                },
                -1.0,
                1.0,
                abs,
                rel,
                2000,
            )
        } else if hi.is_infinite() {
            // x = lo + t / (1 - t)
            quad::integrate(
                |t| {
                    let d = 1.0 - t;
                    call(lo + t / d) / (d * d)
                },
                0.0,
                1.0,
                abs,
                rel,
                2000,
            )
        } else if lo.is_infinite() {
            // x = hi - t / (1 - t)
            quad::integrate(
                |t| {
                    let d = 1.0 - t;
                    call(hi - t / d) / (d * d)
                },
                0.0,
                1.0,
                abs,
                rel,
                2000,
            )
        } else {
            quad::integrate(&mut call, lo, hi, abs, rel, 2000)
        };
        r.map(|i| sign * i.value)
    };
    if let Some(e) = failure {
        return Err(e);
    }
    result.map_err(|e| match e {
        NumError::NonFinite(_) => Error::new(
            "Abaco:numeric",
            "the integrand returned NaN or Inf inside the interval",
        ),
        other => num_err(other),
    })
}

fn f_integral(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 7, "integral")?;
    check_fn(arg(a, 0, "integral")?, "integral")?;
    let (lo, hi) = (scalar_arg(a, 1, "integral")?, scalar_arg(a, 2, "integral")?);
    let (abs, rel) = tolerances(a, 3)?;
    num(integrate_with(i, &a[0], lo, hi, abs, rel)?)
}

fn f_quad(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 4, "quad")?;
    check_fn(arg(a, 0, "quad")?, "quad")?;
    let (lo, hi) = (scalar_arg(a, 1, "quad")?, scalar_arg(a, 2, "quad")?);
    let tol = if a.len() > 3 {
        scalar_arg(a, 3, "quad")?
    } else {
        1e-10
    };
    num(integrate_with(i, &a[0], lo, hi, tol, 1e-8)?)
}

fn f_integral2(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 5, 5, "integral2")?;
    check_fn(arg(a, 0, "integral2")?, "integral2")?;
    let (xa, xb, ya, yb) = (
        scalar_arg(a, 1, "integral2")?,
        scalar_arg(a, 2, "integral2")?,
        scalar_arg(a, 3, "integral2")?,
        scalar_arg(a, 4, "integral2")?,
    );
    let mut failure: Option<Error> = None;
    let f = a[0].clone();
    let outer = quad::integrate(
        |x| {
            if failure.is_some() {
                return f64::NAN;
            }
            let mut inner_fail: Option<Error> = None;
            let r = quad::integrate(
                |y| {
                    if inner_fail.is_some() {
                        return f64::NAN;
                    }
                    if let Err(e) = i.charge(1).and_then(|()| i.check_cancel()) {
                        inner_fail = Some(e);
                        return f64::NAN;
                    }
                    match i
                        .call1(&f, vec![Value::num(x), Value::num(y)])
                        .and_then(|v| v.scalar_f64("the integrand value"))
                    {
                        Ok(v) => v,
                        Err(e) => {
                            inner_fail = Some(e);
                            f64::NAN
                        }
                    }
                },
                ya,
                yb,
                1e-10,
                1e-7,
                200,
            );
            if let Some(e) = inner_fail {
                failure = Some(e);
                return f64::NAN;
            }
            match r {
                Ok(v) => v.value,
                Err(e) => {
                    failure = Some(num_err(e));
                    f64::NAN
                }
            }
        },
        xa,
        xb,
        1e-10,
        1e-7,
        200,
    );
    if let Some(e) = failure {
        return Err(e);
    }
    num(outer.map_err(num_err)?.value)
}

fn f_fzero(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "fzero")?;
    check_fn(arg(a, 0, "fzero")?, "fzero")?;
    let f = a[0].clone();
    let x0 = super::real_arg(a, 1, "fzero")?;
    let mut failure: Option<Error> = None;
    let (lo, hi) = match x0.d.as_slice() {
        [p, q] => (*p, *q),
        [x] => {
            let fx = eval_scalar(i, &f, *x, &mut failure);
            if let Some(e) = failure {
                return Err(e);
            }
            if fx == 0.0 {
                return Ok(vec![Value::num(*x), Value::num(0.0), Value::num(1.0)]);
            }
            let base = if *x == 0.0 { 1.0 } else { x.abs() };
            let mut found = None;
            for k in 0..80 {
                let d = base * 0.02 * 1.4f64.powi(k);
                for cand in [*x - d, *x + d] {
                    let fc = eval_scalar(i, &f, cand, &mut failure);
                    if let Some(e) = failure.take() {
                        return Err(e);
                    }
                    if fc.is_finite() && (fc == 0.0 || (fc < 0.0) != (fx < 0.0)) {
                        found = Some(cand);
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            match found {
                Some(c) => (x.min(c), x.max(c)),
                None => return err("Abaco:convergence", format!("`fzero` could not find a sign change near {x}; give an interval [a b] where f changes sign")),
            }
        }
        _ => {
            return err(
                "Abaco:type",
                "`fzero` needs a starting point or an interval [a b]",
            )
        }
    };
    let r = roots::brent(|x| eval_scalar(i, &f, x, &mut failure), lo, hi, 1e-15, 200);
    if let Some(e) = failure {
        return Err(e);
    }
    match r {
        Ok(root) => Ok(vec![
            Value::num(root.x),
            Value::num(root.fx),
            Value::num(1.0),
        ]),
        Err(NumError::NoBracket) => err(
            "Abaco:convergence",
            format!("the function does not change sign on [{lo}, {hi}]"),
        ),
        Err(e) => Err(num_err(e)),
    }
}

pub const TABLE: &[(&str, Bf)] = &[
    ("integral", f_integral),
    ("quad", f_quad),
    ("quadgk", f_integral),
    ("integral2", f_integral2),
    ("fzero", f_fzero),
];
