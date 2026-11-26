// SPDX-License-Identifier: MIT OR Apache-2.0
//! Signal processing: Fourier transforms, filtering, windows, filter design and spectra.

use super::{arg, arity, is_flag, one, pick_dim, real_arg, scalar_arg, usize_arg, Bf, Out};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::linalg::num_err;
use crate::value::{Arr, Value};
use nodalia_numerics::{fft, solve, Matrix};
use num_complex::Complex64;
use std::f64::consts::PI;

type C = Complex64;

// ---------- Fourier transforms ----------

/// Applies a transform of `n` points along `dim`; the other dimension is untouched.
fn along(
    a: &Arr<C>,
    n: Option<usize>,
    dim: usize,
    f: &dyn Fn(&[C]) -> Result<Vec<C>>,
) -> Result<Arr<C>> {
    let len_of = |m: usize| n.unwrap_or(m);
    let resize = |mut v: Vec<C>, to: usize| {
        v.resize(to, C::new(0.0, 0.0));
        v
    };
    match dim {
        1 => {
            let to = len_of(a.r);
            let mut d = Vec::with_capacity(to.saturating_mul(a.c));
            for j in 0..a.c {
                let col = resize(a.column(j), to);
                d.extend(f(&col)?);
            }
            Ok(Arr::new(to, a.c, d))
        }
        2 => {
            let to = len_of(a.c);
            let mut rows: Vec<Vec<C>> = Vec::with_capacity(a.r);
            for i in 0..a.r {
                rows.push(f(&resize(a.row_vec(i), to))?);
            }
            let mut d = Vec::with_capacity(a.r.saturating_mul(to));
            for j in 0..to {
                for row in &rows {
                    d.push(row.get(j).copied().unwrap_or_default());
                }
            }
            Ok(Arr::new(a.r, to, d))
        }
        _ => Ok(a.clone()),
    }
}

fn limit_points(n: usize, cols: usize) -> Result<()> {
    if n.saturating_mul(cols.max(1)) > crate::ops::MAX_ELEMENTS {
        return err(
            "Abaco:limit",
            format!("a transform of {n} points would be too large"),
        );
    }
    Ok(())
}

fn fourier(interp: &mut Interp, a: &[Value], name: &str, inverse: bool) -> Out {
    arity(a, 1, 3, name)?;
    let x = arg(a, 0, name)?.to_complex()?;
    let n = match a.get(1) {
        Some(v) if !v.is_empty() => Some(v.scalar_usize("the transform length")?),
        _ => None,
    };
    let dim = pick_dim(a, 2, x.r, name)?;
    limit_points(n.unwrap_or_else(|| x.r.max(x.c)), x.r.max(x.c))?;
    interp.charge(x.d.len() as u64 / 8 + 1)?;
    let out = along(&x, n, dim, &|v| {
        if inverse {
            fft::ifft(v).map_err(num_err)
        } else {
            fft::fft(v).map_err(num_err)
        }
    })?;
    // A real signal has a conjugate-symmetric spectrum; make that exact so ifft returns it real.
    let out = if !inverse && matches!(a.first(), Some(Value::Num(_) | Value::Bool(_))) {
        symmetrised(out, dim)
    } else {
        out
    };
    // ifft of a conjugate-symmetric spectrum is real.
    if inverse && is_conj_symmetric(&x, dim) {
        return one(Value::Num(out.map(|z| z.re)));
    }
    one(Value::cplx(out))
}

fn symmetrised(mut x: Arr<C>, dim: usize) -> Arr<C> {
    let fix = |v: &mut [C]| {
        let n = v.len();
        for k in 1..=n / 2 {
            if let Some(&a) = v.get(k) {
                if let Some(b) = v.get_mut(n - k) {
                    *b = a.conj();
                }
            }
        }
        if let Some(first) = v.first_mut() {
            first.im = 0.0;
        }
        if n.is_multiple_of(2) {
            if let Some(mid) = v.get_mut(n / 2) {
                mid.im = 0.0;
            }
        }
    };
    match dim {
        1 => {
            let r = x.r;
            for j in 0..x.c {
                if let Some(col) = x.d.get_mut(j * r..(j + 1) * r) {
                    fix(col);
                }
            }
        }
        2 => {
            for i in 0..x.r {
                let mut row = x.row_vec(i);
                fix(&mut row);
                for (j, v) in row.into_iter().enumerate() {
                    if let Some(slot) = x.d.get_mut(j * x.r + i) {
                        *slot = v;
                    }
                }
            }
        }
        _ => {}
    }
    x
}

fn is_conj_symmetric(x: &Arr<C>, dim: usize) -> bool {
    let check = |v: &[C]| {
        let n = v.len();
        (0..n).all(|k| match (v.get(k), v.get((n - k) % n.max(1))) {
            (Some(a), Some(b)) => *a == b.conj(),
            _ => true,
        })
    };
    match dim {
        1 => (0..x.c).all(|j| check(&x.column(j))),
        2 => (0..x.r).all(|i| check(&x.row_vec(i))),
        _ => false,
    }
}

fn f_fft(i: &mut Interp, a: &[Value], _: usize) -> Out {
    fourier(i, a, "fft", false)
}

fn f_ifft(i: &mut Interp, a: &[Value], _: usize) -> Out {
    fourier(i, a, "ifft", true)
}

fn fourier2(interp: &mut Interp, a: &[Value], name: &str, inverse: bool) -> Out {
    arity(a, 1, 1, name)?;
    let x = arg(a, 0, name)?.to_complex()?;
    limit_points(x.r.max(x.c), x.r.max(x.c))?;
    interp.charge(x.d.len() as u64 / 4 + 1)?;
    let f = |v: &[C]| {
        if inverse {
            fft::ifft(v).map_err(num_err)
        } else {
            fft::fft(v).map_err(num_err)
        }
    };
    let step = along(&x, None, 1, &f)?;
    let out = along(&step, None, 2, &f)?;
    one(Value::cplx(out))
}

fn f_fft2(i: &mut Interp, a: &[Value], _: usize) -> Out {
    fourier2(i, a, "fft2", false)
}

fn f_ifft2(i: &mut Interp, a: &[Value], _: usize) -> Out {
    fourier2(i, a, "ifft2", true)
}

/// Rotates a vector or matrix by `k` positions along each dimension.
fn shifted<T: Clone>(a: &Arr<T>, dim: Option<usize>, inverse: bool) -> Arr<T> {
    let shift_for = |n: usize| {
        let k = n / 2;
        if inverse {
            (n - k) % n.max(1)
        } else {
            k
        }
    };
    let rows_shift = |_: ()| {
        if a.r > 1 || dim == Some(1) {
            shift_for(a.r)
        } else {
            0
        }
    };
    let cols_shift = |_: ()| {
        if a.c > 1 || dim == Some(2) {
            shift_for(a.c)
        } else {
            0
        }
    };
    let (sr, sc) = match dim {
        Some(1) => (rows_shift(()), 0),
        Some(2) => (0, cols_shift(())),
        _ => (rows_shift(()), cols_shift(())),
    };
    let mut d = Vec::with_capacity(a.d.len());
    for j in 0..a.c {
        for i in 0..a.r {
            let si = (i + a.r - sr % a.r.max(1)) % a.r.max(1);
            let sj = (j + a.c - sc % a.c.max(1)) % a.c.max(1);
            if let Some(v) = a.d.get(sj * a.r + si) {
                d.push(v.clone());
            }
        }
    }
    Arr::new(a.r, a.c, d)
}

fn shift_value(a: &[Value], name: &str, inverse: bool) -> Out {
    arity(a, 1, 2, name)?;
    let dim = match a.get(1) {
        Some(v) => Some(v.scalar_usize("the dimension")?),
        None => None,
    };
    match arg(a, 0, name)? {
        Value::Num(x) => one(Value::Num(shifted(x, dim, inverse))),
        Value::Cplx(x) => one(Value::Cplx(shifted(x, dim, inverse))),
        Value::Bool(x) => one(Value::Bool(shifted(x, dim, inverse))),
        other => err(
            "Abaco:type",
            format!("`{name}` needs numbers, got {}", other.describe()),
        ),
    }
}

fn f_fftshift(_: &mut Interp, a: &[Value], _: usize) -> Out {
    shift_value(a, "fftshift", false)
}

fn f_ifftshift(_: &mut Interp, a: &[Value], _: usize) -> Out {
    shift_value(a, "ifftshift", true)
}

fn f_nextpow2(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "nextpow2")?;
    let x = real_arg(a, 0, "nextpow2")?;
    one(Value::Num(x.map(|&v| {
        let v = v.abs();
        if v == 0.0 {
            0.0
        } else if !v.is_finite() {
            f64::NAN
        } else {
            v.log2().ceil()
        }
    })))
}

// ---------- digital filters ----------

/// Normalised, equal-length numerator and denominator.
fn normalise(b: &[f64], a: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
    let a0 = match a.first() {
        Some(&v) if v != 0.0 && v.is_finite() => v,
        _ => {
            return err(
                "Abaco:domain",
                "the first coefficient of the denominator must be non-zero",
            )
        }
    };
    if b.is_empty() {
        return err("Abaco:domain", "the numerator is empty");
    }
    let n = a.len().max(b.len());
    let mut bb: Vec<f64> = b.iter().map(|v| v / a0).collect();
    let mut aa: Vec<f64> = a.iter().map(|v| v / a0).collect();
    bb.resize(n, 0.0);
    aa.resize(n, 0.0);
    Ok((bb, aa))
}

/// Direct form II transposed; returns the output and the final state.
fn lfilter(b: &[f64], a: &[f64], x: &[f64], zi: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = b.len();
    let mut z: Vec<f64> = (0..n.saturating_sub(1))
        .map(|i| zi.get(i).copied().unwrap_or(0.0))
        .collect();
    let mut y = Vec::with_capacity(x.len());
    for &xv in x {
        let out = b[0] * xv + z.first().copied().unwrap_or(0.0);
        for i in 0..z.len() {
            let next = z.get(i + 1).copied().unwrap_or(0.0);
            z[i] = b[i + 1] * xv - a[i + 1] * out + next;
        }
        y.push(out);
    }
    (y, z)
}

/// The steady-state initial condition for a unit step.
fn filter_zi(b: &[f64], a: &[f64]) -> Result<Vec<f64>> {
    let n = b.len();
    if n < 2 {
        return Ok(Vec::new());
    }
    let m = n - 1;
    let mut mat = vec![0.0; m * m];
    for i in 0..m {
        for j in 0..m {
            // (I - companion(a)^T)[i][j]; companion has -a[1..]/a0 in its first row and ones
            // on the sub-diagonal, so its transpose has them in the first column and super-diagonal.
            let mut v = if i == j { 1.0 } else { 0.0 };
            if j == 0 {
                v += a[i + 1];
            }
            if j == i + 1 {
                v -= 1.0;
            }
            mat[i * m + j] = v;
        }
    }
    let rhs: Vec<f64> = (0..m).map(|i| b[i + 1] - a[i + 1] * b[0]).collect();
    let matrix = Matrix::new(m, m, mat).map_err(num_err)?;
    solve(&matrix, &rhs).map_err(num_err)
}

fn f_filter(interp: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 3, 4, "filter")?;
    let b = real_arg(a, 0, "filter")?.d;
    let den = real_arg(a, 1, "filter")?.d;
    let x = real_arg(a, 2, "filter")?;
    let (bb, aa) = normalise(&b, &den)?;
    let zi = match a.get(3) {
        Some(v) => v.to_real()?.d,
        None => Vec::new(),
    };
    interp.charge((x.d.len() * bb.len()) as u64 / 16 + 1)?;
    let _ = nargout;
    let cols: Vec<Vec<f64>> = if x.r == 1 {
        vec![x.d.clone()]
    } else {
        (0..x.c).map(|j| x.column(j)).collect()
    };
    let mut out = Vec::new();
    let mut zf = Vec::new();
    for col in &cols {
        let (y, z) = lfilter(&bb, &aa, col, &zi);
        out.extend(y);
        zf.extend(z);
    }
    let y = if x.r == 1 {
        Value::row(out)
    } else {
        Value::mat(x.r, x.c, out)
    };
    let zrows = bb.len().saturating_sub(1);
    Ok(vec![
        y,
        if x.r == 1 {
            Value::col(zf)
        } else {
            Value::mat(zrows, x.c, zf)
        },
    ])
}

fn f_filtfilt(interp: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 3, "filtfilt")?;
    let b = real_arg(a, 0, "filtfilt")?.d;
    let den = real_arg(a, 1, "filtfilt")?.d;
    let x = real_arg(a, 2, "filtfilt")?;
    let (bb, aa) = normalise(&b, &den)?;
    let order = bb.len() - 1;
    let pad = 3 * order.max(1);
    let zi = filter_zi(&bb, &aa)?;
    let cols: Vec<Vec<f64>> = if x.r == 1 {
        vec![x.d.clone()]
    } else {
        (0..x.c).map(|j| x.column(j)).collect()
    };
    interp.charge((x.d.len() * bb.len()) as u64 / 8 + 1)?;
    let mut out = Vec::new();
    for col in &cols {
        let n = col.len();
        if n <= pad {
            return err(
                "Abaco:domain",
                format!("`filtfilt` needs more than {pad} samples for this filter, got {n}"),
            );
        }
        // Odd reflection about the end points, so the transient starts quiet.
        let first = col[0];
        let last = col[n - 1];
        let mut ext = Vec::with_capacity(n + 2 * pad);
        for k in (1..=pad).rev() {
            ext.push(2.0 * first - col[k]);
        }
        ext.extend_from_slice(col);
        for k in 1..=pad {
            ext.push(2.0 * last - col[n - 1 - k]);
        }
        let z0: Vec<f64> = zi.iter().map(|z| z * ext[0]).collect();
        let (mut y, _) = lfilter(&bb, &aa, &ext, &z0);
        y.reverse();
        let z1: Vec<f64> = zi.iter().map(|z| z * y[0]).collect();
        let (mut y2, _) = lfilter(&bb, &aa, &y, &z1);
        y2.reverse();
        out.extend_from_slice(&y2[pad..pad + n]);
    }
    one(if x.r == 1 {
        Value::row(out)
    } else {
        Value::mat(x.r, x.c, out)
    })
}

// ---------- windows ----------

fn window_vec(kind: &str, n: usize, periodic: bool, beta: f64) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1.0];
    }
    let m = if periodic { n } else { n - 1 } as f64;
    (0..n)
        .map(|k| {
            let x = 2.0 * PI * k as f64 / m;
            match kind {
                "hamming" => 0.54 - 0.46 * x.cos(),
                "hann" => 0.5 - 0.5 * x.cos(),
                "blackman" => 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos(),
                "bartlett" => 1.0 - (2.0 * k as f64 / m - 1.0).abs(),
                "kaiser" => {
                    let r = 2.0 * k as f64 / m - 1.0;
                    bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta)
                }
                _ => 1.0,
            }
        })
        .collect()
}

/// Modified Bessel function of order zero (power series).
fn bessel_i0(x: f64) -> f64 {
    let q = x * x / 4.0;
    let (mut term, mut sum) = (1.0, 1.0);
    for k in 1..500 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn window(a: &[Value], name: &str, kind: &str) -> Out {
    arity(a, 1, 3, name)?;
    let n = usize_arg(a, 0, name)?;
    if n > 10_000_000 {
        return err("Abaco:limit", "the window is too long");
    }
    let mut beta = 0.5;
    let mut k = 1;
    if kind == "kaiser" {
        if let Some(v) = a.get(1).filter(|v| !matches!(v, Value::Str(_))) {
            beta = v.scalar_f64("beta")?;
            k = 2;
        }
    }
    one(Value::col(window_vec(
        kind,
        n,
        is_flag(a, k, "periodic"),
        beta,
    )))
}

fn f_hamming(_: &mut Interp, a: &[Value], _: usize) -> Out {
    window(a, "hamming", "hamming")
}
fn f_hann(_: &mut Interp, a: &[Value], _: usize) -> Out {
    window(a, "hann", "hann")
}
fn f_blackman(_: &mut Interp, a: &[Value], _: usize) -> Out {
    window(a, "blackman", "blackman")
}
fn f_bartlett(_: &mut Interp, a: &[Value], _: usize) -> Out {
    window(a, "bartlett", "bartlett")
}
fn f_kaiser(_: &mut Interp, a: &[Value], _: usize) -> Out {
    window(a, "kaiser", "kaiser")
}
fn f_rectwin(_: &mut Interp, a: &[Value], _: usize) -> Out {
    window(a, "rectwin", "rect")
}

// ---------- frequency response ----------

fn f_freqz(interp: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "freqz")?;
    let b = real_arg(a, 0, "freqz")?.d;
    let den = match a.get(1) {
        Some(v) if !v.is_empty() => v.to_real()?.d,
        _ => vec![1.0],
    };
    let w: Vec<f64> = match a.get(2) {
        Some(v) if v.numel() > 1 => v.to_real()?.d,
        Some(v) => {
            let n = v.scalar_usize("the number of points")?;
            (0..n).map(|k| PI * k as f64 / n as f64).collect()
        }
        None => (0..512).map(|k| PI * k as f64 / 512.0).collect(),
    };
    interp.charge(w.len() as u64 + 1)?;
    let h: Vec<C> = w
        .iter()
        .map(|&wk| {
            let z = C::from_polar(1.0, -wk);
            let num_ = eval_poly_rev(&b, z);
            let den_ = eval_poly_rev(&den, z);
            num_ / den_
        })
        .collect();
    Ok(vec![Value::cplx(Arr::col(h)), Value::col(w)])
}

/// `c[0] + c[1] u + c[2] u² …` evaluated at `u`.
fn eval_poly_rev(c: &[f64], u: C) -> C {
    c.iter().rev().fold(C::new(0.0, 0.0), |acc, &k| acc * u + k)
}

// ---------- filter design ----------

fn poly_from_roots(r: &[C]) -> Vec<f64> {
    let mut p = vec![C::new(1.0, 0.0)];
    for &root in r {
        let mut next = vec![C::new(0.0, 0.0); p.len() + 1];
        for (i, &c) in p.iter().enumerate() {
            next[i] += c;
            next[i + 1] -= c * root;
        }
        p = next;
    }
    p.into_iter().map(|c| c.re).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Band {
    Low,
    High,
    Pass,
    Stop,
}

/// Analog lowpass prototype → digital filter by frequency transformation and the bilinear map.
fn digital_from_prototype(
    mut zeros: Vec<C>,
    mut poles: Vec<C>,
    mut gain: f64,
    band: Band,
    wn: &[f64],
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n = poles.len();
    let warp = |w: f64| 4.0 * (PI * w / 2.0).tan();
    match band {
        Band::Low | Band::High => {
            let w = warp(wn[0]);
            if band == Band::Low {
                zeros = zeros.iter().map(|z| z * w).collect();
                poles = poles.iter().map(|p| p * w).collect();
                gain *= w.powi((poles.len() - zeros.len()) as i32);
            } else {
                // s -> w / s
                let prod_z: C = zeros.iter().map(|z| -z).product();
                let prod_p: C = poles.iter().map(|p| -p).product();
                gain *= (prod_z / prod_p).re;
                let mut nz: Vec<C> = zeros.iter().map(|z| C::new(w, 0.0) / z).collect();
                nz.extend(std::iter::repeat_n(
                    C::new(0.0, 0.0),
                    poles.len() - zeros.len(),
                ));
                zeros = nz;
                poles = poles.iter().map(|p| C::new(w, 0.0) / p).collect();
            }
        }
        Band::Pass | Band::Stop => {
            let (w1, w2) = (warp(wn[0]), warp(wn[1]));
            let (bw, w0) = (w2 - w1, (w1 * w2).sqrt());
            let split = |roots: &[C], stop: bool| -> Vec<C> {
                let mut out = Vec::with_capacity(roots.len() * 2);
                for &r in roots {
                    let s = if stop { C::new(bw, 0.0) / r } else { r * bw };
                    let half = s / 2.0;
                    let disc = (half * half - w0 * w0).sqrt();
                    out.push(half + disc);
                    out.push(half - disc);
                }
                out
            };
            if band == Band::Pass {
                let extra = poles.len() - zeros.len();
                let mut nz = split(&zeros, false);
                nz.extend(std::iter::repeat_n(C::new(0.0, 0.0), extra));
                poles = split(&poles, false);
                zeros = nz;
                gain *= bw.powi(extra as i32);
            } else {
                let prod_z: C = zeros.iter().map(|z| -z).product();
                let prod_p: C = poles.iter().map(|p| -p).product();
                gain *= (prod_z / prod_p).re;
                let mut nz = split(&zeros, true);
                let extra = poles.len() - zeros.len();
                for _ in 0..extra {
                    nz.push(C::new(0.0, w0));
                    nz.push(C::new(0.0, -w0));
                }
                poles = split(&poles, true);
                zeros = nz;
            }
        }
    }
    let _ = n;
    // Bilinear transform with fs = 2: s -> 4 (z - 1) / (z + 1).
    let fs2 = 4.0;
    let degree = poles.len() as i64 - zeros.len() as i64;
    let prod_z: C = zeros.iter().map(|z| C::new(fs2, 0.0) - z).product();
    let prod_p: C = poles.iter().map(|p| C::new(fs2, 0.0) - p).product();
    gain *= (prod_z / prod_p).re;
    let mut zd: Vec<C> = zeros.iter().map(|z| (fs2 + z) / (fs2 - z)).collect();
    let pd: Vec<C> = poles.iter().map(|p| (fs2 + p) / (fs2 - p)).collect();
    for _ in 0..degree.max(0) {
        zd.push(C::new(-1.0, 0.0));
    }
    let b: Vec<f64> = poly_from_roots(&zd).into_iter().map(|c| c * gain).collect();
    let a = poly_from_roots(&pd);
    if b.iter().chain(&a).any(|v| !v.is_finite()) {
        return err(
            "Abaco:numeric",
            "the filter design produced non-finite coefficients",
        );
    }
    Ok((b, a))
}

fn band_args(a: &[Value], from: usize, name: &str) -> Result<(Band, Vec<f64>)> {
    let wn = real_arg(a, from, name)?.d;
    let mut kind = None;
    for v in a.iter().skip(from + 1) {
        if let Value::Str(s) = v {
            kind = Some(s.to_ascii_lowercase());
        }
    }
    let band = match (wn.len(), kind.as_deref()) {
        (1, None | Some("low")) => Band::Low,
        (1, Some("high")) => Band::High,
        (2, None | Some("bandpass")) => Band::Pass,
        (2, Some("stop" | "bandstop")) => Band::Stop,
        _ => {
            return err(
                "Abaco:domain",
                format!("`{name}`: use a scalar cutoff with 'low' or 'high', or [low high] with 'bandpass' or 'stop'"),
            )
        }
    };
    if wn.iter().any(|&w| !(w > 0.0 && w < 1.0)) {
        return err(
            "Abaco:domain",
            format!("`{name}`: cutoffs are fractions of the Nyquist frequency, strictly between 0 and 1"),
        );
    }
    if wn.len() == 2 && wn[0] >= wn[1] {
        return err(
            "Abaco:domain",
            format!("`{name}`: the band edges must increase"),
        );
    }
    Ok((band, wn))
}

fn order_arg(a: &[Value], name: &str) -> Result<usize> {
    let n = usize_arg(a, 0, name)?;
    if n == 0 || n > 32 {
        return err(
            "Abaco:domain",
            format!("`{name}`: the order must be 1 to 32"),
        );
    }
    Ok(n)
}

fn f_butter(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "butter")?;
    let n = order_arg(a, "butter")?;
    let (band, wn) = band_args(a, 1, "butter")?;
    let poles: Vec<C> = (1..=n)
        .map(|k| C::from_polar(1.0, PI * (2 * k + n - 1) as f64 / (2 * n) as f64))
        .collect();
    let (b, den) = digital_from_prototype(Vec::new(), poles, 1.0, band, &wn)?;
    Ok(vec![Value::row(b), Value::row(den)])
}

fn f_cheby1(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 4, "cheby1")?;
    let n = order_arg(a, "cheby1")?;
    let rp = scalar_arg(a, 1, "cheby1")?;
    if !(rp > 0.0 && rp.is_finite()) {
        return err(
            "Abaco:domain",
            "`cheby1`: the passband ripple in dB must be positive",
        );
    }
    let (band, wn) = band_args(a, 2, "cheby1")?;
    let eps = (10f64.powf(rp / 10.0) - 1.0).sqrt();
    let mu = (1.0 / eps).asinh() / n as f64;
    let poles: Vec<C> = (1..=n)
        .map(|k| {
            let theta = PI * (2 * k - 1) as f64 / (2 * n) as f64;
            C::new(-mu.sinh() * theta.sin(), mu.cosh() * theta.cos())
        })
        .collect();
    let mut gain: f64 = poles.iter().map(|p| -p).product::<C>().re;
    if n.is_multiple_of(2) {
        gain /= (1.0 + eps * eps).sqrt();
    }
    let (b, den) = digital_from_prototype(Vec::new(), poles, gain, band, &wn)?;
    Ok(vec![Value::row(b), Value::row(den)])
}

fn sinc_pi(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

fn f_sinc(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "sinc")?;
    one(Value::Num(real_arg(a, 0, "sinc")?.map(|&x| sinc_pi(x))))
}

fn f_fir1(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 4, "fir1")?;
    let order = usize_arg(a, 0, "fir1")?;
    if order == 0 || order > 100_000 {
        return err(
            "Abaco:domain",
            "`fir1`: the order must be between 1 and 100000",
        );
    }
    let wn = real_arg(a, 1, "fir1")?.d;
    let mut kind = String::new();
    let mut win: Option<Vec<f64>> = None;
    for v in a.iter().skip(2) {
        match v {
            Value::Str(s) => kind = s.to_ascii_lowercase(),
            other => win = Some(other.to_real()?.d),
        }
    }
    let band = match (wn.len(), kind.as_str()) {
        (1, "" | "low") => Band::Low,
        (1, "high") => Band::High,
        (2, "" | "bandpass") => Band::Pass,
        (2, "stop" | "bandstop") => Band::Stop,
        _ => {
            return err(
                "Abaco:domain",
                "`fir1`: use a scalar cutoff with 'low' or 'high', or [low high] with 'bandpass' or 'stop'",
            )
        }
    };
    if wn.iter().any(|&w| !(w > 0.0 && w < 1.0)) || (wn.len() == 2 && wn[0] >= wn[1]) {
        return err(
            "Abaco:domain",
            "`fir1`: cutoffs are increasing fractions of the Nyquist frequency between 0 and 1",
        );
    }
    if matches!(band, Band::High | Band::Stop) && order % 2 == 1 {
        return err(
            "Abaco:domain",
            "`fir1`: a highpass or bandstop filter needs an even order",
        );
    }
    let len = order + 1;
    let w = match win {
        Some(w) if w.len() == len => w,
        Some(w) => {
            return err(
                "Abaco:size",
                format!(
                    "`fir1`: the window has {} points, the filter {len}",
                    w.len()
                ),
            )
        }
        None => window_vec("hamming", len, false, 0.0),
    };
    let mid = order as f64 / 2.0;
    let lowpass = |cut: f64, m: f64| cut * sinc_pi(cut * (m - mid));
    let mut h: Vec<f64> = (0..len)
        .map(|k| {
            let m = k as f64;
            match band {
                Band::Low => lowpass(wn[0], m),
                Band::High => sinc_pi(m - mid) - lowpass(wn[0], m),
                Band::Pass => lowpass(wn[1], m) - lowpass(wn[0], m),
                Band::Stop => sinc_pi(m - mid) - (lowpass(wn[1], m) - lowpass(wn[0], m)),
            }
        })
        .zip(&w)
        .map(|(v, wk)| v * wk)
        .collect();
    // Scale for unit gain at the centre of the first passband.
    let f0 = match band {
        Band::Low | Band::Stop => 0.0,
        Band::High => 1.0,
        Band::Pass => (wn[0] + wn[1]) / 2.0,
    };
    let gain: C = h
        .iter()
        .enumerate()
        .map(|(k, &v)| C::from_polar(v, -PI * f0 * k as f64))
        .sum();
    let g = gain.norm();
    if g == 0.0 || !g.is_finite() {
        return err(
            "Abaco:numeric",
            "`fir1`: the design has no gain in its passband",
        );
    }
    for v in &mut h {
        *v /= g;
    }
    one(Value::row(h))
}

// ---------- correlation and peaks ----------

fn f_xcorr(interp: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "xcorr")?;
    let x = real_arg(a, 0, "xcorr")?.d;
    let (y, mode_at) = match a.get(1) {
        Some(Value::Str(_)) | None => (x.clone(), 1),
        Some(_) => (real_arg(a, 1, "xcorr")?.d, 2),
    };
    let mode = match a.get(mode_at) {
        Some(Value::Str(s)) => s.to_ascii_lowercase(),
        _ => "none".into(),
    };
    let (n, m) = (x.len(), y.len());
    if n == 0 || m == 0 {
        return err("Abaco:domain", "`xcorr` needs non-empty signals");
    }
    interp.charge((n + m) as u64)?;
    let yr: Vec<f64> = y.iter().rev().copied().collect();
    let mut c = fft::convolve(&x, &yr).map_err(num_err)?;
    match mode.as_str() {
        "none" => {}
        "biased" => c.iter_mut().for_each(|v| *v /= n.max(m) as f64),
        "unbiased" => {
            let len = c.len();
            for (k, v) in c.iter_mut().enumerate() {
                let lag = (k as i64 - (m as i64 - 1)).unsigned_abs() as usize;
                let overlap = (n.max(m)).saturating_sub(lag).max(1);
                let _ = len;
                *v /= overlap as f64;
            }
        }
        "coeff" => {
            let ex: f64 = x.iter().map(|v| v * v).sum();
            let ey: f64 = y.iter().map(|v| v * v).sum();
            let s = (ex * ey).sqrt();
            if s > 0.0 {
                c.iter_mut().for_each(|v| *v /= s);
            }
        }
        other => {
            return err(
                "Abaco:domain",
                format!("`xcorr`: unknown scaling '{other}' (use none, biased, unbiased or coeff)"),
            )
        }
    }
    let lags: Vec<f64> = (0..c.len()).map(|k| k as f64 - (m as f64 - 1.0)).collect();
    Ok(vec![Value::col(c), Value::col(lags)])
}

fn f_findpeaks(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 9, "findpeaks")?;
    let x = real_arg(a, 0, "findpeaks")?.d;
    let mut min_height = f64::NEG_INFINITY;
    let mut min_dist = 0usize;
    let mut min_prom = 0.0;
    let mut i = 1;
    while i + 1 < a.len() + 1 && i < a.len() {
        if let Value::Str(k) = &a[i] {
            let v = scalar_arg(a, i + 1, "findpeaks")?;
            match k.to_ascii_lowercase().as_str() {
                "minpeakheight" => min_height = v,
                "minpeakdistance" => min_dist = v.max(0.0) as usize,
                "minpeakprominence" => min_prom = v,
                other => {
                    return err(
                        "Abaco:domain",
                        format!("`findpeaks`: unknown option '{other}'"),
                    )
                }
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    let n = x.len();
    let mut cand: Vec<usize> = Vec::new();
    let mut k = 1;
    while k + 1 < n {
        if x[k] > x[k - 1] {
            // Walk across a flat top.
            let mut e = k;
            while e + 1 < n && x[e + 1] == x[k] {
                e += 1;
            }
            if e + 1 < n && x[e + 1] < x[k] {
                let idx = (k + e) / 2;
                if x[idx] >= min_height && prominence(&x, idx) >= min_prom {
                    cand.push(idx);
                }
            }
            k = e + 1;
        } else {
            k += 1;
        }
    }
    if min_dist > 0 {
        let mut order: Vec<usize> = (0..cand.len()).collect();
        order.sort_by(|&p, &q| x[cand[q]].total_cmp(&x[cand[p]]));
        let mut keep = vec![true; cand.len()];
        for &p in &order {
            if !keep[p] {
                continue;
            }
            for q in 0..cand.len() {
                if q != p && keep[q] && cand[p].abs_diff(cand[q]) < min_dist {
                    keep[q] = false;
                }
            }
        }
        cand = cand
            .into_iter()
            .zip(keep)
            .filter_map(|(c, k)| k.then_some(c))
            .collect();
    }
    let pks: Vec<f64> = cand.iter().map(|&i| x[i]).collect();
    let locs: Vec<f64> = cand.iter().map(|&i| (i + 1) as f64).collect();
    Ok(vec![Value::col(pks), Value::col(locs)])
}

/// How far a peak stands above the higher of the two lowest points between it and a higher peak.
fn prominence(x: &[f64], i: usize) -> f64 {
    let p = x[i];
    let mut left_min = p;
    for &v in x[..i].iter().rev() {
        if v > p {
            break;
        }
        left_min = left_min.min(v);
    }
    let mut right_min = p;
    for &v in &x[i + 1..] {
        if v > p {
            break;
        }
        right_min = right_min.min(v);
    }
    p - left_min.max(right_min)
}

fn f_unwrap(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "unwrap")?;
    let x = real_arg(a, 0, "unwrap")?;
    let tol = if a.len() > 1 {
        scalar_arg(a, 1, "unwrap")?
    } else {
        PI
    };
    let unwrap_col = |v: &[f64]| -> Vec<f64> {
        let mut out = Vec::with_capacity(v.len());
        let mut offset = 0.0;
        for (k, &p) in v.iter().enumerate() {
            if k > 0 {
                let d = p - v[k - 1];
                if d.abs() > tol {
                    offset -= 2.0 * PI * (d / (2.0 * PI)).round();
                }
            }
            out.push(p + offset);
        }
        out
    };
    let out = if x.r == 1 {
        Arr::row(unwrap_col(&x.d))
    } else {
        let mut d = Vec::with_capacity(x.d.len());
        for j in 0..x.c {
            d.extend(unwrap_col(&x.column(j)));
        }
        Arr::new(x.r, x.c, d)
    };
    one(Value::Num(out))
}

fn f_detrend(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "detrend")?;
    let x = real_arg(a, 0, "detrend")?;
    let degree = if a.len() > 1 {
        match &a[1] {
            Value::Str(s) if s.eq_ignore_ascii_case("constant") => 0,
            Value::Str(s) if s.eq_ignore_ascii_case("linear") => 1,
            other => other.scalar_usize("the degree")?,
        }
    } else {
        1
    };
    let detrend_col = |v: &[f64]| -> Result<Vec<f64>> {
        let n = v.len();
        if n <= degree {
            return err(
                "Abaco:domain",
                "`detrend` needs more samples than the degree",
            );
        }
        // Least squares on a scaled abscissa for conditioning.
        let t: Vec<f64> = (0..n)
            .map(|k| {
                if n > 1 {
                    2.0 * k as f64 / (n - 1) as f64 - 1.0
                } else {
                    0.0
                }
            })
            .collect();
        let m = degree + 1;
        let mut ata = vec![0.0; m * m];
        let mut atb = vec![0.0; m];
        for (k, &tk) in t.iter().enumerate() {
            let pows: Vec<f64> = (0..m).map(|p| tk.powi(p as i32)).collect();
            for i in 0..m {
                atb[i] += pows[i] * v[k];
                for j in 0..m {
                    ata[i * m + j] += pows[i] * pows[j];
                }
            }
        }
        let coef = solve(&Matrix::new(m, m, ata).map_err(num_err)?, &atb).map_err(num_err)?;
        Ok(t.iter()
            .zip(v)
            .map(|(&tk, &vk)| vk - coef.iter().rev().fold(0.0, |acc, &c| acc * tk + c))
            .collect())
    };
    let out = if x.r == 1 {
        Arr::row(detrend_col(&x.d)?)
    } else {
        let mut d = Vec::with_capacity(x.d.len());
        for j in 0..x.c {
            d.extend(detrend_col(&x.column(j))?);
        }
        Arr::new(x.r, x.c, d)
    };
    one(Value::Num(out))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("bartlett", f_bartlett),
    ("blackman", f_blackman),
    ("butter", f_butter),
    ("cheby1", f_cheby1),
    ("detrend", f_detrend),
    ("fft", f_fft),
    ("fft2", f_fft2),
    ("fftshift", f_fftshift),
    ("filter", f_filter),
    ("filtfilt", f_filtfilt),
    ("findpeaks", f_findpeaks),
    ("fir1", f_fir1),
    ("freqz", f_freqz),
    ("hamming", f_hamming),
    ("hann", f_hann),
    ("hanning", f_hann),
    ("ifft", f_ifft),
    ("ifft2", f_ifft2),
    ("ifftshift", f_ifftshift),
    ("kaiser", f_kaiser),
    ("nextpow2", f_nextpow2),
    ("rectwin", f_rectwin),
    ("sinc", f_sinc),
    ("unwrap", f_unwrap),
    ("xcorr", f_xcorr),
];
