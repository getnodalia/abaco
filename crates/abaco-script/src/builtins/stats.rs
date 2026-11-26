// SPDX-License-Identifier: MIT OR Apache-2.0
//! Statistics beyond the basic reductions, probability distributions and special functions.

use super::{arity, one, pick_dim, real_arg, reduce_dim, scalar_arg, usize_arg, Bf, Out};
use crate::error::err;
use crate::interp::Interp;
use crate::ops::zip;
use crate::value::{Arr, Value};
use nodalia_numerics::special as sp;
use std::f64::consts::PI;

fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| !x.is_nan()).collect();
    s.sort_by(f64::total_cmp);
    s
}

fn mean_of(v: &[f64]) -> f64 {
    if v.is_empty() {
        f64::NAN
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Linear-interpolated quantile with the plotting positions (k - 0.5) / n.
fn quantile_of(s: &[f64], p: f64) -> f64 {
    if s.is_empty() || !(0.0..=1.0).contains(&p) {
        return f64::NAN;
    }
    let n = s.len() as f64;
    let pos = p * n + 0.5;
    if pos <= 1.0 {
        s[0]
    } else if pos >= n {
        s[s.len() - 1]
    } else {
        let lo = pos.floor() as usize;
        s[lo - 1] + (pos - lo as f64) * (s[lo] - s[lo - 1])
    }
}

/// A reduction down columns (or over a vector) giving one row of results.
fn per_column(a: &[Value], name: &str, dim_at: usize, f: impl Fn(&[f64]) -> f64) -> Out {
    let x = real_arg(a, 0, name)?;
    let dim = pick_dim(a, dim_at, x.r, name)?;
    one(Value::Num(reduce_dim(&x, dim, f)))
}

fn central_moment(v: &[f64], k: i32) -> f64 {
    let m = mean_of(v);
    v.iter().map(|x| (x - m).powi(k)).sum::<f64>() / v.len() as f64
}

fn f_skewness(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "skewness")?;
    let unbiased = matches!(a.get(1), Some(v) if v.scalar_f64("flag")? == 0.0);
    per_column(a, "skewness", 2, |v| {
        let n = v.len() as f64;
        let s = central_moment(v, 3) / central_moment(v, 2).powf(1.5);
        if unbiased && n > 2.0 {
            s * (n * (n - 1.0)).sqrt() / (n - 2.0)
        } else {
            s
        }
    })
}

fn f_kurtosis(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "kurtosis")?;
    let unbiased = matches!(a.get(1), Some(v) if v.scalar_f64("flag")? == 0.0);
    per_column(a, "kurtosis", 2, |v| {
        let n = v.len() as f64;
        let k = central_moment(v, 4) / central_moment(v, 2).powi(2);
        if unbiased && n > 3.0 {
            3.0 + (n - 1.0) / ((n - 2.0) * (n - 3.0)) * ((n + 1.0) * k - 3.0 * (n - 1.0))
        } else {
            k
        }
    })
}

fn f_quantile(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "quantile")?;
    let x = real_arg(a, 0, "quantile")?;
    let p = real_arg(a, 1, "quantile")?;
    if x.r == 1 || x.c == 1 {
        let s = sorted(&x.d);
        return one(Value::Num(p.map(|&q| quantile_of(&s, q))));
    }
    // matrix: one row per probability, one column per data column
    let mut d = vec![0.0; p.d.len() * x.c];
    for j in 0..x.c {
        let s = sorted(&x.column(j));
        for (i, &q) in p.d.iter().enumerate() {
            d[j * p.d.len() + i] = quantile_of(&s, q);
        }
    }
    one(Value::mat(p.d.len(), x.c, d))
}

fn f_iqr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "iqr")?;
    per_column(a, "iqr", 1, |v| {
        let s = sorted(v);
        quantile_of(&s, 0.75) - quantile_of(&s, 0.25)
    })
}

fn f_mad(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "mad")?;
    let use_median = matches!(a.get(1), Some(v) if v.scalar_f64("flag")? == 1.0);
    per_column(a, "mad", 2, |v| {
        let s = sorted(v);
        if use_median {
            let m = quantile_of(&s, 0.5);
            quantile_of(
                &sorted(&s.iter().map(|x| (x - m).abs()).collect::<Vec<_>>()),
                0.5,
            )
        } else {
            let m = mean_of(&s);
            mean_of(&s.iter().map(|x| (x - m).abs()).collect::<Vec<_>>())
        }
    })
}

fn f_rms(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "rms")?;
    per_column(a, "rms", 1, |v| {
        (v.iter().map(|x| x * x).sum::<f64>() / v.len().max(1) as f64).sqrt()
    })
}

fn f_bounds(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "bounds")?;
    let lo = per_column(a, "bounds", 1, |v| {
        v.iter()
            .copied()
            .filter(|x| !x.is_nan())
            .fold(f64::NAN, |m, x| if m.is_nan() || x < m { x } else { m })
    })?;
    let hi = per_column(a, "bounds", 1, |v| {
        v.iter()
            .copied()
            .filter(|x| !x.is_nan())
            .fold(f64::NAN, |m, x| if m.is_nan() || x > m { x } else { m })
    })?;
    Ok(vec![lo[0].clone(), hi[0].clone()])
}

fn f_zscore(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "zscore")?;
    let x = real_arg(a, 0, "zscore")?;
    let col = |v: &[f64]| -> Vec<f64> {
        let m = mean_of(v);
        let n = v.len() as f64;
        let sd = if n > 1.0 {
            (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        } else {
            0.0
        };
        let sd = if sd == 0.0 { 1.0 } else { sd };
        v.iter().map(|x| (x - m) / sd).collect()
    };
    one(Value::Num(if x.r == 1 || x.c == 1 {
        Arr::new(x.r, x.c, col(&x.d))
    } else {
        let mut d = Vec::with_capacity(x.d.len());
        for j in 0..x.c {
            d.extend(col(&x.column(j)));
        }
        Arr::new(x.r, x.c, d)
    }))
}

fn f_rescale(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "rescale")?;
    let x = real_arg(a, 0, "rescale")?;
    let (l, u) = if a.len() == 3 {
        (scalar_arg(a, 1, "rescale")?, scalar_arg(a, 2, "rescale")?)
    } else {
        (0.0, 1.0)
    };
    let finite = x.d.iter().copied().filter(|v| !v.is_nan());
    let lo = finite.clone().fold(f64::INFINITY, f64::min);
    let hi = finite.fold(f64::NEG_INFINITY, f64::max);
    one(Value::Num(x.map(|&v| {
        if hi == lo {
            l
        } else {
            l + (v - lo) / (hi - lo) * (u - l)
        }
    })))
}

fn f_corr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "corr")?;
    let x = real_arg(a, 0, "corr")?;
    let y = if a.len() == 2 {
        real_arg(a, 1, "corr")?
    } else {
        x.clone()
    };
    if x.r != y.r {
        return err(
            "Abaco:size",
            "`corr` needs inputs with the same number of rows",
        );
    }
    let pearson = |u: &[f64], v: &[f64]| {
        let (mu, mv) = (mean_of(u), mean_of(v));
        let (mut suv, mut suu, mut svv) = (0.0, 0.0, 0.0);
        for (p, q) in u.iter().zip(v) {
            suv += (p - mu) * (q - mv);
            suu += (p - mu).powi(2);
            svv += (q - mv).powi(2);
        }
        suv / (suu * svv).sqrt()
    };
    let mut d = Vec::with_capacity(x.c * y.c);
    for j in 0..y.c {
        for i in 0..x.c {
            d.push(pearson(&x.column(i), &y.column(j)));
        }
    }
    one(Value::mat(x.c, y.c, d))
}

/// Centered moving window with the window shrunk at the ends.
fn moving(a: &[Value], name: &str, f: impl Fn(&[f64]) -> f64) -> Out {
    arity(a, 2, 4, name)?;
    let x = real_arg(a, 0, name)?;
    let w = real_arg(a, 1, name)?;
    let (nb, na) = match w.d.as_slice() {
        [k] if *k >= 1.0 && k.fract() == 0.0 => {
            let k = *k as usize;
            if k % 2 == 1 {
                ((k - 1) / 2, (k - 1) / 2)
            } else {
                (k / 2, k / 2 - 1)
            }
        }
        [b, a2] if *b >= 0.0 && *a2 >= 0.0 => (*b as usize, *a2 as usize),
        _ => {
            return err(
                "Abaco:domain",
                format!("`{name}`: the window must be a positive whole number or [before after]"),
            )
        }
    };
    let one_col = |v: &[f64]| -> Vec<f64> {
        (0..v.len())
            .map(|i| {
                let lo = i.saturating_sub(nb);
                let hi = (i + na + 1).min(v.len());
                f(&v[lo..hi])
            })
            .collect()
    };
    one(Value::Num(if x.r == 1 || x.c == 1 {
        Arr::new(x.r, x.c, one_col(&x.d))
    } else {
        let mut d = Vec::with_capacity(x.d.len());
        for j in 0..x.c {
            d.extend(one_col(&x.column(j)));
        }
        Arr::new(x.r, x.c, d)
    }))
}

fn f_movmean(_: &mut Interp, a: &[Value], _: usize) -> Out {
    moving(a, "movmean", mean_of)
}
fn f_movsum(_: &mut Interp, a: &[Value], _: usize) -> Out {
    moving(a, "movsum", |v| v.iter().sum())
}
fn f_movmedian(_: &mut Interp, a: &[Value], _: usize) -> Out {
    moving(a, "movmedian", |v| quantile_of(&sorted(v), 0.5))
}
fn f_movmax(_: &mut Interp, a: &[Value], _: usize) -> Out {
    moving(a, "movmax", |v| {
        v.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    })
}
fn f_movmin(_: &mut Interp, a: &[Value], _: usize) -> Out {
    moving(a, "movmin", |v| {
        v.iter().copied().fold(f64::INFINITY, f64::min)
    })
}
fn f_movstd(_: &mut Interp, a: &[Value], _: usize) -> Out {
    moving(a, "movstd", |v| {
        if v.len() < 2 {
            return 0.0;
        }
        let m = mean_of(v);
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
    })
}

fn f_histcounts(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "histcounts")?;
    let x: Vec<f64> = real_arg(a, 0, "histcounts")?
        .d
        .into_iter()
        .filter(|v| v.is_finite())
        .collect();
    let edges: Vec<f64> = match a.get(1) {
        Some(v) if v.numel() > 1 => v.to_real()?.d,
        other => {
            let bins = match other {
                Some(v) => v.scalar_usize("the number of bins")?,
                None => ((x.len() as f64).sqrt().ceil() as usize).clamp(1, 100),
            };
            if bins == 0 || bins > 1_000_000 {
                return err(
                    "Abaco:domain",
                    "`histcounts`: the number of bins must be between 1 and 1000000",
                );
            }
            let lo = x.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let (lo, hi) = if x.is_empty() {
                (0.0, 1.0)
            } else if lo == hi {
                (lo - 0.5, hi + 0.5)
            } else {
                (lo, hi)
            };
            (0..=bins)
                .map(|k| lo + (hi - lo) * k as f64 / bins as f64)
                .collect()
        }
    };
    if edges.len() < 2 || edges.windows(2).any(|w| w[1] <= w[0]) {
        return err(
            "Abaco:domain",
            "`histcounts`: the bin edges must be increasing",
        );
    }
    let mut counts = vec![0.0; edges.len() - 1];
    let last = edges[edges.len() - 1];
    for &v in &x {
        if v < edges[0] || v > last {
            continue;
        }
        let i = if v == last {
            edges.len() - 2
        } else {
            edges.partition_point(|&e| e <= v) - 1
        };
        counts[i] += 1.0;
    }
    Ok(vec![Value::row(counts), Value::row(edges)])
}

// ---------- special functions and distributions ----------

fn map1(a: &[Value], name: &str, f: impl Fn(f64) -> f64) -> Out {
    arity(a, 1, 1, name)?;
    one(Value::Num(real_arg(a, 0, name)?.map(|&x| f(x))))
}

fn map2(a: &[Value], name: &str, defaults: [f64; 1], f: impl Fn(f64, f64) -> f64) -> Out {
    arity(a, 1, 2, name)?;
    let x = real_arg(a, 0, name)?;
    let p = if a.len() > 1 {
        real_arg(a, 1, name)?
    } else {
        Arr::scalar(defaults[0])
    };
    one(Value::Num(zip(&x, &p, |&u, &v| f(u, v))?))
}

fn map3(a: &[Value], name: &str, defaults: [f64; 2], f: impl Fn(f64, f64, f64) -> f64) -> Out {
    arity(a, 1, 3, name)?;
    let x = real_arg(a, 0, name)?;
    let p = if a.len() > 1 {
        real_arg(a, 1, name)?
    } else {
        Arr::scalar(defaults[0])
    };
    let q = if a.len() > 2 {
        real_arg(a, 2, name)?
    } else {
        Arr::scalar(defaults[1])
    };
    let xp = zip(&x, &p, |&u, &v| (u, v))?;
    one(Value::Num(zip(&xp, &q, |&(u, v), &w| f(u, v, w))?))
}

fn f_erfinv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map1(a, "erfinv", sp::erfinv)
}
fn f_erfcinv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map1(a, "erfcinv", sp::erfcinv)
}

fn f_gammainc(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "gammainc")?;
    let upper = matches!(a.get(2), Some(Value::Str(s)) if s.eq_ignore_ascii_case("upper"));
    let x = real_arg(a, 0, "gammainc")?;
    let s = real_arg(a, 1, "gammainc")?;
    one(Value::Num(zip(&x, &s, |&x, &s| {
        if upper {
            sp::gamma_q(s, x)
        } else {
            sp::gamma_p(s, x)
        }
    })?))
}

fn f_betainc(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 4, "betainc")?;
    let upper = matches!(a.get(3), Some(Value::Str(s)) if s.eq_ignore_ascii_case("upper"));
    map3(&a[..3], "betainc", [1.0, 1.0], |x, p, q| {
        let v = sp::beta_inc(x, p, q);
        if upper {
            1.0 - v
        } else {
            v
        }
    })
}

fn f_beta(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "beta")?;
    let (x, y) = (real_arg(a, 0, "beta")?, real_arg(a, 1, "beta")?);
    one(Value::Num(zip(&x, &y, |&p, &q| {
        (sp::ln_gamma(p) + sp::ln_gamma(q) - sp::ln_gamma(p + q)).exp()
    })?))
}

fn f_betaln(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "betaln")?;
    let (x, y) = (real_arg(a, 0, "betaln")?, real_arg(a, 1, "betaln")?);
    one(Value::Num(zip(&x, &y, |&p, &q| {
        sp::ln_gamma(p) + sp::ln_gamma(q) - sp::ln_gamma(p + q)
    })?))
}

fn z(x: f64, mu: f64, sigma: f64) -> f64 {
    (x - mu) / sigma
}

fn f_normpdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "normpdf", [0.0, 1.0], |x, mu, s| {
        if s > 0.0 {
            (-0.5 * z(x, mu, s).powi(2)).exp() / (s * (2.0 * PI).sqrt())
        } else {
            f64::NAN
        }
    })
}
fn f_normcdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "normcdf", [0.0, 1.0], |x, mu, s| {
        if s > 0.0 {
            0.5 * sp::erfc(-z(x, mu, s) / std::f64::consts::SQRT_2)
        } else {
            f64::NAN
        }
    })
}
fn f_norminv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "norminv", [0.0, 1.0], |p, mu, s| {
        if s > 0.0 && (0.0..=1.0).contains(&p) {
            mu - s * std::f64::consts::SQRT_2 * sp::erfcinv(2.0 * p)
        } else {
            f64::NAN
        }
    })
}
fn f_exppdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "exppdf", [1.0], |x, mu| {
        if mu > 0.0 {
            if x < 0.0 {
                0.0
            } else {
                (-x / mu).exp() / mu
            }
        } else {
            f64::NAN
        }
    })
}
fn f_expcdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "expcdf", [1.0], |x, mu| {
        if mu > 0.0 {
            if x < 0.0 {
                0.0
            } else {
                -(-x / mu).exp_m1()
            }
        } else {
            f64::NAN
        }
    })
}
fn f_expinv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "expinv", [1.0], |p, mu| {
        if mu > 0.0 && (0.0..=1.0).contains(&p) {
            -mu * (-p).ln_1p()
        } else {
            f64::NAN
        }
    })
}
fn f_unifpdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "unifpdf", [0.0, 1.0], |x, lo, hi| {
        if hi > lo {
            if (lo..=hi).contains(&x) {
                1.0 / (hi - lo)
            } else {
                0.0
            }
        } else {
            f64::NAN
        }
    })
}
fn f_unifcdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "unifcdf", [0.0, 1.0], |x, lo, hi| {
        if hi > lo {
            ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            f64::NAN
        }
    })
}
fn f_chi2pdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "chi2pdf", [1.0], |x, k| {
        if k > 0.0 {
            if x < 0.0 {
                0.0
            } else if x == 0.0 {
                if k < 2.0 {
                    f64::INFINITY
                } else if k == 2.0 {
                    0.5
                } else {
                    0.0
                }
            } else {
                ((k / 2.0 - 1.0) * x.ln() - x / 2.0 - (k / 2.0) * 2f64.ln() - sp::ln_gamma(k / 2.0))
                    .exp()
            }
        } else {
            f64::NAN
        }
    })
}
fn f_chi2cdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "chi2cdf", [1.0], |x, k| {
        if k > 0.0 {
            if x <= 0.0 {
                0.0
            } else {
                sp::gamma_p(k / 2.0, x / 2.0)
            }
        } else {
            f64::NAN
        }
    })
}
fn f_tpdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "tpdf", [1.0], |x, nu| {
        if nu > 0.0 {
            (sp::ln_gamma((nu + 1.0) / 2.0)
                - sp::ln_gamma(nu / 2.0)
                - 0.5 * (nu * PI).ln()
                - (nu + 1.0) / 2.0 * (x * x / nu).ln_1p())
            .exp()
        } else {
            f64::NAN
        }
    })
}
fn f_tcdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "tcdf", [1.0], |x, nu| {
        if nu > 0.0 {
            let tail = 0.5 * sp::beta_inc(nu / (nu + x * x), nu / 2.0, 0.5);
            if x > 0.0 {
                1.0 - tail
            } else {
                tail
            }
        } else {
            f64::NAN
        }
    })
}
fn f_fcdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "fcdf", [1.0, 1.0], |x, d1, d2| {
        if d1 > 0.0 && d2 > 0.0 {
            if x <= 0.0 {
                0.0
            } else {
                sp::beta_inc(d1 * x / (d1 * x + d2), d1 / 2.0, d2 / 2.0)
            }
        } else {
            f64::NAN
        }
    })
}

fn ln_choose(n: f64, k: f64) -> f64 {
    sp::ln_gamma(n + 1.0) - sp::ln_gamma(k + 1.0) - sp::ln_gamma(n - k + 1.0)
}

fn f_binopdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "binopdf", [1.0, 0.5], |k, n, p| {
        if n < 0.0 || n.fract() != 0.0 || !(0.0..=1.0).contains(&p) {
            f64::NAN
        } else if k < 0.0 || k > n || k.fract() != 0.0 {
            0.0
        } else if p == 0.0 {
            f64::from(u8::from(k == 0.0))
        } else if p == 1.0 {
            f64::from(u8::from(k == n))
        } else {
            (ln_choose(n, k) + k * p.ln() + (n - k) * (1.0 - p).ln()).exp()
        }
    })
}
fn f_binocdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map3(a, "binocdf", [1.0, 0.5], |k, n, p| {
        if n < 0.0 || n.fract() != 0.0 || !(0.0..=1.0).contains(&p) {
            f64::NAN
        } else if k < 0.0 {
            0.0
        } else if k >= n {
            1.0
        } else {
            sp::beta_inc(1.0 - p, n - k.floor(), k.floor() + 1.0)
        }
    })
}
fn f_poisspdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "poisspdf", [1.0], |k, l| {
        if l < 0.0 {
            f64::NAN
        } else if k < 0.0 || k.fract() != 0.0 {
            0.0
        } else if l == 0.0 {
            f64::from(u8::from(k == 0.0))
        } else {
            (k * l.ln() - l - sp::ln_gamma(k + 1.0)).exp()
        }
    })
}
fn f_poisscdf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    map2(a, "poisscdf", [1.0], |k, l| {
        if l < 0.0 {
            f64::NAN
        } else if k < 0.0 {
            0.0
        } else if l == 0.0 {
            1.0
        } else {
            sp::gamma_q(k.floor() + 1.0, l)
        }
    })
}

/// `normrnd(mu, sigma, r, c)` and friends need the interpreter's generator.
fn f_normrnd(interp: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 0, 4, "normrnd")?;
    let mu = if a.is_empty() {
        0.0
    } else {
        scalar_arg(a, 0, "normrnd")?
    };
    let sigma = if a.len() > 1 {
        scalar_arg(a, 1, "normrnd")?
    } else {
        1.0
    };
    if !(sigma >= 0.0 && sigma.is_finite() && mu.is_finite()) {
        return err(
            "Abaco:domain",
            "`normrnd`: sigma must be finite and not negative",
        );
    }
    let r = if a.len() > 2 {
        usize_arg(a, 2, "normrnd")?
    } else {
        1
    };
    let c = if a.len() > 3 {
        usize_arg(a, 3, "normrnd")?
    } else {
        r
    };
    let count = r.checked_mul(c).filter(|&n| n <= crate::ops::MAX_ELEMENTS);
    let Some(count) = count else {
        return err("Abaco:limit", "`normrnd`: the result would be too large");
    };
    let d: Vec<f64> = (0..count)
        .map(|_| mu + sigma * interp.rng.normal())
        .collect();
    one(Value::mat(r, c, d))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("beta", f_beta),
    ("betainc", f_betainc),
    ("betaln", f_betaln),
    ("binocdf", f_binocdf),
    ("binopdf", f_binopdf),
    ("bounds", f_bounds),
    ("chi2cdf", f_chi2cdf),
    ("chi2pdf", f_chi2pdf),
    ("corr", f_corr),
    ("erfcinv", f_erfcinv),
    ("erfinv", f_erfinv),
    ("expcdf", f_expcdf),
    ("expinv", f_expinv),
    ("exppdf", f_exppdf),
    ("fcdf", f_fcdf),
    ("gammainc", f_gammainc),
    ("histcounts", f_histcounts),
    ("iqr", f_iqr),
    ("kurtosis", f_kurtosis),
    ("mad", f_mad),
    ("movmax", f_movmax),
    ("movmean", f_movmean),
    ("movmedian", f_movmedian),
    ("movmin", f_movmin),
    ("movstd", f_movstd),
    ("movsum", f_movsum),
    ("normcdf", f_normcdf),
    ("norminv", f_norminv),
    ("normpdf", f_normpdf),
    ("normrnd", f_normrnd),
    ("poisscdf", f_poisscdf),
    ("poisspdf", f_poisspdf),
    ("quantile", f_quantile),
    ("rescale", f_rescale),
    ("rms", f_rms),
    ("skewness", f_skewness),
    ("tcdf", f_tcdf),
    ("tpdf", f_tpdf),
    ("unifcdf", f_unifcdf),
    ("unifpdf", f_unifpdf),
    ("zscore", f_zscore),
];
