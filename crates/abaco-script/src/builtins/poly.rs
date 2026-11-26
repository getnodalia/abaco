// SPDX-License-Identifier: MIT OR Apache-2.0
//! Polynomials: roots, evaluation, fitting, products and calculus.

use super::{arg, arity, num, one, real_arg, real_out, shaped_like, usize_arg, vec_arg, Bf, Out};
use crate::error::err;
use crate::interp::Interp;
use crate::linalg::{num_err, to_matrix};
use crate::value::{Arr, Value};
use nodalia_numerics::{qr, roots, Complex64, Matrix};

fn f_roots(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "roots")?;
    let c = vec_arg(a, 0, "roots")?;
    if c.iter().all(|&x| x == 0.0) {
        return one(Value::empty());
    }
    let trailing = c.iter().rev().take_while(|&&x| x == 0.0).count();
    let core = &c[..c.len() - trailing];
    let mut r: Vec<Complex64> = roots::poly_roots(core).map_err(num_err)?;
    r.reverse();
    r.extend(std::iter::repeat_n(Complex64::new(0.0, 0.0), trailing));
    if r.is_empty() {
        return one(Value::empty());
    }
    one(Value::cplx(Arr::col(r)))
}

fn horner(p: &[f64], x: f64) -> f64 {
    p.iter().fold(0.0, |acc, &c| acc * x + c)
}

fn horner_c(p: &[f64], x: Complex64) -> Complex64 {
    p.iter()
        .fold(Complex64::new(0.0, 0.0), |acc, &c| acc * x + c)
}

fn f_polyval(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "polyval")?;
    let p = vec_arg(a, 0, "polyval")?;
    match arg(a, 1, "polyval")? {
        Value::Cplx(z) => one(Value::cplx(z.map(|&x| horner_c(&p, x)))),
        v => {
            let x = real_arg(std::slice::from_ref(v), 0, "polyval")?;
            real_out(x.map(|&t| horner(&p, t)))
        }
    }
}

fn f_polyfit(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 3, "polyfit")?;
    let (x, y) = (vec_arg(a, 0, "polyfit")?, vec_arg(a, 1, "polyfit")?);
    let n = usize_arg(a, 2, "polyfit")?;
    if x.len() != y.len() {
        return err(
            "Abaco:size",
            format!(
                "`polyfit` needs x and y of the same length, got {} and {}",
                x.len(),
                y.len()
            ),
        );
    }
    if x.len() <= n {
        return err(
            "Abaco:size",
            format!(
                "`polyfit` of degree {n} needs at least {} points, got {}",
                n + 1,
                x.len()
            ),
        );
    }
    let m = x.len();
    let mut rows = Vec::with_capacity(m * (n + 1));
    for &xi in &x {
        for k in 0..=n {
            rows.push(xi.powi((n - k) as i32));
        }
    }
    let v = Matrix::new(m, n + 1, rows).map_err(num_err)?;
    let sol = qr::least_squares(&v, &y).map_err(num_err)?;
    one(Value::row(sol.x))
}

fn conv_vec(a: &[f64], b: &[f64]) -> Vec<f64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

fn f_conv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "conv")?;
    let (x, y) = (real_arg(a, 0, "conv")?, real_arg(a, 1, "conv")?);
    let out = conv_vec(&x.d, &y.d);
    // a column input gives a column result, as in MATLAB
    let like = if x.c == 1 && x.r > 1 || (x.d.len() == 1 && y.c == 1 && y.r > 1) {
        Arr::col(vec![0.0])
    } else {
        Arr::row(vec![0.0])
    };
    one(shaped_like(out, &like))
}

fn f_deconv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "deconv")?;
    let (num_, den) = (vec_arg(a, 0, "deconv")?, vec_arg(a, 1, "deconv")?);
    let lead = den.iter().position(|&x| x != 0.0).ok_or_else(|| {
        crate::error::Error::new("Abaco:domain", "`deconv` by the zero polynomial")
    })?;
    let den = &den[lead..];
    if num_.len() < den.len() {
        return Ok(vec![Value::num(0.0), Value::row(num_)]);
    }
    let mut rem = num_.clone();
    let q_len = num_.len() - den.len() + 1;
    let mut q = vec![0.0; q_len];
    for i in 0..q_len {
        let c = rem[i] / den[0];
        q[i] = c;
        for (j, d) in den.iter().enumerate() {
            rem[i + j] -= c * d;
        }
    }
    Ok(vec![Value::row(q), Value::row(rem)])
}

fn f_polyder(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "polyder")?;
    let p = vec_arg(a, 0, "polyder")?;
    let n = p.len();
    if n <= 1 {
        return num(0.0);
    }
    let d: Vec<f64> = p
        .iter()
        .take(n - 1)
        .enumerate()
        .map(|(i, c)| c * (n - 1 - i) as f64)
        .collect();
    let first = d.iter().position(|&x| x != 0.0).unwrap_or(d.len() - 1);
    one(Value::row(d[first..].to_vec()))
}

fn f_polyint(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "polyint")?;
    let p = vec_arg(a, 0, "polyint")?;
    let k = if a.len() > 1 {
        super::scalar_arg(a, 1, "polyint")?
    } else {
        0.0
    };
    let n = p.len();
    let mut out: Vec<f64> = p
        .iter()
        .enumerate()
        .map(|(i, c)| c / (n - i) as f64)
        .collect();
    out.push(k);
    one(Value::row(out))
}

fn f_poly(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "poly")?;
    let rts: Vec<Complex64> = match arg(a, 0, "poly")? {
        Value::Cplx(z) if z.r == 1 || z.c == 1 => z.d.clone(),
        v => {
            let m = v.to_real()?;
            if m.r == m.c && m.r > 1 {
                // characteristic polynomial from the eigenvalues
                nodalia_numerics::eigen::general(&to_matrix(&m)?).map_err(num_err)?
            } else {
                m.d.iter().map(|&x| Complex64::new(x, 0.0)).collect()
            }
        }
    };
    let mut c = vec![Complex64::new(1.0, 0.0)];
    for r in rts {
        let mut next = vec![Complex64::new(0.0, 0.0); c.len() + 1];
        for (i, x) in c.iter().enumerate() {
            next[i] += x;
            next[i + 1] -= x * r;
        }
        c = next;
    }
    one(Value::cplx(Arr::row(c)))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("roots", f_roots),
    ("polyval", f_polyval),
    ("polyfit", f_polyfit),
    ("conv", f_conv),
    ("deconv", f_deconv),
    ("polyder", f_polyder),
    ("polyint", f_polyint),
    ("poly", f_poly),
];
