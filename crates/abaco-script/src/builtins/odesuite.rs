// SPDX-License-Identifier: MIT OR Apache-2.0
//! Ordinary differential equations: `ode45`, `ode23`, `ode113` (non-stiff), `ode23s`, `ode15s`,
//! `ode23t` (stiff, with a constant mass matrix for index-1 DAEs), and `odeset`/`odeget`.

use super::optimize::Caller;
use super::{arg, arity, one, Bf, Out};
use crate::error::{err, Error, Result};
use crate::interp::Interp;
use crate::linalg::num_err;
use crate::ops::MAX_ELEMENTS;
use crate::value::{Arr, StructV, Value};
use nodalia_numerics::ivp::{self, EventSpec, Method, Options};

fn read_ode_options(a: &[Value], from: usize) -> Result<Option<StructV>> {
    match a.get(from) {
        None => Ok(None),
        Some(Value::Struct(s)) => Ok(Some((**s).clone())),
        Some(Value::Num(e)) if e.d.is_empty() => Ok(None),
        Some(other) => err(
            "Abaco:type",
            format!(
                "the options must be a struct from odeset, got {}",
                other.describe()
            ),
        ),
    }
}

fn opt<'s>(s: &'s Option<StructV>, name: &str) -> Option<&'s Value> {
    s.as_ref().and_then(|s| {
        s.fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
            .filter(|v| !matches!(v, Value::Num(a) if a.d.is_empty()))
    })
}

fn positive(s: &Option<StructV>, name: &str) -> Result<Option<f64>> {
    match opt(s, name) {
        None => Ok(None),
        Some(Value::Num(a)) if a.d.len() > 1 && name.eq_ignore_ascii_case("AbsTol") => err(
            "Abaco:domain",
            "a vector AbsTol is not supported; give one scalar tolerance",
        ),
        Some(v) => {
            let x = v.scalar_f64(&format!("the option {name}"))?;
            if x > 0.0 && x.is_finite() {
                Ok(Some(x))
            } else {
                err(
                    "Abaco:domain",
                    format!("the option {name} must be positive and finite"),
                )
            }
        }
    }
}

fn solve_with(i: &mut Interp, a: &[Value], name: &str, method: Method) -> Out {
    arity(a, 3, 4, name)?;
    let tspan = super::real_arg(a, 1, name)?;
    if tspan.d.len() < 2 || tspan.d.iter().any(|t| !t.is_finite()) {
        return err(
            "Abaco:domain",
            format!("`{name}`: tspan needs at least two finite times"),
        );
    }
    let up = tspan.d[1] > tspan.d[0];
    if tspan
        .d
        .windows(2)
        .any(|w| (w[1] > w[0]) != up || w[1] == w[0])
    {
        return err(
            "Abaco:domain",
            format!("`{name}`: tspan must be strictly monotone"),
        );
    }
    let y0 = super::real_arg(a, 2, name)?;
    if y0.d.is_empty() || y0.d.iter().any(|v| !v.is_finite()) {
        return err(
            "Abaco:domain",
            format!("`{name}`: the initial state must be non-empty and finite"),
        );
    }
    let n = y0.d.len();
    let o = read_ode_options(a, 3)?;
    let mut opts = Options {
        method,
        ..Options::default()
    };
    if let Some(v) = positive(&o, "RelTol")? {
        opts.rtol = v;
    }
    if let Some(v) = positive(&o, "AbsTol")? {
        opts.atol = v;
    }
    if let Some(v) = positive(&o, "MaxStep")? {
        opts.max_step = v;
    }
    if let Some(v) = positive(&o, "InitialStep")? {
        opts.initial_step = Some(v);
    }
    match opt(&o, "Mass") {
        None => {}
        Some(Value::Fn(_)) | Some(Value::Str(_)) => {
            return err(
                "Abaco:type",
                "time-dependent mass matrices are not supported; give a constant matrix",
            );
        }
        Some(m) => {
            let m = m.to_real()?;
            if m.r != n || m.c != n {
                return err(
                    "Abaco:size",
                    format!("the mass matrix must be {n}x{n}, it is {}x{}", m.r, m.c),
                );
            }
            let mut rm = Vec::with_capacity(n * n);
            for r in 0..n {
                rm.extend(m.row_vec(r));
            }
            if method == Method::Dopri5 {
                opts.method = Method::Rosenbrock;
            }
            opts.mass = Some(rm);
        }
    }
    let cancel = i.cancel.clone();
    opts.cancel = Some(&cancel);
    opts.max_steps = 200_000;
    let ev_fn = opt(&o, "Events").cloned();
    let (t0, tf) = (tspan.d[0], tspan.d[tspan.d.len() - 1]);
    let t_out: Option<&[f64]> = if tspan.d.len() > 2 {
        Some(&tspan.d)
    } else {
        None
    };

    // The event function's flags are read once, at the start.
    let mut specs: Vec<EventSpec> = Vec::new();
    if let Some(ef) = &ev_fn {
        let r = i.call_value(ef, vec![Value::num(t0), Value::col(y0.d.clone())], 3)?;
        if r.len() < 3 {
            return err(
                "Abaco:arity",
                "the Events function must return [value, isterminal, direction]",
            );
        }
        let term = r[1].to_real()?.d;
        let dir = r[2].to_real()?.d;
        let count = r[0].to_real()?.d.len();
        if term.len() != count || dir.len() != count {
            return err(
                "Abaco:size",
                "isterminal and direction must have one entry per event value",
            );
        }
        specs = (0..count)
            .map(|k| EventSpec {
                terminal: term[k] != 0.0,
                direction: if dir[k] > 0.0 {
                    1
                } else if dir[k] < 0.0 {
                    -1
                } else {
                    0
                },
            })
            .collect();
    }

    let caller = std::cell::RefCell::new(Caller::new(i, arg(a, 0, name)?, name)?);
    let mut f = |t: f64, y: &[f64], dy: &mut [f64]| {
        let mut c = caller.borrow_mut();
        match c.call(vec![Value::num(t), Value::col(y.to_vec())]) {
            Some(v) if v.d.len() == n => dy.copy_from_slice(&v.d),
            Some(v) => {
                c.failure = Some(Error::new(
                    "Abaco:size",
                    format!(
                        "the function returned {} values for a state of {n}",
                        v.d.len()
                    ),
                ));
                dy.fill(f64::NAN);
            }
            None => dy.fill(f64::NAN),
        }
    };
    let want = specs.len();
    let mut g = |t: f64, y: &[f64]| -> Vec<f64> {
        let mut c = caller.borrow_mut();
        let Some(ev) = &ev_fn else {
            return Vec::new();
        };
        if c.failure.is_some() {
            return vec![f64::NAN; want];
        }
        let r = c
            .interp
            .charge(1)
            .and_then(|()| {
                c.interp
                    .call_value(ev, vec![Value::num(t), Value::col(y.to_vec())], 3)
            })
            .and_then(|v| {
                v.into_iter()
                    .next()
                    .map_or(Ok(Arr::empty()), |x| x.to_real())
            });
        match r {
            Ok(a) if a.d.len() == want => a.d,
            Ok(a) => {
                c.failure = Some(Error::new(
                    "Abaco:size",
                    format!(
                        "the Events function returned {} values, but {want} at the start",
                        a.d.len()
                    ),
                ));
                vec![f64::NAN; want]
            }
            Err(e) => {
                c.failure = Some(e);
                vec![f64::NAN; want]
            }
        }
    };
    let r = if ev_fn.is_some() {
        ivp::solve(
            &mut f,
            Some((&mut g, &specs)),
            (t0, tf),
            &y0.d,
            t_out,
            &opts,
        )
    } else {
        ivp::solve(&mut f, None, (t0, tf), &y0.d, t_out, &opts)
    };
    caller.into_inner().finish()?;
    let sol = r.map_err(num_err)?;
    let nt = sol.t.len();
    if nt.saturating_mul(n) > MAX_ELEMENTS {
        return err(
            "Abaco:limit",
            "the solution would be too large; ask for fewer output times",
        );
    }
    // y as an n x nt matrix (column k is the state at t[k]); the [t, y] form transposes it.
    let mut ycols = Vec::with_capacity(nt * n);
    for row in &sol.y {
        ycols.extend_from_slice(row);
    }
    let ne = sol.events.len();
    let te: Vec<f64> = sol.events.iter().map(|e| e.t).collect();
    let ie: Vec<f64> = sol.events.iter().map(|e| (e.index + 1) as f64).collect();
    let mut ye_cols = Vec::with_capacity(ne * n);
    for k in 0..n {
        for e in &sol.events {
            ye_cols.push(e.y.get(k).copied().unwrap_or(f64::NAN));
        }
    }
    let has_events = ev_fn.is_some();
    // nargout is not visible here; build both forms and let the caller pick through `Out`.
    let t_col = Value::col(sol.t.clone());
    let mut y_rows = Vec::with_capacity(nt * n);
    for k in 0..n {
        for row in &sol.y {
            y_rows.push(row[k]);
        }
    }
    let y_mat = Value::mat(nt, n, y_rows);
    let mut s = StructV::default();
    s.set("solver", Value::text(name));
    s.set("x", Value::row(sol.t.clone()));
    s.set("y", Value::mat(n, nt, ycols));
    if has_events {
        s.set("xe", Value::col(te.clone()));
        s.set("ye", Value::mat(ne, n, ye_cols.clone()));
        s.set("ie", Value::col(ie.clone()));
    }
    let stats = {
        let mut st = StructV::default();
        st.set("nsteps", Value::num(sol.accepted as f64));
        st.set("nfailed", Value::num(sol.rejected as f64));
        st.set("nfevals", Value::num(sol.evaluations as f64));
        Value::Struct(Box::new(st))
    };
    s.set("stats", stats);
    Ok(vec![
        t_col,
        y_mat,
        Value::col(te),
        Value::mat(ne, n, ye_cols),
        Value::col(ie),
        Value::Struct(Box::new(s)),
    ])
}

fn run(i: &mut Interp, a: &[Value], nargout: usize, name: &str, m: Method) -> Out {
    let all = solve_with(i, a, name, m)?;
    if nargout <= 1 {
        return one(all[5].clone());
    }
    Ok(all[..5].to_vec())
}

fn f_ode45(i: &mut Interp, a: &[Value], n: usize) -> Out {
    run(i, a, n, "ode45", Method::Dopri5)
}
fn f_ode23(i: &mut Interp, a: &[Value], n: usize) -> Out {
    run(i, a, n, "ode23", Method::Dopri5)
}
fn f_ode113(i: &mut Interp, a: &[Value], n: usize) -> Out {
    run(i, a, n, "ode113", Method::Dopri5)
}
fn f_ode23s(i: &mut Interp, a: &[Value], n: usize) -> Out {
    run(i, a, n, "ode23s", Method::Rosenbrock)
}
fn f_ode15s(i: &mut Interp, a: &[Value], n: usize) -> Out {
    run(i, a, n, "ode15s", Method::Rosenbrock)
}
fn f_ode23t(i: &mut Interp, a: &[Value], n: usize) -> Out {
    run(i, a, n, "ode23t", Method::Rosenbrock)
}

fn f_odeset(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let mut s = StructV::default();
    let mut k = 0;
    while k < a.len() {
        match &a[k] {
            Value::Struct(old) => {
                for (n, v) in &old.fields {
                    s.set(n, v.clone());
                }
                k += 1;
            }
            Value::Str(name) => {
                let v = a.get(k + 1).ok_or_else(|| {
                    Error::new(
                        "Abaco:arity",
                        format!("odeset: the option '{name}' has no value"),
                    )
                })?;
                s.set(name, v.clone());
                k += 2;
            }
            other => {
                return err(
                    "Abaco:type",
                    format!(
                        "`odeset` takes option names and values, got {}",
                        other.describe()
                    ),
                )
            }
        }
    }
    one(Value::Struct(Box::new(s)))
}

fn f_odeget(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "odeget")?;
    let Value::Struct(s) = &a[0] else {
        return err("Abaco:type", "`odeget` needs an options struct");
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
    ("ode113", f_ode113),
    ("ode15s", f_ode15s),
    ("ode23", f_ode23),
    ("ode23s", f_ode23s),
    ("ode23t", f_ode23t),
    ("ode45", f_ode45),
    ("odeget", f_odeget),
    ("odeset", f_odeset),
];
