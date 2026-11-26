// SPDX-License-Identifier: MIT OR Apache-2.0
//! Optimisation: `fminsearch`, `fminunc`, `fminbnd`, `fsolve`, `lsqnonlin`, `lsqcurvefit`,
//! `linprog`, and the `optimset`/`optimget` option structs.

use super::{arg, arity, num, one, Bf, Out};
use crate::error::{err, Error, Result};
use crate::interp::Interp;
use crate::linalg::num_err;
use crate::value::{Arr, StructV, Value};
use nodalia_numerics::optim::{self, LinearProgram, LpStatus, Options};
use nodalia_numerics::NumError;

/// Calls a script function from inside a numerical routine. The routines cannot return errors,
/// so the first failure is parked in `failure` and NaN data is returned from then on.
pub(crate) struct Caller<'a> {
    pub interp: &'a mut Interp,
    pub f: Value,
    pub failure: Option<Error>,
}

impl<'a> Caller<'a> {
    pub fn new(interp: &'a mut Interp, f: &Value, what: &str) -> Result<Self> {
        match f {
            Value::Fn(_) | Value::Str(_) => Ok(Self {
                interp,
                f: f.clone(),
                failure: None,
            }),
            other => err(
                "Abaco:type",
                format!("`{what}` needs a function handle, got {}", other.describe()),
            ),
        }
    }

    /// `f(args…)` as real data (column-major), or `None` after a failure.
    pub fn call(&mut self, args: Vec<Value>) -> Option<Arr<f64>> {
        if self.failure.is_some() {
            return None;
        }
        let r = self
            .interp
            .charge(1)
            .and_then(|()| self.interp.check_cancel())
            .and_then(|()| self.interp.call1(&self.f, args))
            .and_then(|v| v.to_real());
        match r {
            Ok(a) => {
                if a.d.len() > crate::ops::MAX_ELEMENTS {
                    self.failure = Some(Error::new(
                        "Abaco:limit",
                        "the function returned too much data",
                    ));
                    return None;
                }
                Some(a)
            }
            Err(e) => {
                self.failure = Some(e);
                None
            }
        }
    }

    pub fn scalar(&mut self, args: Vec<Value>) -> f64 {
        match self.call(args) {
            Some(a) if a.d.len() == 1 => a.d.first().copied().unwrap_or(f64::NAN),
            Some(a) => {
                self.failure = Some(Error::new(
                    "Abaco:size",
                    format!("the objective must return a scalar, got {}x{}", a.r, a.c),
                ));
                f64::NAN
            }
            None => f64::NAN,
        }
    }

    pub fn finish(self) -> Result<()> {
        match self.failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Reads `TolX`, `TolFun`, `MaxIter` and `MaxFunEvals` from a trailing options struct.
pub(crate) fn read_options(a: &[Value], from: usize, base: &mut OptionSet) -> Result<()> {
    for v in a.iter().skip(from) {
        match v {
            Value::Struct(s) => {
                for (name, val) in &s.fields {
                    base.set(name, val)?;
                }
            }
            Value::Num(e) if e.d.is_empty() => {}
            other => {
                return err(
                    "Abaco:type",
                    format!(
                        "options must be a struct from optimset, got {}",
                        other.describe()
                    ),
                )
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct OptionSet {
    pub tol_x: f64,
    pub tol_f: f64,
    pub max_iter: f64,
    pub max_evals: f64,
}

impl OptionSet {
    pub fn set(&mut self, name: &str, v: &Value) -> Result<()> {
        let slot = match name.to_ascii_lowercase().as_str() {
            "tolx" => &mut self.tol_x,
            "tolfun" => &mut self.tol_f,
            "maxiter" => &mut self.max_iter,
            "maxfunevals" => &mut self.max_evals,
            _ => return Ok(()),
        };
        if let Value::Num(a) = v {
            if a.d.is_empty() {
                return Ok(());
            }
        }
        let x = v.scalar_f64(&format!("the option {name}"))?;
        if x.is_nan() || x <= 0.0 {
            return err(
                "Abaco:domain",
                format!("the option {name} must be positive"),
            );
        }
        *slot = x;
        Ok(())
    }

    pub fn build<'c>(&self, cancel: &'c std::sync::atomic::AtomicBool) -> Options<'c> {
        Options {
            tol_x: self.tol_x,
            tol_f: self.tol_f,
            max_iter: self.max_iter.min(1e9) as usize,
            max_evals: self.max_evals.min(1e9) as usize,
            cancel: Some(cancel),
        }
    }
}

fn shaped(x: &[f64], like: &Arr<f64>) -> Value {
    Value::Num(Arr::new(like.r, like.c, x.to_vec()))
}

fn output_struct(iterations: usize, evals: usize, algorithm: &str) -> Value {
    let mut s = StructV::default();
    s.set("iterations", Value::num(iterations as f64));
    s.set("funcCount", Value::num(evals as f64));
    s.set("algorithm", Value::text(algorithm));
    Value::Struct(Box::new(s))
}

fn start_point(a: &[Value], i: usize, name: &str) -> Result<Arr<f64>> {
    let x0 = super::real_arg(a, i, name)?;
    if x0.d.is_empty() || x0.d.iter().any(|v| !v.is_finite()) {
        return err(
            "Abaco:domain",
            format!("`{name}`: the starting point must be non-empty and finite"),
        );
    }
    Ok(x0)
}

fn f_fminsearch(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "fminsearch")?;
    let x0 = start_point(a, 1, "fminsearch")?;
    let n = x0.d.len();
    let mut set = OptionSet {
        tol_x: 1e-4,
        tol_f: 1e-4,
        max_iter: 200.0 * n as f64,
        max_evals: 200.0 * n as f64,
    };
    read_options(a, 2, &mut set)?;
    let cancel = i.cancel.clone();
    let mut c = Caller::new(i, arg(a, 0, "fminsearch")?, "fminsearch")?;
    let like = x0.clone();
    let r = optim::nelder_mead(
        |x| c.scalar(vec![shaped(x, &like)]),
        &x0.d,
        &set.build(&cancel),
    );
    c.finish()?;
    let m = r.map_err(num_err)?;
    Ok(vec![
        shaped(&m.x, &x0),
        Value::num(m.f),
        Value::num(if m.converged { 1.0 } else { 0.0 }),
        output_struct(
            m.iterations,
            m.evaluations,
            "Nelder-Mead simplex direct search",
        ),
    ])
}

fn f_fminunc(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "fminunc")?;
    let x0 = start_point(a, 1, "fminunc")?;
    let mut set = OptionSet {
        tol_x: 1e-6,
        tol_f: 1e-6,
        max_iter: 400.0,
        max_evals: 100_000.0,
    };
    read_options(a, 2, &mut set)?;
    let cancel = i.cancel.clone();
    let mut c = Caller::new(i, arg(a, 0, "fminunc")?, "fminunc")?;
    let like = x0.clone();
    let r = optim::bfgs(
        |x| c.scalar(vec![shaped(x, &like)]),
        &x0.d,
        &set.build(&cancel),
    );
    c.finish()?;
    let m = r.map_err(num_err)?;
    Ok(vec![
        shaped(&m.x, &x0),
        Value::num(m.f),
        Value::num(if m.converged { 1.0 } else { 0.0 }),
        output_struct(m.iterations, m.evaluations, "BFGS quasi-Newton"),
    ])
}

fn f_fminbnd(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 4, "fminbnd")?;
    let lo = a
        .get(1)
        .map(|v| v.scalar_f64("the lower bound"))
        .transpose()?
        .unwrap_or(0.0);
    let hi = a
        .get(2)
        .map(|v| v.scalar_f64("the upper bound"))
        .transpose()?
        .unwrap_or(0.0);
    let mut set = OptionSet {
        tol_x: 1e-4,
        tol_f: 1e-4,
        max_iter: 500.0,
        max_evals: 500.0,
    };
    read_options(a, 3, &mut set)?;
    let mut c = Caller::new(i, arg(a, 0, "fminbnd")?, "fminbnd")?;
    let r = optim::brent_min(
        |x| c.scalar(vec![Value::num(x)]),
        lo,
        hi,
        set.tol_x,
        set.max_iter as usize,
    );
    c.finish()?;
    let m = r.map_err(num_err)?;
    Ok(vec![
        num(m.x.first().copied().unwrap_or(f64::NAN))?.remove(0),
        Value::num(m.f),
        Value::num(if m.converged { 1.0 } else { 0.0 }),
        output_struct(
            m.iterations,
            m.evaluations,
            "golden section search and parabolic interpolation",
        ),
    ])
}

struct Fit {
    x: Vec<f64>,
    residual: Vec<f64>,
    iterations: usize,
    evaluations: usize,
    converged: bool,
}

fn fit(
    i: &mut Interp,
    f: &Value,
    x0: &Arr<f64>,
    set: &OptionSet,
    name: &str,
    extra: Option<(&Value, &Value)>,
) -> Result<(Fit, usize, usize)> {
    let cancel = i.cancel.clone();
    let mut c = Caller::new(i, f, name)?;
    let like = x0.clone();
    let mut shape = (0usize, 0usize);
    let r = optim::levenberg_marquardt(
        |x| {
            let xv = shaped(x, &like);
            let args = match extra {
                Some((xd, _)) => vec![xv, xd.clone()],
                None => vec![xv],
            };
            match c.call(args) {
                Some(mut a) => {
                    shape = (a.r, a.c);
                    if let Some((_, yd)) = extra {
                        let want = match yd.to_real() {
                            Ok(y) => y,
                            Err(e) => {
                                c.failure = Some(e);
                                return vec![f64::NAN];
                            }
                        };
                        if want.d.len() != a.d.len() {
                            c.failure = Some(Error::new(
                                "Abaco:size",
                                format!(
                                    "the model returned {} values for {} data points",
                                    a.d.len(),
                                    want.d.len()
                                ),
                            ));
                            return vec![f64::NAN];
                        }
                        for (p, q) in a.d.iter_mut().zip(&want.d) {
                            *p -= q;
                        }
                    }
                    a.d
                }
                None => vec![f64::NAN],
            }
        },
        &x0.d,
        &set.build(&cancel),
    );
    c.finish()?;
    let s = r.map_err(num_err)?;
    Ok((
        Fit {
            x: s.x,
            residual: s.residual,
            iterations: s.iterations,
            evaluations: s.evaluations,
            converged: s.converged,
        },
        shape.0,
        shape.1,
    ))
}

fn lsq_defaults() -> OptionSet {
    OptionSet {
        tol_x: 1e-6,
        tol_f: 1e-6,
        max_iter: 400.0,
        max_evals: 100_000.0,
    }
}

fn f_fsolve(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "fsolve")?;
    let x0 = start_point(a, 1, "fsolve")?;
    let mut set = lsq_defaults();
    read_options(a, 2, &mut set)?;
    let (r, rr, rc) = fit(i, arg(a, 0, "fsolve")?, &x0, &set, "fsolve", None)?;
    let worst = r.residual.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let flag = if worst <= 1e-6_f64.max(set.tol_f) {
        1.0
    } else if r.converged {
        -2.0
    } else {
        0.0
    };
    Ok(vec![
        shaped(&r.x, &x0),
        Value::Num(Arr::new(rr, rc, r.residual)),
        Value::num(flag),
        output_struct(r.iterations, r.evaluations, "Levenberg-Marquardt"),
    ])
}

fn lsq_outputs(r: Fit, x0: &Arr<f64>, rr: usize, rc: usize) -> Out {
    let resnorm: f64 = r.residual.iter().map(|v| v * v).sum();
    let flag = if r.converged { 1.0 } else { 0.0 };
    Ok(vec![
        shaped(&r.x, x0),
        Value::num(resnorm),
        Value::Num(Arr::new(rr, rc, r.residual)),
        Value::num(flag),
        output_struct(r.iterations, r.evaluations, "Levenberg-Marquardt"),
    ])
}

fn f_lsqnonlin(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "lsqnonlin")?;
    let x0 = start_point(a, 1, "lsqnonlin")?;
    let mut set = lsq_defaults();
    read_options(a, 2, &mut set)?;
    let (r, rr, rc) = fit(i, arg(a, 0, "lsqnonlin")?, &x0, &set, "lsqnonlin", None)?;
    lsq_outputs(r, &x0, rr, rc)
}

fn f_lsqcurvefit(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 4, 5, "lsqcurvefit")?;
    let x0 = start_point(a, 1, "lsqcurvefit")?;
    let mut set = lsq_defaults();
    read_options(a, 4, &mut set)?;
    let (r, rr, rc) = fit(
        i,
        arg(a, 0, "lsqcurvefit")?,
        &x0,
        &set,
        "lsqcurvefit",
        Some((arg(a, 2, "lsqcurvefit")?, arg(a, 3, "lsqcurvefit")?)),
    )?;
    lsq_outputs(r, &x0, rr, rc)
}

fn matrix_rows(v: &Value, n: usize, what: &str) -> Result<Vec<Vec<f64>>> {
    let m = v.to_real()?;
    if m.d.is_empty() {
        return Ok(Vec::new());
    }
    if m.c != n {
        return err(
            "Abaco:size",
            format!("`linprog`: {what} must have {n} columns, it has {}", m.c),
        );
    }
    Ok((0..m.r).map(|r| m.row_vec(r)).collect())
}

fn f_linprog(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 7, "linprog")?;
    let c = super::vec_arg(a, 0, "linprog")?;
    let n = c.len();
    let get = |k: usize| a.get(k).cloned().unwrap_or_else(Value::empty);
    let a_ub = matrix_rows(&get(1), n, "A")?;
    let b_ub = get(2).to_real()?.d;
    let a_eq = matrix_rows(&get(3), n, "Aeq")?;
    let b_eq = get(4).to_real()?.d;
    let lb = get(5).to_real()?.d;
    let ub = get(6).to_real()?.d;
    let p = LinearProgram {
        c,
        a_ub,
        b_ub,
        a_eq,
        b_eq,
        lb,
        ub,
    };
    let s = optim::linprog(&p).map_err(|e| match e {
        NumError::NoConvergence { .. } => Error::new("Abaco:convergence", e.to_string()),
        other => num_err(other),
    })?;
    let (flag, x, fval) = match s.status {
        LpStatus::Optimal => (1.0, Value::col(s.x), Value::num(s.fval)),
        LpStatus::Infeasible => (-2.0, Value::empty(), Value::empty()),
        LpStatus::Unbounded => (-3.0, Value::empty(), Value::empty()),
    };
    Ok(vec![
        x,
        fval,
        Value::num(flag),
        output_struct(s.iterations, 0, "two-phase simplex"),
    ])
}

fn f_optimset(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let mut s = StructV::default();
    let mut i = 0;
    while i < a.len() {
        match &a[i] {
            Value::Struct(old) => {
                for (k, v) in &old.fields {
                    s.set(k, v.clone());
                }
                i += 1;
            }
            Value::Str(name) => {
                let v = a.get(i + 1).ok_or_else(|| {
                    Error::new(
                        "Abaco:arity",
                        format!("optimset: the option '{name}' has no value"),
                    )
                })?;
                s.set(name, v.clone());
                i += 2;
            }
            other => {
                return err(
                    "Abaco:type",
                    format!(
                        "`optimset` takes option names and values, got {}",
                        other.describe()
                    ),
                )
            }
        }
    }
    one(Value::Struct(Box::new(s)))
}

fn f_optimget(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "optimget")?;
    let Value::Struct(s) = &a[0] else {
        return err("Abaco:type", "`optimget` needs an options struct");
    };
    let name = a[1].as_str("the option name")?;
    let found = s
        .fields
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone());
    one(found
        .or_else(|| a.get(2).cloned())
        .unwrap_or_else(Value::empty))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("fminbnd", f_fminbnd),
    ("fminsearch", f_fminsearch),
    ("fminunc", f_fminunc),
    ("fsolve", f_fsolve),
    ("linprog", f_linprog),
    ("lsqcurvefit", f_lsqcurvefit),
    ("lsqnonlin", f_lsqnonlin),
    ("optimget", f_optimget),
    ("optimset", f_optimset),
];
