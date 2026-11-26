// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interpolation: `interp1`, `interp2`, splines and piecewise polynomials.

use super::{arg, arity, one, real_arg, Bf, Out};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::linalg::num_err;
use crate::value::{Arr, Value};
use nodalia_numerics::interp::{self as ni, PiecewiseCubic};

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Linear,
    Nearest,
    Previous,
    Next,
    Pchip,
    Spline,
}

fn parse_method(s: &str) -> Result<Method> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "linear" => Method::Linear,
        "nearest" => Method::Nearest,
        "previous" => Method::Previous,
        "next" => Method::Next,
        "pchip" | "cubic" => Method::Pchip,
        "spline" => Method::Spline,
        other => {
            return err(
                "Abaco:domain",
                format!("`interp1`: unknown method '{other}' (linear, nearest, previous, next, pchip, cubic or spline)"),
            )
        }
    })
}

/// Sorts `x` ascending carrying the matching rows of `y`; rejects repeated points.
fn sorted_points(x: &[f64], ys: &[Vec<f64>]) -> Result<(Vec<f64>, Vec<Vec<f64>>)> {
    if x.iter().any(|v| !v.is_finite()) {
        return err(
            "Abaco:domain",
            "`interp1`: the sample points must be finite",
        );
    }
    let mut order: Vec<usize> = (0..x.len()).collect();
    order.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let xs: Vec<f64> = order.iter().map(|&i| x[i]).collect();
    if xs.windows(2).any(|w| w[1] <= w[0]) {
        return err(
            "Abaco:domain",
            "`interp1`: the sample points must be distinct",
        );
    }
    let yy = ys
        .iter()
        .map(|y| order.iter().map(|&i| y[i]).collect())
        .collect();
    Ok((xs, yy))
}

fn eval_one(m: Method, x: &[f64], y: &[f64], pp: Option<&PiecewiseCubic>, q: f64) -> f64 {
    if q.is_nan() {
        return f64::NAN;
    }
    match m {
        Method::Linear => ni::linear(x, y, q).unwrap_or(f64::NAN),
        Method::Pchip | Method::Spline => pp.map_or(f64::NAN, |p| p.eval(q)),
        Method::Nearest => {
            let i = x.partition_point(|&b| b <= q);
            if i == 0 {
                y[0]
            } else if i >= x.len() {
                y[x.len() - 1]
            } else if q - x[i - 1] < x[i] - q {
                y[i - 1]
            } else {
                y[i]
            }
        }
        Method::Previous => {
            let i = x.partition_point(|&b| b <= q);
            if i == 0 {
                y[0]
            } else {
                y[i - 1]
            }
        }
        Method::Next => {
            let i = x.partition_point(|&b| b < q);
            if i >= x.len() {
                y[x.len() - 1]
            } else {
                y[i]
            }
        }
    }
}

fn f_interp1(interp: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 6, "interp1")?;
    let n_data = a.iter().take_while(|v| !matches!(v, Value::Str(_))).count();
    let mut rest = a.get(n_data..).unwrap_or(&[]).to_vec();
    let (x, v, xq) = match n_data {
        2 => {
            let v = real_arg(a, 0, "interp1")?;
            let n = if v.r == 1 { v.c } else { v.r };
            (
                (1..=n).map(|k| k as f64).collect(),
                v,
                real_arg(a, 1, "interp1")?,
            )
        }
        3 | 4 => {
            // A trailing numeric after the query is the extrapolation value.
            if n_data == 4 {
                rest.insert(0, a[3].clone());
            }
            (
                real_arg(a, 0, "interp1")?.d,
                real_arg(a, 1, "interp1")?,
                real_arg(a, 2, "interp1")?,
            )
        }
        _ => return err("Abaco:arity", "`interp1` needs (x, v, xq) or (v, xq)"),
    };
    let mut method = Method::Linear;
    let mut extrap: Option<Option<f64>> = None; // Some(None) = extrapolate; Some(Some(v)) = constant
    for item in &rest {
        match item {
            Value::Str(s) if s.eq_ignore_ascii_case("extrap") => extrap = Some(None),
            Value::Str(s) => method = parse_method(s)?,
            other => extrap = Some(Some(other.scalar_f64("the extrapolation value")?)),
        }
    }
    let vector = v.r == 1 || v.c == 1;
    let cols: Vec<Vec<f64>> = if vector {
        vec![v.d.clone()]
    } else {
        (0..v.c).map(|j| v.column(j)).collect()
    };
    if cols.iter().any(|c| c.len() != x.len()) {
        return err(
            "Abaco:size",
            format!(
                "`interp1`: x has {} points but the values have {}",
                x.len(),
                if vector { v.d.len() } else { v.r }
            ),
        );
    }
    if x.len() < 2 {
        return err("Abaco:domain", "`interp1` needs at least two sample points");
    }
    let (xs, ys) = sorted_points(&x, &cols)?;
    interp.charge((xq.d.len() * ys.len()) as u64 / 8 + 1)?;
    let mut out = Vec::with_capacity(xq.d.len() * ys.len());
    for y in &ys {
        let pp = match method {
            Method::Pchip => Some(ni::pchip(&xs, y).map_err(num_err)?),
            Method::Spline => Some(ni::spline(&xs, y).map_err(num_err)?),
            _ => None,
        };
        let (lo, hi) = (xs[0], xs[xs.len() - 1]);
        for &q in &xq.d {
            let outside = q < lo || q > hi;
            let value = match (outside, extrap) {
                (true, None) => f64::NAN,
                (true, Some(Some(c))) => c,
                _ => eval_one(method, &xs, y, pp.as_ref(), q),
            };
            out.push(value);
        }
    }
    one(Value::Num(if vector {
        Arr::new(xq.r, xq.c, out)
    } else {
        Arr::new(xq.d.len(), ys.len(), out)
    }))
}

fn pp_value(p: &PiecewiseCubic) -> Value {
    let pieces = p.coefs.len();
    // MATLAB stores each piece's coefficients highest power first, one piece per row.
    let mut d = vec![0.0; pieces * 4];
    for (i, c) in p.coefs.iter().enumerate() {
        for k in 0..4 {
            d[k * pieces + i] = c[3 - k];
        }
    }
    Value::structure(vec![
        ("form", Value::text("pp")),
        ("breaks", Value::row(p.breaks.clone())),
        ("coefs", Value::mat(pieces, 4, d)),
        ("pieces", Value::num(pieces as f64)),
        ("order", Value::num(4.0)),
        ("dim", Value::num(1.0)),
    ])
}

fn pp_from_value(v: &Value) -> Result<PiecewiseCubic> {
    let Value::Struct(s) = v else {
        return err(
            "Abaco:type",
            "`ppval` needs a piecewise polynomial made by spline, pchip or mkpp",
        );
    };
    let breaks = s
        .get("breaks")
        .ok_or_else(|| {
            crate::error::Error::new("Abaco:type", "the piecewise polynomial has no breaks")
        })?
        .to_real()?
        .d;
    let c = s
        .get("coefs")
        .ok_or_else(|| {
            crate::error::Error::new("Abaco:type", "the piecewise polynomial has no coefs")
        })?
        .to_real()?;
    if c.r + 1 != breaks.len() || c.c == 0 || c.c > 4 {
        return err("Abaco:size", "the piecewise polynomial's breaks and coefs do not agree (up to cubic pieces are supported)");
    }
    let mut coefs = Vec::with_capacity(c.r);
    for i in 0..c.r {
        let mut row = [0.0; 4];
        for k in 0..c.c {
            // column k holds the coefficient of t^(order-1-k)
            row[c.c - 1 - k] = c.d.get(k * c.r + i).copied().unwrap_or(0.0);
        }
        coefs.push(row);
    }
    Ok(PiecewiseCubic { breaks, coefs })
}

fn spline_like(a: &[Value], name: &str, pchip: bool) -> Out {
    arity(a, 2, 3, name)?;
    let (x, y) = (real_arg(a, 0, name)?.d, real_arg(a, 1, name)?.d);
    // spline(x, y) with y holding two extra end slopes is clamped in MATLAB; not supported.
    if y.len() != x.len() {
        return err(
            "Abaco:size",
            format!(
                "`{name}`: x has {} points and y has {}; clamped end slopes are not supported",
                x.len(),
                y.len()
            ),
        );
    }
    let (xs, ys) = sorted_points(&x, &[y])?;
    let p = if pchip {
        ni::pchip(&xs, &ys[0])
    } else {
        ni::spline(&xs, &ys[0])
    }
    .map_err(num_err)?;
    match a.get(2) {
        Some(q) => {
            let q = q.to_real()?;
            one(Value::Num(q.map(|&v| p.eval(v))))
        }
        None => one(pp_value(&p)),
    }
}

fn f_spline(_: &mut Interp, a: &[Value], _: usize) -> Out {
    spline_like(a, "spline", false)
}

fn f_pchip(_: &mut Interp, a: &[Value], _: usize) -> Out {
    spline_like(a, "pchip", true)
}

fn f_ppval(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "ppval")?;
    let p = pp_from_value(arg(a, 0, "ppval")?)?;
    let q = real_arg(a, 1, "ppval")?;
    one(Value::Num(q.map(|&v| p.eval(v))))
}

fn f_mkpp(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "mkpp")?;
    let breaks = real_arg(a, 0, "mkpp")?.d;
    let c = real_arg(a, 1, "mkpp")?;
    if breaks.len() < 2 || c.r + 1 != breaks.len() || c.c == 0 || c.c > 4 {
        return err(
            "Abaco:size",
            "`mkpp` needs n+1 breaks and an n-by-k coefficient matrix with k <= 4",
        );
    }
    if breaks.windows(2).any(|w| w[1] <= w[0]) {
        return err(
            "Abaco:domain",
            "`mkpp`: the breaks must be strictly increasing",
        );
    }
    one(Value::structure(vec![
        ("form", Value::text("pp")),
        ("breaks", Value::row(breaks)),
        ("coefs", Value::Num(c.clone())),
        ("pieces", Value::num(c.r as f64)),
        ("order", Value::num(c.c as f64)),
        ("dim", Value::num(1.0)),
    ]))
}

fn f_interp2(interp: &mut Interp, a: &[Value], _: usize) -> Out {
    let n_data = a.iter().take_while(|v| !matches!(v, Value::Str(_))).count();
    let method = match a.get(n_data) {
        Some(Value::Str(s)) => parse_method(s)?,
        _ => Method::Linear,
    };
    if !matches!(method, Method::Linear | Method::Nearest) {
        return err(
            "Abaco:domain",
            "`interp2` supports the 'linear' and 'nearest' methods",
        );
    }
    let (xs, ys, v, xq, yq) = match n_data {
        3 => {
            let v = real_arg(a, 0, "interp2")?;
            (
                (1..=v.c).map(|k| k as f64).collect::<Vec<_>>(),
                (1..=v.r).map(|k| k as f64).collect::<Vec<_>>(),
                v,
                real_arg(a, 1, "interp2")?,
                real_arg(a, 2, "interp2")?,
            )
        }
        5 => {
            let x = real_arg(a, 0, "interp2")?;
            let y = real_arg(a, 1, "interp2")?;
            let v = real_arg(a, 2, "interp2")?;
            // Grids from meshgrid are matrices: the axes are their first row and first column.
            let xs = if x.r == 1 || x.c == 1 {
                x.d.clone()
            } else {
                x.row_vec(0)
            };
            let ys = if y.r == 1 || y.c == 1 {
                y.d.clone()
            } else {
                y.column(0)
            };
            (
                xs,
                ys,
                v,
                real_arg(a, 3, "interp2")?,
                real_arg(a, 4, "interp2")?,
            )
        }
        _ => {
            return err(
                "Abaco:arity",
                "`interp2` needs (X, Y, V, Xq, Yq) or (V, Xq, Yq)",
            )
        }
    };
    if xs.len() != v.c || ys.len() != v.r || xs.len() < 2 || ys.len() < 2 {
        return err(
            "Abaco:size",
            format!(
                "`interp2`: V is {}x{} but the grid has {} x-points and {} y-points",
                v.r,
                v.c,
                xs.len(),
                ys.len()
            ),
        );
    }
    if xs.windows(2).any(|w| w[1] <= w[0]) || ys.windows(2).any(|w| w[1] <= w[0]) {
        return err(
            "Abaco:domain",
            "`interp2`: the grid axes must be strictly increasing",
        );
    }
    let shape = crate::ops::zip(&xq, &yq, |&p, &q| (p, q))?;
    interp.charge(shape.d.len() as u64 / 4 + 1)?;
    let at = |i: usize, j: usize| v.d.get(j * v.r + i).copied().unwrap_or(f64::NAN);
    let out = shape.map(|&(px, py)| {
        if px.is_nan()
            || py.is_nan()
            || px < xs[0]
            || px > xs[xs.len() - 1]
            || py < ys[0]
            || py > ys[ys.len() - 1]
        {
            return f64::NAN;
        }
        let j = xs
            .partition_point(|&b| b <= px)
            .saturating_sub(1)
            .min(xs.len() - 2);
        let i = ys
            .partition_point(|&b| b <= py)
            .saturating_sub(1)
            .min(ys.len() - 2);
        let tx = (px - xs[j]) / (xs[j + 1] - xs[j]);
        let ty = (py - ys[i]) / (ys[i + 1] - ys[i]);
        if method == Method::Nearest {
            let jj = if tx < 0.5 { j } else { j + 1 };
            let ii = if ty < 0.5 { i } else { i + 1 };
            return at(ii, jj);
        }
        let top = at(i, j) * (1.0 - tx) + at(i, j + 1) * tx;
        let bottom = at(i + 1, j) * (1.0 - tx) + at(i + 1, j + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    });
    one(Value::Num(out))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("interp1", f_interp1),
    ("interp2", f_interp2),
    ("mkpp", f_mkpp),
    ("pchip", f_pchip),
    ("ppval", f_ppval),
    ("spline", f_spline),
];
