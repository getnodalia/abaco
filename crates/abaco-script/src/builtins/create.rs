// SPDX-License-Identifier: MIT OR Apache-2.0
//! Constants, array construction and shape manipulation.

use super::{
    arg, arity, bool_arr, boolean, dims_from, num, one, real_arg, real_out, scalar_arg, size_args,
    usize_arg, Bf, Out,
};
use crate::error::{err, Result};
use crate::index;
use crate::interp::Interp;
use crate::value::{Arr, Value};
use num_complex::Complex64;

// ---------- constants ----------

fn filled(args: &[Value], name: &str, x: f64) -> Out {
    let (r, c) = dims_from(size_args(args), name)?;
    one(Value::mat(r, c, vec![x; r * c]))
}

fn f_pi(_: &mut Interp, a: &[Value], _: usize) -> Out {
    filled(a, "pi", std::f64::consts::PI)
}

fn f_e(_: &mut Interp, a: &[Value], _: usize) -> Out {
    filled(a, "e", std::f64::consts::E)
}

fn f_inf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    filled(a, "Inf", f64::INFINITY)
}

fn f_nan(_: &mut Interp, a: &[Value], _: usize) -> Out {
    filled(a, "NaN", f64::NAN)
}

fn f_eps(_: &mut Interp, a: &[Value], _: usize) -> Out {
    match a.first() {
        Some(Value::Num(x)) => real_out(x.map(|&v| {
            let v = v.abs();
            if !v.is_finite() {
                f64::NAN
            } else if v < f64::MIN_POSITIVE {
                f64::MIN_POSITIVE * f64::EPSILON
            } else {
                let e = v.log2().floor();
                2f64.powf(e - 52.0)
            }
        })),
        _ => num(f64::EPSILON),
    }
}

fn f_imag_unit(_: &mut Interp, _: &[Value], _: usize) -> Out {
    one(Value::Cplx(Arr::scalar(Complex64::new(0.0, 1.0))))
}

fn f_realmax(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(f64::MAX)
}

fn f_realmin(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(f64::MIN_POSITIVE)
}

fn f_flintmax(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(9_007_199_254_740_992.0)
}

fn f_true(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = dims_from(a, "true")?;
    bool_arr(Arr::filled(r, c, true))
}

fn f_false(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = dims_from(a, "false")?;
    bool_arr(Arr::filled(r, c, false))
}

// ---------- construction ----------

fn f_zeros(_: &mut Interp, a: &[Value], _: usize) -> Out {
    filled(a, "zeros", 0.0)
}

fn f_ones(_: &mut Interp, a: &[Value], _: usize) -> Out {
    filled(a, "ones", 1.0)
}

fn f_eye(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = dims_from(size_args(a), "eye")?;
    let mut d = vec![0.0; r * c];
    for i in 0..r.min(c) {
        if let Some(x) = d.get_mut(i * r + i) {
            *x = 1.0;
        }
    }
    one(Value::mat(r, c, d))
}

fn f_rand(i: &mut Interp, a: &[Value], _: usize) -> Out {
    if matches!(a.first(), Some(Value::Str(_))) {
        // rand('seed', n) / rand('state', n)
        if let Some(s) = a.get(1) {
            i.rng = crate::interp::Rng::new(s.scalar_f64("the seed")? as u64);
        }
        return Ok(vec![]);
    }
    let (r, c) = dims_from(a, "rand")?;
    let d: Vec<f64> = (0..r * c).map(|_| i.rng.uniform()).collect();
    one(Value::mat(r, c, d))
}

fn f_randn(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = dims_from(size_args(a), "randn")?;
    let d: Vec<f64> = (0..r * c).map(|_| i.rng.normal()).collect();
    one(Value::mat(r, c, d))
}

fn f_randi(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 4, "randi")?;
    let lim = real_arg(a, 0, "randi")?;
    let (lo, hi) = match lim.d.as_slice() {
        [h] => (1.0, *h),
        [l, h] => (*l, *h),
        _ => {
            return err(
                "Abaco:type",
                "`randi` needs a maximum or a [low high] range",
            )
        }
    };
    if lo.fract() != 0.0 || hi.fract() != 0.0 || hi < lo {
        return err(
            "Abaco:domain",
            "`randi` needs whole-number limits with low <= high",
        );
    }
    let (r, c) = dims_from(&a[1..], "randi")?;
    let span = hi - lo + 1.0;
    let d: Vec<f64> = (0..r * c)
        .map(|_| lo + (i.rng.uniform() * span).floor())
        .collect();
    one(Value::mat(r, c, d))
}

fn f_randperm(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let n = usize_arg(a, 0, "randperm")?;
    if n > 10_000_000 {
        return err("Abaco:limit", "`randperm` is limited to 10 million");
    }
    let mut v: Vec<f64> = (1..=n).map(|k| k as f64).collect();
    for k in (1..n).rev() {
        let j = (i.rng.uniform() * (k + 1) as f64) as usize;
        v.swap(k, j.min(k));
    }
    one(Value::row(v))
}

fn f_rng(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let seed = match a.first() {
        Some(Value::Str(s)) if s == "default" => 0x5EED,
        Some(Value::Str(_)) => 0x5EED_u64,
        Some(v) => v.scalar_f64("the seed")?.abs() as u64,
        None => 0x5EED,
    };
    i.rng = crate::interp::Rng::new(seed);
    Ok(vec![])
}

fn f_linspace(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "linspace")?;
    let (lo, hi) = (scalar_arg(a, 0, "linspace")?, scalar_arg(a, 1, "linspace")?);
    let n = if a.len() > 2 {
        usize_arg(a, 2, "linspace")?
    } else {
        100
    };
    if n > 50_000_000 {
        return err("Abaco:limit", "`linspace` is limited to 50 million points");
    }
    let v: Vec<f64> = match n {
        0 => Vec::new(),
        1 => vec![hi],
        _ => (0..n)
            .map(|k| {
                if k == n - 1 {
                    hi
                } else {
                    lo + (hi - lo) * k as f64 / (n - 1) as f64
                }
            })
            .collect(),
    };
    one(Value::row(v))
}

fn f_logspace(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "logspace")?;
    let (lo, hi) = (scalar_arg(a, 0, "logspace")?, scalar_arg(a, 1, "logspace")?);
    let n = if a.len() > 2 {
        usize_arg(a, 2, "logspace")?
    } else {
        50
    };
    if n > 50_000_000 {
        return err("Abaco:limit", "`logspace` is limited to 50 million points");
    }
    let v: Vec<f64> = (0..n)
        .map(|k| {
            10f64.powf(if n == 1 {
                hi
            } else {
                lo + (hi - lo) * k as f64 / (n - 1) as f64
            })
        })
        .collect();
    one(Value::row(v))
}

fn f_colon(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "colon")?;
    let (s, e) = (
        scalar_arg(a, 0, "colon")?,
        scalar_arg(a, a.len() - 1, "colon")?,
    );
    let step = if a.len() == 3 {
        scalar_arg(a, 1, "colon")?
    } else {
        1.0
    };
    one(crate::interp::make_range(s, step, e)?)
}

fn f_meshgrid(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "meshgrid")?;
    let x = real_arg(a, 0, "meshgrid")?.d;
    let y = if a.len() > 1 {
        real_arg(a, 1, "meshgrid")?.d
    } else {
        x.clone()
    };
    let (nx, ny) = (x.len(), y.len());
    if nx.saturating_mul(ny) > crate::ops::MAX_ELEMENTS {
        return err("Abaco:limit", "`meshgrid` would produce too many points");
    }
    let mut xx = Vec::with_capacity(nx * ny);
    let mut yy = Vec::with_capacity(nx * ny);
    for j in 0..nx {
        for i in 0..ny {
            xx.push(x.get(j).copied().unwrap_or(0.0));
            yy.push(y.get(i).copied().unwrap_or(0.0));
        }
    }
    Ok(vec![Value::mat(ny, nx, xx), Value::mat(ny, nx, yy)])
}

fn f_ndgrid(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "ndgrid")?;
    let x = real_arg(a, 0, "ndgrid")?.d;
    let y = if a.len() > 1 {
        real_arg(a, 1, "ndgrid")?.d
    } else {
        x.clone()
    };
    let (nx, ny) = (x.len(), y.len());
    if nx.saturating_mul(ny) > crate::ops::MAX_ELEMENTS {
        return err("Abaco:limit", "`ndgrid` would produce too many points");
    }
    let mut xx = Vec::with_capacity(nx * ny);
    let mut yy = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            xx.push(x.get(i).copied().unwrap_or(0.0));
            yy.push(y.get(j).copied().unwrap_or(0.0));
        }
    }
    Ok(vec![Value::mat(nx, ny, xx), Value::mat(nx, ny, yy)])
}

// ---------- gathering elements into a new shape ----------

fn pick<T: Clone + Default>(a: &Arr<T>, r: usize, c: usize, src: &[usize]) -> Arr<T> {
    Arr {
        r,
        c,
        d: src
            .iter()
            .map(|&i| a.d.get(i).cloned().unwrap_or_default())
            .collect(),
    }
}

/// Builds an `r`×`c` value whose element `k` is element `src[k]` of `v`.
pub(crate) fn gather(v: &Value, r: usize, c: usize, src: &[usize]) -> Result<Value> {
    Ok(match v {
        Value::Num(a) => Value::Num(pick(a, r, c, src)),
        Value::Cplx(a) => Value::Cplx(pick(a, r, c, src)),
        Value::Bool(a) => Value::Bool(pick(a, r, c, src)),
        Value::Cell(a) => Value::Cell(pick(a, r, c, src)),
        Value::Str(s) => {
            let chars: Vec<char> = s.chars().collect();
            if r <= 1 || c == 0 {
                Value::Str(src.iter().filter_map(|&i| chars.get(i).copied()).collect())
            } else {
                Value::Num(Arr {
                    r,
                    c,
                    d: src
                        .iter()
                        .map(|&i| chars.get(i).map_or(0.0, |&ch| f64::from(u32::from(ch))))
                        .collect(),
                })
            }
        }
        other => {
            return err(
                "Abaco:type",
                format!("this operation does not work on {}", other.describe()),
            )
        }
    })
}

fn f_reshape(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 4, "reshape")?;
    let v = arg(a, 0, "reshape")?;
    let n = v.numel();
    let dims: Vec<Option<usize>> = if a.len() == 2 {
        real_arg(a, 1, "reshape")?
            .d
            .iter()
            .map(|&x| Some(x.max(0.0) as usize))
            .collect()
    } else {
        a[1..]
            .iter()
            .map(|d| {
                if d.is_empty() {
                    Ok(None)
                } else {
                    d.scalar_usize("a dimension").map(Some)
                }
            })
            .collect::<Result<_>>()?
    };
    let known: usize = dims.iter().flatten().product();
    let unknown = dims.iter().filter(|d| d.is_none()).count();
    if unknown > 1 {
        return err("Abaco:size", "`reshape` can leave only one dimension empty");
    }
    let dims: Vec<usize> = dims
        .iter()
        .map(|d| match d {
            Some(x) => Ok(*x),
            None if known == 0 || n % known != 0 => err(
                "Abaco:size",
                format!("cannot reshape {n} elements into the requested shape"),
            ),
            None => Ok(n / known),
        })
        .collect::<Result<_>>()?;
    let (r, c) = match dims.as_slice() {
        [r, c] => (*r, *c),
        [r, c, rest @ ..] if rest.iter().all(|&k| k == 1) => (*r, *c),
        _ => {
            return err(
                "Abaco:size",
                "`reshape` supports two-dimensional results only",
            )
        }
    };
    if r * c != n {
        return err(
            "Abaco:size",
            format!("cannot reshape {n} elements into {r}x{c}"),
        );
    }
    let src: Vec<usize> = (0..n).collect();
    one(gather(v, r, c, &src)?)
}

fn f_repmat(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "repmat")?;
    let v = arg(a, 0, "repmat")?;
    let (m, n) = dims_from(&a[1..], "repmat")?;
    let (r, c) = v.dims();
    let (rr, cc) = (r * m, c * n);
    if rr.saturating_mul(cc) > crate::ops::MAX_ELEMENTS {
        return err("Abaco:limit", "`repmat` would produce too many elements");
    }
    let mut src = Vec::with_capacity(rr * cc);
    for j in 0..cc {
        for i in 0..rr {
            src.push((j % c.max(1)) * r + (i % r.max(1)));
        }
    }
    one(gather(v, rr, cc, &src)?)
}

fn flip_dim(v: &Value, dim: usize) -> Result<Value> {
    let (r, c) = v.dims();
    let mut src = Vec::with_capacity(r * c);
    for j in 0..c {
        for i in 0..r {
            let (si, sj) = if dim == 1 {
                (r - 1 - i, j)
            } else {
                (i, c - 1 - j)
            };
            src.push(sj * r + si);
        }
    }
    gather(v, r, c, &src)
}

fn f_fliplr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "fliplr")?;
    one(flip_dim(arg(a, 0, "fliplr")?, 2)?)
}

fn f_flipud(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "flipud")?;
    one(flip_dim(arg(a, 0, "flipud")?, 1)?)
}

fn f_flip(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "flip")?;
    let v = arg(a, 0, "flip")?;
    let dim = super::pick_dim(a, 1, v.dims().0, "flip")?;
    one(flip_dim(v, dim.min(2))?)
}

fn f_circshift(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "circshift")?;
    let v = arg(a, 0, "circshift")?;
    let (r, c) = v.dims();
    let k = real_arg(a, 1, "circshift")?;
    let (sr, sc) = match (a.len(), k.d.as_slice()) {
        (3, [s]) => {
            if usize_arg(a, 2, "circshift")? == 2 {
                (0, *s as i64)
            } else {
                (*s as i64, 0)
            }
        }
        (_, [s]) => {
            if r == 1 {
                (0, *s as i64)
            } else {
                (*s as i64, 0)
            }
        }
        (_, [s, t, ..]) => (*s as i64, *t as i64),
        _ => (0, 0),
    };
    let mut src = Vec::with_capacity(r * c);
    for j in 0..c {
        for i in 0..r {
            let si = (i as i64 - sr).rem_euclid(r.max(1) as i64) as usize;
            let sj = (j as i64 - sc).rem_euclid(c.max(1) as i64) as usize;
            src.push(sj * r + si);
        }
    }
    one(gather(v, r, c, &src)?)
}

fn f_rot90(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "rot90")?;
    let v = arg(a, 0, "rot90")?;
    let k = if a.len() > 1 {
        scalar_arg(a, 1, "rot90")? as i64
    } else {
        1
    }
    .rem_euclid(4);
    let (r, c) = v.dims();
    let (rr, cc) = if k % 2 == 1 { (c, r) } else { (r, c) };
    let mut src = Vec::with_capacity(r * c);
    for j in 0..cc {
        for i in 0..rr {
            let (si, sj) = match k {
                0 => (i, j),
                1 => (j, c - 1 - i),
                2 => (r - 1 - i, c - 1 - j),
                _ => (r - 1 - j, i),
            };
            src.push(sj * r + si);
        }
    }
    one(gather(v, rr, cc, &src)?)
}

fn f_diag(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "diag")?;
    let k = if a.len() > 1 {
        scalar_arg(a, 1, "diag")? as i64
    } else {
        0
    };
    let v = real_arg(a, 0, "diag")?;
    if v.r == 1 || v.c == 1 {
        let n = v.d.len() + k.unsigned_abs() as usize;
        if n.saturating_mul(n) > crate::ops::MAX_ELEMENTS {
            return err("Abaco:limit", "`diag` would produce too many elements");
        }
        let mut d = vec![0.0; n * n];
        for (t, x) in v.d.iter().enumerate() {
            let (i, j) = if k >= 0 {
                (t, t + k as usize)
            } else {
                (t + (-k) as usize, t)
            };
            if let Some(s) = d.get_mut(j * n + i) {
                *s = *x;
            }
        }
        one(Value::mat(n, n, d))
    } else {
        let mut out = Vec::new();
        let mut i = if k < 0 { (-k) as usize } else { 0 };
        let mut j = if k > 0 { k as usize } else { 0 };
        while i < v.r && j < v.c {
            out.push(v.at(i, j).copied().unwrap_or(0.0));
            i += 1;
            j += 1;
        }
        one(Value::col(out))
    }
}

fn tri(a: &[Value], name: &str, upper: bool) -> Out {
    arity(a, 1, 2, name)?;
    let k = if a.len() > 1 {
        scalar_arg(a, 1, name)? as i64
    } else {
        0
    };
    let m = real_arg(a, 0, name)?;
    let mut out = m.clone();
    for j in 0..m.c {
        for i in 0..m.r {
            let d = j as i64 - i as i64;
            let keep = if upper { d >= k } else { d <= k };
            if !keep {
                if let Some(x) = out.d.get_mut(j * m.r + i) {
                    *x = 0.0;
                }
            }
        }
    }
    real_out(out)
}

fn f_triu(_: &mut Interp, a: &[Value], _: usize) -> Out {
    tri(a, "triu", true)
}

fn f_tril(_: &mut Interp, a: &[Value], _: usize) -> Out {
    tri(a, "tril", false)
}

fn f_kron(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "kron")?;
    let (x, y) = (real_arg(a, 0, "kron")?, real_arg(a, 1, "kron")?);
    let (r, c) = (x.r * y.r, x.c * y.c);
    if r.saturating_mul(c) > crate::ops::MAX_ELEMENTS {
        return err("Abaco:limit", "`kron` would produce too many elements");
    }
    let mut d = vec![0.0; r * c];
    for j in 0..c {
        for i in 0..r {
            let p = x.at(i / y.r.max(1), j / y.c.max(1)).copied().unwrap_or(0.0);
            let q = y.at(i % y.r.max(1), j % y.c.max(1)).copied().unwrap_or(0.0);
            if let Some(s) = d.get_mut(j * r + i) {
                *s = p * q;
            }
        }
    }
    one(Value::mat(r, c, d))
}

fn f_horzcat(_: &mut Interp, a: &[Value], _: usize) -> Out {
    one(index::concat(a, true)?)
}

fn f_vertcat(_: &mut Interp, a: &[Value], _: usize) -> Out {
    one(index::concat(a, false)?)
}

fn f_cat(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let dim = usize_arg(a, 0, "cat")?;
    match dim {
        1 => one(index::concat(&a[1..], false)?),
        2 => one(index::concat(&a[1..], true)?),
        _ => err("Abaco:size", "`cat` supports dimensions 1 and 2"),
    }
}

fn magic(n: usize) -> Vec<Vec<f64>> {
    let nf = n as f64;
    let mut m = vec![vec![0.0; n]; n];
    if n % 2 == 1 {
        for i in 1..=n {
            for j in 1..=n {
                let a = ((i + j) as f64 - (nf + 3.0) / 2.0).rem_euclid(nf);
                let b = ((i + 2 * j) as f64 - 2.0).rem_euclid(nf);
                m[i - 1][j - 1] = nf * a + b + 1.0;
            }
        }
    } else if n.is_multiple_of(4) {
        for i in 1..=n {
            for j in 1..=n {
                let v = ((i - 1) * n + j) as f64;
                let k = (i % 4) / 2 == (j % 4) / 2;
                m[i - 1][j - 1] = if k { nf * nf + 1.0 - v } else { v };
            }
        }
    } else {
        let p = n / 2;
        let q = magic(p);
        let pf = p as f64;
        for i in 0..p {
            for j in 0..p {
                let base = q[i][j];
                m[i][j] = base;
                m[i][j + p] = base + 2.0 * pf * pf;
                m[i + p][j] = base + 3.0 * pf * pf;
                m[i + p][j + p] = base + pf * pf;
            }
        }
        let k = (n - 2) / 4;
        let mut cols: Vec<usize> = (0..k).collect();
        cols.extend((n - k + 1)..n);
        for &j in &cols {
            for i in 0..p {
                let t = m[i][j];
                m[i][j] = m[i + p][j];
                m[i + p][j] = t;
            }
        }
        let i = k;
        for &j in &[0, i] {
            let t = m[i][j];
            m[i][j] = m[i + p][j];
            m[i + p][j] = t;
        }
    }
    m
}

fn f_magic(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let n = usize_arg(a, 0, "magic")?;
    if n > 2000 {
        return err("Abaco:limit", "`magic` is limited to 2000");
    }
    match n {
        0 => one(Value::empty()),
        1 => num(1.0),
        2 => one(Value::mat(2, 2, vec![4.0, 3.0, 1.0, 2.0])),
        _ => {
            let m = magic(n);
            let mut d = vec![0.0; n * n];
            for (i, row) in m.iter().enumerate() {
                for (j, x) in row.iter().enumerate() {
                    d[j * n + i] = *x;
                }
            }
            one(Value::mat(n, n, d))
        }
    }
}

fn f_toeplitz(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "toeplitz")?;
    let c = real_arg(a, 0, "toeplitz")?.d;
    let r = if a.len() > 1 {
        real_arg(a, 1, "toeplitz")?.d
    } else {
        c.clone()
    };
    let (m, n) = (c.len(), r.len());
    if m.saturating_mul(n) > crate::ops::MAX_ELEMENTS {
        return err("Abaco:limit", "`toeplitz` would produce too many elements");
    }
    let mut d = vec![0.0; m * n];
    for j in 0..n {
        for i in 0..m {
            d[j * m + i] = if i >= j {
                c.get(i - j).copied().unwrap_or(0.0)
            } else {
                r.get(j - i).copied().unwrap_or(0.0)
            };
        }
    }
    one(Value::mat(m, n, d))
}

fn f_vander(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "vander")?;
    let x = real_arg(a, 0, "vander")?.d;
    let n = if a.len() > 1 {
        usize_arg(a, 1, "vander")?
    } else {
        x.len()
    };
    let m = x.len();
    if m.saturating_mul(n) > crate::ops::MAX_ELEMENTS {
        return err("Abaco:limit", "`vander` would produce too many elements");
    }
    let mut d = vec![0.0; m * n];
    for j in 0..n {
        for i in 0..m {
            d[j * m + i] = x[i].powi((n - 1 - j) as i32);
        }
    }
    one(Value::mat(m, n, d))
}

fn f_hilb(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let n = usize_arg(a, 0, "hilb")?;
    if n > 3000 {
        return err("Abaco:limit", "`hilb` is limited to 3000");
    }
    let mut d = vec![0.0; n * n];
    for j in 0..n {
        for i in 0..n {
            d[j * n + i] = 1.0 / (i + j + 1) as f64;
        }
    }
    one(Value::mat(n, n, d))
}

// ---------- shape queries ----------

fn f_size(_: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 1, 2, "size")?;
    let (r, c) = arg(a, 0, "size")?.dims();
    if a.len() == 2 {
        let d = usize_arg(a, 1, "size")?;
        return num(match d {
            1 => r as f64,
            2 => c as f64,
            0 => return err("Abaco:size", "`size`: the dimension must be at least 1"),
            _ => 1.0,
        });
    }
    if nargout <= 1 {
        one(Value::row(vec![r as f64, c as f64]))
    } else {
        let mut out = vec![Value::num(r as f64), Value::num(c as f64)];
        out.resize(nargout, Value::num(1.0));
        Ok(out)
    }
}

fn f_numel(_: &mut Interp, a: &[Value], _: usize) -> Out {
    num(arg(a, 0, "numel")?.numel() as f64)
}

fn f_length(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = arg(a, 0, "length")?.dims();
    num(if r == 0 || c == 0 {
        0.0
    } else {
        r.max(c) as f64
    })
}

fn f_ndims(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(2.0)
}

fn f_isempty(_: &mut Interp, a: &[Value], _: usize) -> Out {
    boolean(arg(a, 0, "isempty")?.is_empty())
}

fn f_isscalar(_: &mut Interp, a: &[Value], _: usize) -> Out {
    boolean(arg(a, 0, "isscalar")?.dims() == (1, 1))
}

fn f_isvector(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = arg(a, 0, "isvector")?.dims();
    boolean((r == 1 && c >= 1) || (c == 1 && r >= 1))
}

fn f_isrow(_: &mut Interp, a: &[Value], _: usize) -> Out {
    boolean(arg(a, 0, "isrow")?.dims().0 == 1)
}

fn f_iscolumn(_: &mut Interp, a: &[Value], _: usize) -> Out {
    boolean(arg(a, 0, "iscolumn")?.dims().1 == 1)
}

fn f_ismatrix(_: &mut Interp, _: &[Value], _: usize) -> Out {
    boolean(true)
}

fn f_issquare(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = arg(a, 0, "issquare")?.dims();
    boolean(r == c)
}

fn f_cell(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let (r, c) = dims_from(a, "cell")?;
    one(Value::Cell(Arr::filled(r, c, Value::empty())))
}

fn f_squeeze(_: &mut Interp, a: &[Value], _: usize) -> Out {
    one(arg(a, 0, "squeeze")?.clone())
}

fn f_isequal_dims(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "sameSize")?;
    boolean(a[0].dims() == a[1].dims())
}

pub const TABLE: &[(&str, Bf)] = &[
    ("pi", f_pi),
    ("e", f_e),
    ("Inf", f_inf),
    ("inf", f_inf),
    ("NaN", f_nan),
    ("nan", f_nan),
    ("eps", f_eps),
    ("i", f_imag_unit),
    ("j", f_imag_unit),
    ("realmax", f_realmax),
    ("realmin", f_realmin),
    ("flintmax", f_flintmax),
    ("true", f_true),
    ("false", f_false),
    ("zeros", f_zeros),
    ("ones", f_ones),
    ("eye", f_eye),
    ("rand", f_rand),
    ("randn", f_randn),
    ("randi", f_randi),
    ("randperm", f_randperm),
    ("rng", f_rng),
    ("linspace", f_linspace),
    ("logspace", f_logspace),
    ("colon", f_colon),
    ("meshgrid", f_meshgrid),
    ("ndgrid", f_ndgrid),
    ("reshape", f_reshape),
    ("repmat", f_repmat),
    ("fliplr", f_fliplr),
    ("flipud", f_flipud),
    ("flip", f_flip),
    ("circshift", f_circshift),
    ("rot90", f_rot90),
    ("diag", f_diag),
    ("triu", f_triu),
    ("tril", f_tril),
    ("kron", f_kron),
    ("horzcat", f_horzcat),
    ("vertcat", f_vertcat),
    ("cat", f_cat),
    ("magic", f_magic),
    ("toeplitz", f_toeplitz),
    ("vander", f_vander),
    ("hilb", f_hilb),
    ("size", f_size),
    ("numel", f_numel),
    ("length", f_length),
    ("ndims", f_ndims),
    ("isempty", f_isempty),
    ("isscalar", f_isscalar),
    ("isvector", f_isvector),
    ("isrow", f_isrow),
    ("iscolumn", f_iscolumn),
    ("ismatrix", f_ismatrix),
    ("issquare", f_issquare),
    ("cell", f_cell),
    ("squeeze", f_squeeze),
    ("samesize", f_isequal_dims),
];
