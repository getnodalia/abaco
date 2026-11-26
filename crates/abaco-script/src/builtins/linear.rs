// SPDX-License-Identifier: MIT OR Apache-2.0
//! Linear algebra: factorisations, eigenvalues, norms and solvers.

use super::{arg, arity, boolean, is_flag, num, one, real_arg, real_out, Bf, Out};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::linalg::{from_cmatrix, from_matrix, num_err, to_matrix};
use crate::value::{Arr, Value};
use nodalia_numerics::{cholesky::Cholesky, eigen, matfun, svd as nsvd, Matrix};

fn mat(args: &[Value], i: usize, name: &str) -> Result<(Arr<f64>, Matrix<f64>)> {
    let a = real_arg(args, i, name)?;
    let m = to_matrix(&a)?;
    Ok((a, m))
}

fn square(a: &Arr<f64>, name: &str) -> Result<usize> {
    if a.r != a.c {
        return err(
            "Abaco:size",
            format!("`{name}` needs a square matrix, got {}x{}", a.r, a.c),
        );
    }
    Ok(a.r)
}

fn f_det(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "det")?;
    one(crate::linalg::det(arg(a, 0, "det")?)?)
}

fn f_inv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "inv")?;
    one(crate::linalg::inv(arg(a, 0, "inv")?)?)
}

fn f_linsolve(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "linsolve")?;
    one(crate::linalg::solve(
        arg(a, 0, "linsolve")?,
        arg(a, 1, "linsolve")?,
    )?)
}

fn f_expm(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "expm")?;
    one(crate::linalg::expm(arg(a, 0, "expm")?)?)
}

fn f_trace(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "trace")?;
    let m = real_arg(a, 0, "trace")?;
    square(&m, "trace")?;
    num((0..m.r).map(|i| m.at(i, i).copied().unwrap_or(0.0)).sum())
}

fn f_transpose(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "transpose")?;
    one(crate::ops::transpose(arg(a, 0, "transpose")?, false)?)
}

fn f_ctranspose(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "ctranspose")?;
    one(crate::ops::transpose(arg(a, 0, "ctranspose")?, true)?)
}

fn f_svd(_: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 1, 2, "svd")?;
    let (_, m) = mat(a, 0, "svd")?;
    let s = nsvd::svd(&m).map_err(num_err)?;
    if nargout <= 1 {
        return one(Value::col(s.s));
    }
    let k = s.s.len();
    let mut d = vec![0.0; k * k];
    for (i, x) in s.s.iter().enumerate() {
        d[i * k + i] = *x;
    }
    Ok(vec![
        Value::Num(from_matrix(&s.u)),
        Value::mat(k, k, d),
        Value::Num(from_matrix(&s.v)),
    ])
}

fn f_rank(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "rank")?;
    let (_, m) = mat(a, 0, "rank")?;
    if m.rows() == 0 || m.cols() == 0 {
        return num(0.0);
    }
    let tol = if a.len() > 1 {
        Some(super::scalar_arg(a, 1, "rank")?)
    } else {
        None
    };
    num(nsvd::rank(&m, tol).map_err(num_err)? as f64)
}

fn f_pinv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "pinv")?;
    let (r, m) = mat(a, 0, "pinv")?;
    if r.d.is_empty() {
        return one(Value::mat(r.c, r.r, Vec::new()));
    }
    let tol = if a.len() > 1 {
        Some(super::scalar_arg(a, 1, "pinv")?)
    } else {
        None
    };
    real_out(from_matrix(&nsvd::pinv(&m, tol).map_err(num_err)?))
}

fn f_null(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "null")?;
    let (_, m) = mat(a, 0, "null")?;
    real_out(from_matrix(&nsvd::null_space(&m).map_err(num_err)?))
}

fn f_orth(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "orth")?;
    let (_, m) = mat(a, 0, "orth")?;
    let s = nsvd::svd(&m).map_err(num_err)?;
    let tol = s.s.first().copied().unwrap_or(0.0) * m.rows().max(m.cols()) as f64 * f64::EPSILON;
    let k = s.s.iter().filter(|&&x| x > tol).count();
    let u = from_matrix(&s.u);
    real_out(Arr {
        r: u.r,
        c: k,
        d: u.d.get(..u.r * k).unwrap_or(&[]).to_vec(),
    })
}

fn f_cond(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "cond")?;
    let (_, m) = mat(a, 0, "cond")?;
    let s = nsvd::svd(&m).map_err(num_err)?;
    match (s.s.first(), s.s.last()) {
        (Some(hi), Some(lo)) => num(if *lo == 0.0 { f64::INFINITY } else { hi / lo }),
        _ => num(0.0),
    }
}

fn f_norm(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 2, "norm")?;
    if let Value::Cplx(z) = &args[0] {
        if z.r == 1 || z.c == 1 {
            return num(z.d.iter().map(|c| c.norm_sqr()).sum::<f64>().sqrt());
        }
        return err("Abaco:type", "`norm` of a complex matrix is not supported");
    }
    let a = real_arg(args, 0, "norm")?;
    let p = match args.get(1) {
        None => 2.0,
        Some(Value::Str(s)) if s.eq_ignore_ascii_case("fro") => -1.0,
        Some(Value::Str(s)) if s.eq_ignore_ascii_case("inf") => f64::INFINITY,
        Some(v) => v.scalar_f64("the norm type")?,
    };
    if a.d.is_empty() {
        return num(0.0);
    }
    if a.r == 1 || a.c == 1 {
        let v = &a.d;
        return num(if p == -1.0 || p == 2.0 {
            let scale = v.iter().fold(0.0f64, |m, x| m.max(x.abs()));
            if scale == 0.0 || !scale.is_finite() {
                scale
            } else {
                scale
                    * v.iter()
                        .map(|x| (x / scale) * (x / scale))
                        .sum::<f64>()
                        .sqrt()
            }
        } else if p == f64::INFINITY {
            v.iter().fold(0.0f64, |m, x| m.max(x.abs()))
        } else if p == f64::NEG_INFINITY {
            v.iter().fold(f64::INFINITY, |m, x| m.min(x.abs()))
        } else if p == 1.0 {
            v.iter().map(|x| x.abs()).sum()
        } else if p > 0.0 {
            v.iter().map(|x| x.abs().powf(p)).sum::<f64>().powf(1.0 / p)
        } else {
            return err("Abaco:domain", "`norm` needs p > 0 for vectors");
        });
    }
    let m = to_matrix(&a)?;
    num(if p == -1.0 {
        m.norm_frobenius()
    } else if p == 1.0 {
        m.norm_1()
    } else if p == f64::INFINITY {
        m.norm_inf()
    } else if p == 2.0 {
        nsvd::svd(&m)
            .map_err(num_err)?
            .s
            .first()
            .copied()
            .unwrap_or(0.0)
    } else {
        return err(
            "Abaco:domain",
            "`norm` of a matrix supports 1, 2, Inf and 'fro'",
        );
    })
}

fn is_symmetric(a: &Arr<f64>) -> bool {
    if a.r != a.c {
        return false;
    }
    let scale =
        a.d.iter()
            .fold(0.0f64, |m, x| m.max(x.abs()))
            .max(f64::MIN_POSITIVE);
    for i in 0..a.r {
        for j in 0..i {
            let (x, y) = (
                a.at(i, j).copied().unwrap_or(0.0),
                a.at(j, i).copied().unwrap_or(0.0),
            );
            if (x - y).abs() > 1e-12 * scale {
                return false;
            }
        }
    }
    true
}

fn f_issymmetric(_: &mut Interp, a: &[Value], _: usize) -> Out {
    boolean(is_symmetric(&real_arg(a, 0, "issymmetric")?))
}

fn f_eig(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 2, "eig")?;
    let a = match &args[0] {
        Value::Cplx(_) => {
            return err(
                "Abaco:type",
                "`eig` of a complex matrix is not supported yet",
            )
        }
        _ => real_arg(args, 0, "eig")?,
    };
    let n = square(&a, "eig")?;
    if n == 0 {
        return Ok(vec![Value::empty(), Value::empty()]);
    }
    let m = to_matrix(&a)?;
    if is_symmetric(&a) {
        let mut sym = m.clone();
        // symmetrise exactly so rounding in the last bits does not reject the matrix
        for i in 0..n {
            for j in 0..i {
                let v = 0.5 * (sym.get(i, j).unwrap_or(0.0) + sym.get(j, i).unwrap_or(0.0));
                sym.set(i, j, v).map_err(num_err)?;
                sym.set(j, i, v).map_err(num_err)?;
            }
        }
        let e = eigen::symmetric(&sym).map_err(num_err)?;
        if nargout <= 1 {
            return one(Value::col(e.values));
        }
        let mut d = vec![0.0; n * n];
        for (i, x) in e.values.iter().enumerate() {
            d[i * n + i] = *x;
        }
        return Ok(vec![
            Value::Num(from_matrix(&e.vectors)),
            Value::mat(n, n, d),
        ]);
    }
    if nargout <= 1 {
        let vals = eigen::general(&m).map_err(num_err)?;
        return one(Value::cplx(Arr::col(vals)));
    }
    let (vals, vecs) = matfun::eig_general(&m).map_err(num_err)?;
    let mut d = vec![nodalia_numerics::Complex64::new(0.0, 0.0); n * n];
    for (i, x) in vals.iter().enumerate() {
        d[i * n + i] = *x;
    }
    Ok(vec![
        Value::cplx(from_cmatrix(&vecs)),
        Value::cplx(Arr::new(n, n, d)),
    ])
}

fn f_chol(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "chol")?;
    let (r, m) = mat(a, 0, "chol")?;
    square(&r, "chol")?;
    let l = Cholesky::factor(&m)
        .map_err(num_err)?
        .l()
        .map_err(num_err)?;
    let lower = is_flag(a, 1, "lower");
    real_out(from_matrix(&if lower { l } else { l.transpose() }))
}

fn f_lu(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 1, "lu")?;
    let a = real_arg(args, 0, "lu")?;
    let (m, n) = (a.r, a.c);
    let k = m.min(n);
    let mut w = a.clone();
    let mut perm: Vec<usize> = (0..m).collect();
    for c in 0..k {
        let mut p = c;
        let mut best = w.at(c, c).copied().unwrap_or(0.0).abs();
        for r in c + 1..m {
            let v = w.at(r, c).copied().unwrap_or(0.0).abs();
            if v > best {
                best = v;
                p = r;
            }
        }
        if best == 0.0 {
            continue;
        }
        if p != c {
            for j in 0..n {
                w.d.swap(j * m + c, j * m + p);
            }
            perm.swap(c, p);
        }
        let piv = w.at(c, c).copied().unwrap_or(1.0);
        for r in c + 1..m {
            let f = w.at(r, c).copied().unwrap_or(0.0) / piv;
            if let Some(s) = w.d.get_mut(c * m + r) {
                *s = f;
            }
            for j in c + 1..n {
                let u = w.at(c, j).copied().unwrap_or(0.0);
                if let Some(s) = w.d.get_mut(j * m + r) {
                    *s -= f * u;
                }
            }
        }
    }
    let mut l = vec![0.0; m * k];
    let mut u = vec![0.0; k * n];
    for i in 0..m {
        for j in 0..k {
            l[j * m + i] = if i == j {
                1.0
            } else if i > j {
                w.at(i, j).copied().unwrap_or(0.0)
            } else {
                0.0
            };
        }
    }
    for i in 0..k {
        for j in 0..n {
            u[j * k + i] = if j >= i {
                w.at(i, j).copied().unwrap_or(0.0)
            } else {
                0.0
            };
        }
    }
    let mut p = vec![0.0; m * m];
    for (row, &src) in perm.iter().enumerate() {
        p[src * m + row] = 1.0;
    }
    if nargout <= 2 {
        // L is returned already permuted: P' * L
        let mut pl = vec![0.0; m * k];
        for (row, &src) in perm.iter().enumerate() {
            for j in 0..k {
                pl[j * m + src] = l[j * m + row];
            }
        }
        return Ok(vec![Value::mat(m, k, pl), Value::mat(k, n, u)]);
    }
    Ok(vec![
        Value::mat(m, k, l),
        Value::mat(k, n, u),
        Value::mat(m, m, p),
    ])
}

fn f_qr(_: &mut Interp, args: &[Value], nargout: usize) -> Out {
    arity(args, 1, 2, "qr")?;
    let a = real_arg(args, 0, "qr")?;
    let econ = args.len() == 2;
    let (m, n) = (a.r, a.c);
    let mut r = a.clone();
    let mut q = Arr::filled(m, m, 0.0);
    for i in 0..m {
        q.d[i * m + i] = 1.0;
    }
    for k in 0..m.saturating_sub(1).min(n.max(1)).min(m) {
        if k >= n {
            break;
        }
        let x: Vec<f64> = (k..m).map(|i| r.at(i, k).copied().unwrap_or(0.0)).collect();
        let norm = x.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm == 0.0 {
            continue;
        }
        let alpha = if x[0] >= 0.0 { -norm } else { norm };
        let mut v = x;
        v[0] -= alpha;
        let vn = v.iter().map(|t| t * t).sum::<f64>().sqrt();
        if vn == 0.0 {
            continue;
        }
        for t in &mut v {
            *t /= vn;
        }
        for j in k..n {
            let dot: f64 = (k..m)
                .map(|i| v[i - k] * r.at(i, j).copied().unwrap_or(0.0))
                .sum();
            for i in k..m {
                r.d[j * m + i] -= 2.0 * v[i - k] * dot;
            }
        }
        for row in 0..m {
            let dot: f64 = (k..m).map(|i| q.d[i * m + row] * v[i - k]).sum();
            for i in k..m {
                q.d[i * m + row] -= 2.0 * dot * v[i - k];
            }
        }
    }
    for j in 0..n {
        for i in j + 1..m {
            r.d[j * m + i] = 0.0;
        }
    }
    if nargout <= 1 {
        return real_out(r);
    }
    if econ && m > n {
        let qe = Arr {
            r: m,
            c: n,
            d: q.d.get(..m * n).unwrap_or(&[]).to_vec(),
        };
        let mut re = Arr::filled(n, n, 0.0);
        for j in 0..n {
            for i in 0..n {
                re.d[j * n + i] = r.at(i, j).copied().unwrap_or(0.0);
            }
        }
        return Ok(vec![Value::Num(qe), Value::Num(re)]);
    }
    Ok(vec![Value::Num(q), Value::Num(r)])
}

pub const TABLE: &[(&str, Bf)] = &[
    ("det", f_det),
    ("inv", f_inv),
    ("linsolve", f_linsolve),
    ("expm", f_expm),
    ("trace", f_trace),
    ("transpose", f_transpose),
    ("ctranspose", f_ctranspose),
    ("svd", f_svd),
    ("rank", f_rank),
    ("pinv", f_pinv),
    ("null", f_null),
    ("orth", f_orth),
    ("cond", f_cond),
    ("norm", f_norm),
    ("issymmetric", f_issymmetric),
    ("eig", f_eig),
    ("chol", f_chol),
    ("lu", f_lu),
    ("qr", f_qr),
];
