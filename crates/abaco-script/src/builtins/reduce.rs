// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reductions, scans, sorting and searching.

use super::{
    arg, arity, bool_arr, is_flag, num, one, pick_dim, real_arg, real_out, reduce_dim, scan_dim,
    shaped_like, usize_arg, Bf, Out,
};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::ops::zip;
use crate::value::{Arr, Value};
use num_complex::Complex64;
use std::cmp::Ordering;

type C = Complex64;

/// The dimension a reduction works along; `None` for a bare `[]`, which reduces to a scalar.
fn dim_of(a: &Arr<f64>, args: &[Value], i: usize, name: &str) -> Result<Option<usize>> {
    if args.get(i).is_some() {
        return pick_dim(args, i, a.r, name).map(Some);
    }
    if a.r == 0 && a.c == 0 {
        return Ok(None);
    }
    Ok(Some(if a.r == 1 { 2 } else { 1 }))
}

fn reduce_real(args: &[Value], name: &str, f: impl Fn(&[f64]) -> f64) -> Out {
    arity(args, 1, 2, name)?;
    let a = real_arg(args, 0, name)?;
    match dim_of(&a, args, 1, name)? {
        None => num(f(&[])),
        Some(d) => real_out(reduce_dim(&a, d, f)),
    }
}

fn reduce_cplx(args: &[Value], name: &str, f: impl Fn(&[C]) -> C) -> Out {
    arity(args, 1, 2, name)?;
    let a = arg(args, 0, name)?.to_complex()?;
    let d = if args.len() > 1 {
        pick_dim(args, 1, a.r, name)?
    } else if a.r == 1 {
        2
    } else {
        1
    };
    one(Value::cplx(reduce_dim(&a, d, f)))
}

fn is_c(args: &[Value]) -> bool {
    matches!(args.first(), Some(Value::Cplx(_)))
}

fn f_sum(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if is_c(a) {
        return reduce_cplx(a, "sum", |v| v.iter().sum());
    }
    reduce_real(a, "sum", |v| v.iter().sum())
}

fn f_prod(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if is_c(a) {
        return reduce_cplx(a, "prod", |v| v.iter().product());
    }
    reduce_real(a, "prod", |v| v.iter().product())
}

fn f_mean(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if is_c(a) {
        return reduce_cplx(a, "mean", |v| v.iter().sum::<C>() / v.len().max(1) as f64);
    }
    reduce_real(a, "mean", |v| {
        if v.is_empty() {
            f64::NAN
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    })
}

fn sorted_no_nan(v: &[f64]) -> Vec<f64> {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| !x.is_nan()).collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    s
}

fn median_of(v: &[f64]) -> f64 {
    if v.is_empty() || v.iter().any(|x| x.is_nan()) {
        return f64::NAN;
    }
    let s = sorted_no_nan(v);
    let n = s.len();
    if n % 2 == 1 {
        s.get(n / 2).copied().unwrap_or(f64::NAN)
    } else {
        (s.get(n / 2 - 1).copied().unwrap_or(f64::NAN) + s.get(n / 2).copied().unwrap_or(f64::NAN))
            / 2.0
    }
}

fn f_median(_: &mut Interp, a: &[Value], _: usize) -> Out {
    reduce_real(a, "median", median_of)
}

fn f_mode(_: &mut Interp, a: &[Value], _: usize) -> Out {
    reduce_real(a, "mode", |v| {
        let s = sorted_no_nan(v);
        let (mut best, mut best_n, mut i) = (f64::NAN, 0usize, 0usize);
        while i < s.len() {
            let mut j = i;
            while j < s.len() && s.get(j) == s.get(i) {
                j += 1;
            }
            if j - i > best_n {
                best_n = j - i;
                best = s.get(i).copied().unwrap_or(f64::NAN);
            }
            i = j;
        }
        best
    })
}

fn variance(v: &[f64], flag: f64) -> f64 {
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    if n == 1 {
        return 0.0;
    }
    let m = v.iter().sum::<f64>() / n as f64;
    let ss: f64 = v.iter().map(|x| (x - m) * (x - m)).sum();
    ss / if flag == 1.0 {
        n as f64
    } else {
        (n - 1) as f64
    }
}

fn var_std(args: &[Value], name: &str, root: bool) -> Out {
    arity(args, 1, 3, name)?;
    let a = real_arg(args, 0, name)?;
    let flag = match args.get(1) {
        Some(v) if !v.is_empty() => v.scalar_f64("the weight flag")?,
        _ => 0.0,
    };
    if flag != 0.0 && flag != 1.0 {
        return err(
            "Abaco:domain",
            format!("`{name}`: the second argument must be 0 or 1"),
        );
    }
    let f = |v: &[f64]| {
        let x = variance(v, flag);
        if root {
            x.sqrt()
        } else {
            x
        }
    };
    match dim_of(&a, args, 2, name)? {
        None => num(f64::NAN),
        Some(d) => real_out(reduce_dim(&a, d, f)),
    }
}

fn f_var(_: &mut Interp, a: &[Value], _: usize) -> Out {
    var_std(a, "var", false)
}

fn f_std(_: &mut Interp, a: &[Value], _: usize) -> Out {
    var_std(a, "std", true)
}

fn f_range(_: &mut Interp, a: &[Value], _: usize) -> Out {
    reduce_real(a, "range", |v| {
        let s = sorted_no_nan(v);
        match (s.first(), s.last()) {
            (Some(l), Some(h)) => h - l,
            _ => f64::NAN,
        }
    })
}

fn f_nnz(_: &mut Interp, a: &[Value], _: usize) -> Out {
    num(arg(a, 0, "nnz")?
        .to_real()?
        .d
        .iter()
        .filter(|&&x| x != 0.0)
        .count() as f64)
}

// ---------- scans ----------

fn f_cumsum(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "cumsum")?;
    if is_c(args) {
        let a = args[0].to_complex()?;
        let d = if args.len() > 1 {
            pick_dim(args, 1, a.r, "cumsum")?
        } else if a.r == 1 {
            2
        } else {
            1
        };
        return one(Value::cplx(scan_dim(&a, d, |v| {
            let mut s = C::new(0.0, 0.0);
            v.iter()
                .map(|x| {
                    s += x;
                    s
                })
                .collect()
        })));
    }
    let a = real_arg(args, 0, "cumsum")?;
    let d = if args.len() > 1 {
        pick_dim(args, 1, a.r, "cumsum")?
    } else if a.r == 1 {
        2
    } else {
        1
    };
    real_out(scan_dim(&a, d, |v| {
        let mut s = 0.0;
        v.iter()
            .map(|x| {
                s += x;
                s
            })
            .collect()
    }))
}

fn f_cumprod(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "cumprod")?;
    let a = real_arg(args, 0, "cumprod")?;
    let d = if args.len() > 1 {
        pick_dim(args, 1, a.r, "cumprod")?
    } else if a.r == 1 {
        2
    } else {
        1
    };
    real_out(scan_dim(&a, d, |v| {
        let mut s = 1.0;
        v.iter()
            .map(|x| {
                s *= x;
                s
            })
            .collect()
    }))
}

fn cum_ext(args: &[Value], name: &str, better: fn(f64, f64) -> bool) -> Out {
    arity(args, 1, 2, name)?;
    let a = real_arg(args, 0, name)?;
    let d = if args.len() > 1 {
        pick_dim(args, 1, a.r, name)?
    } else if a.r == 1 {
        2
    } else {
        1
    };
    real_out(scan_dim(&a, d, |v| {
        let mut cur = f64::NAN;
        v.iter()
            .map(|&x| {
                if cur.is_nan() || (!x.is_nan() && better(x, cur)) {
                    cur = x;
                }
                cur
            })
            .collect()
    }))
}

fn f_cummax(_: &mut Interp, a: &[Value], _: usize) -> Out {
    cum_ext(a, "cummax", |x, c| x > c)
}

fn f_cummin(_: &mut Interp, a: &[Value], _: usize) -> Out {
    cum_ext(a, "cummin", |x, c| x < c)
}

fn f_diff(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 3, "diff")?;
    let mut a = real_arg(args, 0, "diff")?;
    let order = if args.len() > 1 && !args[1].is_empty() {
        usize_arg(args, 1, "diff")?
    } else {
        1
    };
    for _ in 0..order {
        let d = pick_dim(args, 2, a.r, "diff")?;
        a = if d == 1 {
            if a.r == 0 {
                return real_out(a);
            }
            let mut out = Vec::new();
            for j in 0..a.c {
                let col = a.column(j);
                out.extend(col.windows(2).map(|w| w[1] - w[0]));
            }
            Arr {
                r: a.r - 1,
                c: a.c,
                d: out,
            }
        } else {
            let mut out = Arr {
                r: a.r,
                c: a.c.saturating_sub(1),
                d: Vec::new(),
            };
            for j in 1..a.c {
                for i in 0..a.r {
                    out.d.push(
                        a.at(i, j).copied().unwrap_or(0.0) - a.at(i, j - 1).copied().unwrap_or(0.0),
                    );
                }
            }
            out
        };
    }
    real_out(a)
}

fn f_dot(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 2, 2, "dot")?;
    let (a, b) = (real_arg(args, 0, "dot")?, real_arg(args, 1, "dot")?);
    if a.d.len() != b.d.len() {
        return err(
            "Abaco:size",
            format!(
                "`dot` needs vectors of equal length, got {} and {}",
                a.d.len(),
                b.d.len()
            ),
        );
    }
    if (a.r == 1 || a.c == 1) && (b.r == 1 || b.c == 1) {
        return num(a.d.iter().zip(&b.d).map(|(x, y)| x * y).sum());
    }
    if (a.r, a.c) != (b.r, b.c) {
        return err("Abaco:size", "`dot` needs arrays of the same size");
    }
    let p = zip(&a, &b, |x, y| x * y)?;
    real_out(reduce_dim(&p, if p.r == 1 { 2 } else { 1 }, |v| {
        v.iter().sum()
    }))
}

fn f_cross(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 2, 2, "cross")?;
    let (a, b) = (real_arg(args, 0, "cross")?, real_arg(args, 1, "cross")?);
    if a.d.len() != 3 || b.d.len() != 3 {
        return err(
            "Abaco:size",
            "`cross` needs two vectors with three elements",
        );
    }
    let (x, y) = (&a.d, &b.d);
    let v = vec![
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    one(shaped_like(v, &a))
}

fn f_trapz(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "trapz")?;
    let (x, y) = if args.len() == 2 {
        (
            Some(real_arg(args, 0, "trapz")?.d),
            real_arg(args, 1, "trapz")?,
        )
    } else {
        (None, real_arg(args, 0, "trapz")?)
    };
    if let Some(x) = &x {
        if x.len() != y.d.len().max(y.r) && x.len() != y.r && x.len() != y.d.len() {
            return err("Abaco:size", "`trapz` needs x and y of the same length");
        }
    }
    let f = |v: &[f64]| -> f64 {
        v.windows(2)
            .enumerate()
            .map(|(i, w)| {
                let h = match &x {
                    Some(x) => {
                        x.get(i + 1).copied().unwrap_or(0.0) - x.get(i).copied().unwrap_or(0.0)
                    }
                    None => 1.0,
                };
                h * (w[0] + w[1]) / 2.0
            })
            .sum()
    };
    real_out(reduce_dim(&y, if y.r == 1 { 2 } else { 1 }, f))
}

fn f_cumtrapz(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "cumtrapz")?;
    let (x, y) = if args.len() == 2 {
        (
            Some(real_arg(args, 0, "cumtrapz")?.d),
            real_arg(args, 1, "cumtrapz")?,
        )
    } else {
        (None, real_arg(args, 0, "cumtrapz")?)
    };
    let d = if y.r == 1 { 2 } else { 1 };
    real_out(scan_dim(&y, d, |v| {
        let mut s = 0.0;
        v.iter()
            .enumerate()
            .map(|(i, &yi)| {
                if i > 0 {
                    let h = match &x {
                        Some(x) => {
                            x.get(i).copied().unwrap_or(0.0) - x.get(i - 1).copied().unwrap_or(0.0)
                        }
                        None => 1.0,
                    };
                    s += h * (yi + v[i - 1]) / 2.0;
                }
                s
            })
            .collect()
    }))
}

// ---------- max / min ----------

fn extremum(args: &[Value], nargout: usize, name: &str, better: fn(f64, f64) -> bool) -> Out {
    arity(args, 1, 3, name)?;
    if args.len() >= 2 && !args[1].is_empty() {
        // two-argument form: element-wise with NaN ignored
        let (a, b) = (real_arg(args, 0, name)?, real_arg(args, 1, name)?);
        if args.len() == 3 {
            return err(
                "Abaco:arity",
                format!("`{name}(a, b, dim)` is not supported; use `{name}(a, [], dim)`"),
            );
        }
        return real_out(zip(&a, &b, |&x, &y| {
            if x.is_nan() {
                y
            } else if y.is_nan() || better(x, y) {
                x
            } else {
                y
            }
        })?);
    }
    let a = match args.first() {
        Some(Value::Cplx(z)) => z.map(|c| c.norm()),
        _ => real_arg(args, 0, name)?,
    };
    let Some(d) = dim_of(&a, args, 2, name)? else {
        return Ok(vec![Value::empty(), Value::empty()]);
    };
    let pickone = |v: &[f64]| -> (f64, f64) {
        let mut best = f64::NAN;
        let mut at = 0usize;
        for (i, &x) in v.iter().enumerate() {
            if best.is_nan() && !x.is_nan() || (!x.is_nan() && better(x, best)) {
                best = x;
                at = i;
            }
        }
        (best, (at + 1) as f64)
    };
    if a.r == 0 || a.c == 0 {
        return Ok(vec![Value::empty(), Value::empty()]);
    }
    let pairs = reduce_dim(&a, d, pickone);
    let vals = pairs.map(|p| p.0);
    let vals = match args.first() {
        // complex maximum reports the element, not its modulus
        Some(Value::Cplx(z)) => {
            let idx = pairs.map(|p| p.1 as usize - 1);
            let zr = reduce_dim(z, d, |v| v.first().copied().unwrap_or_default());
            let _ = zr;
            let mut out = Vec::new();
            for (k, &i) in idx.d.iter().enumerate() {
                let col = if d == 1 { z.column(k) } else { z.row_vec(k) };
                out.push(col.get(i).copied().unwrap_or_default());
            }
            let r = Arr {
                r: idx.r,
                c: idx.c,
                d: out,
            };
            return Ok(if nargout > 1 {
                vec![Value::cplx(r), Value::Num(pairs.map(|p| p.1))]
            } else {
                vec![Value::cplx(r)]
            });
        }
        _ => vals,
    };
    if nargout > 1 {
        Ok(vec![Value::Num(vals), Value::Num(pairs.map(|p| p.1))])
    } else {
        one(Value::Num(vals))
    }
}

fn f_max(_: &mut Interp, a: &[Value], n: usize) -> Out {
    extremum(a, n, "max", |x, y| x > y)
}

fn f_min(_: &mut Interp, a: &[Value], n: usize) -> Out {
    extremum(a, n, "min", |x, y| x < y)
}

// ---------- logical reductions ----------

fn logic_reduce(args: &[Value], name: &str, all: bool) -> Out {
    arity(args, 1, 2, name)?;
    let a = match args.first() {
        Some(Value::Cplx(z)) => z.map(|c| f64::from(u8::from(c.re != 0.0 || c.im != 0.0))),
        _ => real_arg(args, 0, name)?,
    };
    let f = |v: &[f64]| {
        if all {
            v.iter().all(|&x| x != 0.0)
        } else {
            v.iter().any(|&x| x != 0.0)
        }
    };
    match dim_of(&a, args, 1, name)? {
        None => bool_arr(Arr::scalar(all)),
        Some(d) => bool_arr(reduce_dim(&a, d, f)),
    }
}

fn f_any(_: &mut Interp, a: &[Value], _: usize) -> Out {
    logic_reduce(a, "any", false)
}

fn f_all(_: &mut Interp, a: &[Value], _: usize) -> Out {
    logic_reduce(a, "all", true)
}

fn f_xor(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 2, 2, "xor")?;
    let (a, b) = (real_arg(args, 0, "xor")?, real_arg(args, 1, "xor")?);
    bool_arr(zip(&a, &b, |&x, &y| (x != 0.0) != (y != 0.0))?)
}

fn f_find(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 3, "find")?;
    let a = real_arg(args, 0, "find")?;
    let limit = if args.len() > 1 {
        usize_arg(args, 1, "find")?
    } else {
        usize::MAX
    };
    let from_end = is_flag(args, 2, "last");
    let mut hits: Vec<usize> =
        a.d.iter()
            .enumerate()
            .filter(|(_, &x)| x != 0.0 && !x.is_nan() || x.is_nan())
            .filter(|(_, &x)| x != 0.0)
            .map(|(i, _)| i)
            .collect();
    if from_end {
        let skip = hits.len().saturating_sub(limit);
        hits = hits.split_off(skip);
    } else {
        hits.truncate(limit);
    }
    let mk = |v: Vec<f64>| {
        if a.r == 1 && a.c != 1 {
            Value::row(v)
        } else {
            Value::col(v)
        }
    };
    if nargout >= 2 {
        let r = hits
            .iter()
            .map(|&i| (i % a.r.max(1)) as f64 + 1.0)
            .collect();
        let c = hits
            .iter()
            .map(|&i| (i / a.r.max(1)) as f64 + 1.0)
            .collect();
        let v = hits
            .iter()
            .map(|&i| a.d.get(i).copied().unwrap_or(0.0))
            .collect();
        return Ok(vec![mk(r), mk(c), mk(v)]);
    }
    let v: Vec<f64> = hits.iter().map(|&i| i as f64 + 1.0).collect();
    if a.r == 0 && a.c == 0 {
        return one(Value::empty());
    }
    one(mk(v))
}

// ---------- sorting ----------

fn cmp_asc(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

fn sort_perm(v: &[f64], desc: bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&i, &j| {
        let (a, b) = (
            v.get(i).copied().unwrap_or(f64::NAN),
            v.get(j).copied().unwrap_or(f64::NAN),
        );
        if desc {
            match (a.is_nan(), b.is_nan()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => b.partial_cmp(&a).unwrap_or(Ordering::Equal),
            }
        } else {
            cmp_asc(a, b)
        }
    });
    idx
}

fn f_sort(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 3, "sort")?;
    let desc = args
        .iter()
        .skip(1)
        .any(|a| matches!(a, Value::Str(s) if s.eq_ignore_ascii_case("descend")));
    if let Value::Cell(c) = &args[0] {
        let mut keys: Vec<(usize, String)> = Vec::new();
        for (i, v) in c.d.iter().enumerate() {
            match v {
                Value::Str(s) => keys.push((i, s.clone())),
                _ => {
                    return err(
                        "Abaco:type",
                        "`sort` of a cell needs every element to be text",
                    )
                }
            }
        }
        keys.sort_by(|a, b| if desc { b.1.cmp(&a.1) } else { a.1.cmp(&b.1) });
        let sorted = Arr {
            r: c.r,
            c: c.c,
            d: keys.iter().map(|(_, s)| Value::Str(s.clone())).collect(),
        };
        let idx = Arr {
            r: c.r,
            c: c.c,
            d: keys.iter().map(|(i, _)| *i as f64 + 1.0).collect(),
        };
        return Ok(vec![Value::Cell(sorted), Value::Num(idx)]);
    }
    if let Value::Str(s) = &args[0] {
        let mut chars: Vec<char> = s.chars().collect();
        chars.sort();
        if desc {
            chars.reverse();
        }
        return one(Value::Str(chars.into_iter().collect()));
    }
    let a = real_arg(args, 0, "sort")?;
    let dim_arg = args.iter().skip(1).find(|v| !matches!(v, Value::Str(_)));
    let d = match dim_arg {
        Some(v) => v.scalar_usize("the dimension")?,
        None => {
            if a.r == 1 {
                2
            } else {
                1
            }
        }
    };
    let (mut vals, mut idxs) = (a.clone(), a.clone());
    let lines = if d == 1 { a.c } else { a.r };
    for k in 0..lines {
        let line = if d == 1 { a.column(k) } else { a.row_vec(k) };
        let p = sort_perm(&line, desc);
        for (pos, &src) in p.iter().enumerate() {
            let (i, j) = if d == 1 { (pos, k) } else { (k, pos) };
            if let Some(s) = vals.d.get_mut(j * a.r + i) {
                *s = line.get(src).copied().unwrap_or(f64::NAN);
            }
            if let Some(s) = idxs.d.get_mut(j * a.r + i) {
                *s = src as f64 + 1.0;
            }
        }
    }
    if nargout > 1 {
        Ok(vec![Value::Num(vals), Value::Num(idxs)])
    } else {
        one(Value::Num(vals))
    }
}

fn f_sortrows(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 2, "sortrows")?;
    let a = real_arg(args, 0, "sortrows")?;
    let cols: Vec<(usize, bool)> = match args.get(1) {
        Some(v) => v
            .to_real()?
            .d
            .iter()
            .map(|&c| ((c.abs() as usize).saturating_sub(1), c < 0.0))
            .collect(),
        None => (0..a.c).map(|c| (c, false)).collect(),
    };
    let mut idx: Vec<usize> = (0..a.r).collect();
    idx.sort_by(|&p, &q| {
        for &(c, desc) in &cols {
            let (x, y) = (
                a.at(p, c).copied().unwrap_or(f64::NAN),
                a.at(q, c).copied().unwrap_or(f64::NAN),
            );
            let o = cmp_asc(x, y);
            if o != Ordering::Equal {
                return if desc { o.reverse() } else { o };
            }
        }
        Ordering::Equal
    });
    let mut out = a.clone();
    for (pos, &src) in idx.iter().enumerate() {
        for j in 0..a.c {
            if let Some(s) = out.d.get_mut(j * a.r + pos) {
                *s = a.at(src, j).copied().unwrap_or(f64::NAN);
            }
        }
    }
    let order = Value::col(idx.iter().map(|&i| i as f64 + 1.0).collect());
    if nargout > 1 {
        Ok(vec![Value::Num(out), order])
    } else {
        one(Value::Num(out))
    }
}

fn f_issorted(_: &mut Interp, args: &[Value], _: usize) -> Out {
    let a = real_arg(args, 0, "issorted")?;
    super::boolean(
        a.d.windows(2)
            .all(|w| cmp_asc(w[0], w[1]) != Ordering::Greater),
    )
}

fn f_unique(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 2, "unique")?;
    let stable = is_flag(args, 1, "stable");
    if let Value::Cell(c) = &args[0] {
        let mut seen: Vec<(String, usize)> = Vec::new();
        for (i, v) in c.d.iter().enumerate() {
            let Value::Str(s) = v else {
                return err(
                    "Abaco:type",
                    "`unique` of a cell needs every element to be text",
                );
            };
            if !seen.iter().any(|(t, _)| t == s) {
                seen.push((s.clone(), i));
            }
        }
        if !stable {
            seen.sort_by(|a, b| a.0.cmp(&b.0));
        }
        let items: Vec<Value> = seen.iter().map(|(s, _)| Value::Str(s.clone())).collect();
        let idx: Vec<f64> = seen.iter().map(|(_, i)| *i as f64 + 1.0).collect();
        return Ok(vec![Value::Cell(Arr::col(items)), Value::col(idx)]);
    }
    let a = real_arg(args, 0, "unique")?;
    let order = sort_perm(&a.d, false);
    let mut groups: Vec<(f64, usize, Vec<usize>)> = Vec::new();
    for &i in &order {
        let x = a.d.get(i).copied().unwrap_or(f64::NAN);
        match groups.last_mut() {
            Some(g) if g.0 == x => {
                g.1 = g.1.max(i);
                g.2.push(i);
            }
            _ => groups.push((x, i, vec![i])),
        }
    }
    // MATLAB reports the last occurrence by default; "first" is also common. Use first for stable order.
    let mut firsts: Vec<(f64, usize, Vec<usize>)> = groups
        .iter()
        .map(|(x, _, members)| {
            (
                *x,
                members.iter().copied().min().unwrap_or(0),
                members.clone(),
            )
        })
        .collect();
    if stable {
        firsts.sort_by_key(|g| g.1);
    }
    let vals: Vec<f64> = firsts.iter().map(|g| g.0).collect();
    let ia: Vec<f64> = groups
        .iter()
        .zip(&firsts)
        .map(|(g, f)| {
            if stable {
                f.1 as f64 + 1.0
            } else {
                g.1 as f64 + 1.0
            }
        })
        .collect();
    let mut ic = vec![0.0; a.d.len()];
    for (k, g) in firsts.iter().enumerate() {
        for &m in &g.2 {
            if let Some(s) = ic.get_mut(m) {
                *s = k as f64 + 1.0;
            }
        }
    }
    let mk = |v: Vec<f64>| {
        if a.r == 1 && a.c >= 1 {
            Value::row(v)
        } else {
            Value::col(v)
        }
    };
    if nargout >= 2 {
        Ok(vec![mk(vals), Value::col(ia), Value::col(ic)])
    } else {
        one(mk(vals))
    }
}

fn f_histc(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 2, 2, "histc")?;
    let (x, edges) = (real_arg(args, 0, "histc")?, real_arg(args, 1, "histc")?);
    let mut counts = vec![0.0; edges.d.len()];
    for &v in &x.d {
        if v.is_nan() {
            continue;
        }
        if let Some(last) = edges.d.last() {
            if v == *last {
                if let Some(c) = counts.last_mut() {
                    *c += 1.0;
                }
                continue;
            }
        }
        let k = edges.d.iter().rposition(|&e| e <= v);
        if let Some(k) = k {
            if k + 1 < edges.d.len() {
                counts[k] += 1.0;
            }
        }
    }
    one(shaped_like(counts, &edges))
}

fn f_cov(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "cov")?;
    let a = real_arg(args, 0, "cov")?;
    if args.len() == 2 {
        let b = real_arg(args, 1, "cov")?;
        if a.d.len() != b.d.len() {
            return err("Abaco:size", "`cov` needs vectors of the same length");
        }
        let n = a.d.len() as f64;
        let (ma, mb) = (a.d.iter().sum::<f64>() / n, b.d.iter().sum::<f64>() / n);
        let s: f64 = a.d.iter().zip(&b.d).map(|(x, y)| (x - ma) * (y - mb)).sum();
        return num(s / (n - 1.0).max(1.0));
    }
    if a.r == 1 || a.c == 1 {
        return num(variance(&a.d, 0.0));
    }
    let (n, k) = (a.r as f64, a.c);
    let means: Vec<f64> = (0..k)
        .map(|j| a.column(j).iter().sum::<f64>() / n)
        .collect();
    let mut d = vec![0.0; k * k];
    for p in 0..k {
        for q in 0..k {
            let s: f64 = (0..a.r)
                .map(|i| {
                    (a.at(i, p).copied().unwrap_or(0.0) - means[p])
                        * (a.at(i, q).copied().unwrap_or(0.0) - means[q])
                })
                .sum();
            d[q * k + p] = s / (n - 1.0).max(1.0);
        }
    }
    one(Value::mat(k, k, d))
}

fn f_corrcoef(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "corrcoef")?;
    let (a, b) = if args.len() == 2 {
        (
            real_arg(args, 0, "corrcoef")?,
            Some(real_arg(args, 1, "corrcoef")?),
        )
    } else {
        (real_arg(args, 0, "corrcoef")?, None)
    };
    let corr = |x: &[f64], y: &[f64]| -> f64 {
        let n = x.len() as f64;
        let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
        let sxy: f64 = x.iter().zip(y).map(|(p, q)| (p - mx) * (q - my)).sum();
        let sxx: f64 = x.iter().map(|p| (p - mx) * (p - mx)).sum();
        let syy: f64 = y.iter().map(|q| (q - my) * (q - my)).sum();
        sxy / (sxx * syy).sqrt()
    };
    match b {
        Some(b) => {
            if a.d.len() != b.d.len() {
                return err("Abaco:size", "`corrcoef` needs vectors of the same length");
            }
            let r = corr(&a.d, &b.d);
            one(Value::mat(2, 2, vec![1.0, r, r, 1.0]))
        }
        None => {
            let k = a.c;
            let cols: Vec<Vec<f64>> = (0..k).map(|j| a.column(j)).collect();
            let mut d = vec![0.0; k * k];
            for p in 0..k {
                for q in 0..k {
                    d[q * k + p] = if p == q {
                        1.0
                    } else {
                        corr(&cols[p], &cols[q])
                    };
                }
            }
            one(Value::mat(k, k, d))
        }
    }
}

fn f_prctile(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 2, 2, "prctile")?;
    let x = sorted_no_nan(&real_arg(args, 0, "prctile")?.d);
    let p = real_arg(args, 1, "prctile")?;
    if x.is_empty() {
        return real_out(p.map(|_| f64::NAN));
    }
    let n = x.len() as f64;
    real_out(p.map(|&q| {
        // the same definition MATLAB uses: piecewise linear through (k-0.5)/n
        let pos = q / 100.0 * n + 0.5;
        if pos <= 1.0 {
            x[0]
        } else if pos >= n {
            x[x.len() - 1]
        } else {
            let lo = pos.floor() as usize;
            let frac = pos - lo as f64;
            x[lo - 1] + frac * (x[lo] - x[lo - 1])
        }
    }))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("sum", f_sum),
    ("prod", f_prod),
    ("mean", f_mean),
    ("median", f_median),
    ("mode", f_mode),
    ("var", f_var),
    ("std", f_std),
    ("range", f_range),
    ("nnz", f_nnz),
    ("cumsum", f_cumsum),
    ("cumprod", f_cumprod),
    ("cummax", f_cummax),
    ("cummin", f_cummin),
    ("diff", f_diff),
    ("dot", f_dot),
    ("cross", f_cross),
    ("trapz", f_trapz),
    ("cumtrapz", f_cumtrapz),
    ("max", f_max),
    ("min", f_min),
    ("any", f_any),
    ("all", f_all),
    ("xor", f_xor),
    ("find", f_find),
    ("sort", f_sort),
    ("sortrows", f_sortrows),
    ("issorted", f_issorted),
    ("unique", f_unique),
    ("histc", f_histc),
    ("cov", f_cov),
    ("corrcoef", f_corrcoef),
    ("prctile", f_prctile),
];
