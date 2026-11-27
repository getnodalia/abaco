// SPDX-License-Identifier: MIT OR Apache-2.0
//! Special matrices, `accumarray`, `perms`, `gradient` and `conv2`.

use super::{arity, one, real_arg, str_arg, usize_arg, Out};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::value::Value;

fn limit(name: &str) -> crate::error::Error {
    crate::error::Error::new(
        "Abaco:limit",
        format!("`{name}` would produce too many elements"),
    )
}

fn check_size(r: usize, c: usize, name: &str) -> Result<()> {
    if r.saturating_mul(c) > crate::ops::MAX_ELEMENTS {
        return Err(limit(name));
    }
    Ok(())
}

fn f_blkdiag(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let mut blocks = Vec::new();
    for i in 0..a.len() {
        blocks.push(real_arg(a, i, "blkdiag")?);
    }
    let rows: usize = blocks.iter().map(|b| b.r).sum();
    let cols: usize = blocks.iter().map(|b| b.c).sum();
    check_size(rows, cols, "blkdiag")?;
    let mut d = vec![0.0; rows * cols];
    let (mut r0, mut c0) = (0, 0);
    for b in &blocks {
        for j in 0..b.c {
            for i in 0..b.r {
                if let (Some(slot), Some(x)) =
                    (d.get_mut((c0 + j) * rows + r0 + i), b.d.get(j * b.r + i))
                {
                    *slot = *x;
                }
            }
        }
        r0 += b.r;
        c0 += b.c;
    }
    one(Value::mat(rows, cols, d))
}

fn f_hankel(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "hankel")?;
    let c = real_arg(a, 0, "hankel")?.d;
    let m = c.len();
    let r = if a.len() > 1 {
        real_arg(a, 1, "hankel")?.d
    } else {
        vec![0.0; m]
    };
    let n = r.len();
    check_size(m, n, "hankel")?;
    let mut d = vec![0.0; m * n];
    for j in 0..n {
        for i in 0..m {
            let k = i + j;
            d[j * m + i] = if k < m {
                c.get(k).copied().unwrap_or(0.0)
            } else {
                r.get(k + 1 - m).copied().unwrap_or(0.0)
            };
        }
    }
    one(Value::mat(m, n, d))
}

fn f_pascal(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "pascal")?;
    let n = usize_arg(a, 0, "pascal")?;
    if n > 1000 {
        return err("Abaco:limit", "`pascal` is limited to 1000");
    }
    let mut d = vec![1.0; n * n];
    for j in 1..n {
        for i in 1..n {
            d[j * n + i] = d[(j - 1) * n + i] + d[j * n + i - 1];
        }
    }
    one(Value::mat(n, n, d))
}

fn f_wilkinson(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "wilkinson")?;
    let n = usize_arg(a, 0, "wilkinson")?;
    if n > 3000 {
        return err("Abaco:limit", "`wilkinson` is limited to 3000");
    }
    let mut d = vec![0.0; n * n];
    for i in 0..n {
        d[i * n + i] = ((n as f64 - 1.0) / 2.0 - i as f64).abs();
        if i + 1 < n {
            d[(i + 1) * n + i] = 1.0;
            d[i * n + i + 1] = 1.0;
        }
    }
    one(Value::mat(n, n, d))
}

fn f_compan(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "compan")?;
    let c = real_arg(a, 0, "compan")?.d;
    let lead = c.iter().position(|&x| x != 0.0).unwrap_or(c.len());
    let c = &c[lead..];
    let n = c.len().saturating_sub(1);
    if n == 0 {
        return one(Value::mat(0, 0, vec![]));
    }
    check_size(n, n, "compan")?;
    let mut d = vec![0.0; n * n];
    for j in 0..n {
        d[j * n] = -c.get(j + 1).copied().unwrap_or(0.0) / c[0];
        if j + 1 < n {
            d[j * n + j + 1] = 1.0;
        }
    }
    one(Value::mat(n, n, d))
}

fn f_realsqrt(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "realsqrt")?;
    let x = real_arg(a, 0, "realsqrt")?;
    if x.d.iter().any(|v| *v < 0.0) {
        return err(
            "Abaco:domain",
            "`realsqrt` needs values that are not negative",
        );
    }
    one(Value::mat(x.r, x.c, x.d.iter().map(|v| v.sqrt()).collect()))
}

/// The rows of `perms(v)`: every ordering of the elements, last permutation of positions first.
fn f_perms(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "perms")?;
    let v = real_arg(a, 0, "perms")?.d;
    let n = v.len();
    if n > 10 {
        return err("Abaco:limit", "`perms` is limited to 10 elements");
    }
    let count: usize = (1..=n).product();
    check_size(count, n, "perms")?;
    // Permutations of positions in lexicographic order, then reversed.
    let mut idx: Vec<usize> = (0..n).collect();
    let mut all: Vec<Vec<usize>> = Vec::with_capacity(count);
    loop {
        all.push(idx.clone());
        // next lexicographic permutation
        let Some(i) = (1..n).rev().find(|&i| idx[i - 1] < idx[i]) else {
            break;
        };
        let Some(j) = (i..n).rev().find(|&j| idx[j] > idx[i - 1]) else {
            break;
        };
        idx.swap(i - 1, j);
        idx[i..].reverse();
    }
    all.reverse();
    let mut d = vec![0.0; count * n];
    for (r, p) in all.iter().enumerate() {
        for (c, &k) in p.iter().enumerate() {
            d[c * count + r] = v.get(k).copied().unwrap_or(0.0);
        }
    }
    one(Value::mat(count, n, d))
}

/// `accumarray(subs, vals)`: sums of `vals` by index. `subs` is a vector of positive integers, or
/// a matrix with a row and a column index per row. `vals` may be one number for every row.
fn f_accumarray(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "accumarray")?;
    let subs = real_arg(a, 0, "accumarray")?;
    let vals = real_arg(a, 1, "accumarray")?;
    let two = subs.c == 2 && subs.r != 0;
    let n = if two { subs.r } else { subs.d.len() };
    if vals.d.len() != n && vals.d.len() != 1 {
        return err(
            "Abaco:size",
            "`accumarray` needs one value per subscript, or a single value",
        );
    }
    let index = |x: f64| -> Result<usize> {
        if x.is_finite() && x >= 1.0 && x.fract() == 0.0 && x <= 1e7 {
            Ok(x as usize)
        } else {
            err(
                "Abaco:index",
                "`accumarray` subscripts must be positive integers",
            )
        }
    };
    let (mut rows, mut cols) = (0usize, 1usize);
    let mut pos = Vec::with_capacity(n);
    for k in 0..n {
        let (i, j) = if two {
            (
                index(subs.d.get(k).copied().unwrap_or(f64::NAN))?,
                index(subs.d.get(subs.r + k).copied().unwrap_or(f64::NAN))?,
            )
        } else {
            (index(subs.d.get(k).copied().unwrap_or(f64::NAN))?, 1)
        };
        rows = rows.max(i);
        cols = cols.max(j);
        pos.push((i - 1, j - 1));
    }
    check_size(rows, cols, "accumarray")?;
    let mut d = vec![0.0; rows * cols];
    for (k, (i, j)) in pos.into_iter().enumerate() {
        let x = if vals.d.len() == 1 {
            vals.d.first()
        } else {
            vals.d.get(k)
        };
        if let (Some(slot), Some(x)) = (d.get_mut(j * rows + i), x) {
            *slot += x;
        }
    }
    one(Value::mat(rows, cols, d))
}

/// One-dimensional derivative estimates: central differences inside, one-sided at the ends.
fn diff1(x: &[f64], h: f64) -> Vec<f64> {
    let n = x.len();
    (0..n)
        .map(|i| {
            if n < 2 {
                0.0
            } else if i == 0 {
                (x[1] - x[0]) / h
            } else if i == n - 1 {
                (x[n - 1] - x[n - 2]) / h
            } else {
                (x[i + 1] - x[i - 1]) / (2.0 * h)
            }
        })
        .collect()
}

/// `gradient(F)`, `gradient(F, h)`; with two outputs, the derivative along columns then rows.
fn f_gradient(_: &mut Interp, a: &[Value], nout: usize) -> Out {
    arity(a, 1, 2, "gradient")?;
    let f = real_arg(a, 0, "gradient")?;
    let h = if a.len() > 1 {
        let v = real_arg(a, 1, "gradient")?;
        if v.d.len() != 1 {
            return err("Abaco:argument", "`gradient` takes a single spacing");
        }
        v.d[0]
    } else {
        1.0
    };
    if !(h.is_finite() && h != 0.0) {
        return err(
            "Abaco:argument",
            "`gradient` needs a spacing that is finite and not zero",
        );
    }
    if f.r == 1 || f.c == 1 {
        return one(Value::mat(f.r, f.c, diff1(&f.d, h)));
    }
    let mut dx = vec![0.0; f.d.len()];
    let mut dy = vec![0.0; f.d.len()];
    for i in 0..f.r {
        let row: Vec<f64> = (0..f.c).map(|j| f.d[j * f.r + i]).collect();
        for (j, v) in diff1(&row, h).into_iter().enumerate() {
            dx[j * f.r + i] = v;
        }
    }
    for j in 0..f.c {
        let col = &f.d[j * f.r..(j + 1) * f.r];
        for (i, v) in diff1(col, h).into_iter().enumerate() {
            dy[j * f.r + i] = v;
        }
    }
    let mut out = vec![Value::mat(f.r, f.c, dx)];
    if nout > 1 {
        out.push(Value::mat(f.r, f.c, dy));
    }
    Ok(out)
}

/// `conv2(A, B[, shape])` with `full` (default), `same` or `valid`.
fn f_conv2(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "conv2")?;
    let (x, k) = (real_arg(a, 0, "conv2")?, real_arg(a, 1, "conv2")?);
    let shape = if a.len() > 2 {
        str_arg(a, 2, "conv2")?.to_ascii_lowercase()
    } else {
        "full".into()
    };
    if x.d.is_empty() || k.d.is_empty() {
        return one(Value::mat(0, 0, vec![]));
    }
    let (fr, fc) = (x.r + k.r - 1, x.c + k.c - 1);
    check_size(fr, fc, "conv2")?;
    let mut full = vec![0.0; fr * fc];
    for j in 0..x.c {
        for i in 0..x.r {
            let xv = x.d[j * x.r + i];
            if xv == 0.0 {
                continue;
            }
            for q in 0..k.c {
                for p in 0..k.r {
                    full[(j + q) * fr + i + p] += xv * k.d[q * k.r + p];
                }
            }
        }
    }
    let window = |r0: usize, c0: usize, rows: usize, cols: usize| {
        let mut d = Vec::with_capacity(rows * cols);
        for j in 0..cols {
            for i in 0..rows {
                d.push(full.get((c0 + j) * fr + r0 + i).copied().unwrap_or(0.0));
            }
        }
        Value::mat(rows, cols, d)
    };
    match shape.as_str() {
        "full" => one(Value::mat(fr, fc, full)),
        "same" => one(window(k.r / 2, k.c / 2, x.r, x.c)),
        "valid" => {
            if x.r < k.r || x.c < k.c {
                return one(Value::mat(0, 0, vec![]));
            }
            one(window(k.r - 1, k.c - 1, x.r - k.r + 1, x.c - k.c + 1))
        }
        other => err(
            "Abaco:argument",
            format!("`conv2` shape must be full, same or valid, not `{other}`"),
        ),
    }
}

pub const TABLE: &[(&str, super::Bf)] = &[
    ("accumarray", f_accumarray),
    ("blkdiag", f_blkdiag),
    ("compan", f_compan),
    ("conv2", f_conv2),
    ("gradient", f_gradient),
    ("hankel", f_hankel),
    ("pascal", f_pascal),
    ("perms", f_perms),
    ("realsqrt", f_realsqrt),
    ("wilkinson", f_wilkinson),
];
