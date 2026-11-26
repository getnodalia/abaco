// SPDX-License-Identifier: MIT OR Apache-2.0
//! Element-wise mathematics: trigonometry, exponentials, rounding, special functions and
//! number theory.

use super::{arg, arity, bool_arr, one, real_arg, real_out, Bf, Out};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::ops::zip;
use crate::value::{Arr, Value};
use num_complex::Complex64;

type C = Complex64;

/// Applies a function that has a real and a complex form. A real argument outside the real
/// domain (for example `sqrt(-4)` or `asin(2)`) continues in the complex plane.
fn un(args: &[Value], name: &str, fr: fn(f64) -> f64, fc: fn(C) -> C, dom: fn(f64) -> bool) -> Out {
    arity(args, 1, 1, name)?;
    match arg(args, 0, name)? {
        Value::Cplx(a) => one(Value::cplx(a.map(|z| fc(*z)))),
        v => {
            let a = real_arg(std::slice::from_ref(v), 0, name)?;
            if a.d.iter().all(|&x| dom(x) || x.is_nan()) {
                real_out(a.map(|&x| fr(x)))
            } else {
                one(Value::cplx(a.map(|&x| fc(C::new(x, 0.0)))))
            }
        }
    }
}

/// A function applied to real numbers only (rounding, signs); complex values apply it to
/// both parts.
fn round_like(args: &[Value], name: &str, f: fn(f64) -> f64) -> Out {
    arity(args, 1, 1, name)?;
    match arg(args, 0, name)? {
        Value::Cplx(a) => one(Value::cplx(a.map(|z| C::new(f(z.re), f(z.im))))),
        v => real_out(real_arg(std::slice::from_ref(v), 0, name)?.map(|&x| f(x))),
    }
}

fn all(_: f64) -> bool {
    true
}

macro_rules! math {
    ($fname:ident, $name:literal, $fr:expr, $fc:expr, $dom:expr) => {
        fn $fname(_: &mut Interp, a: &[Value], _: usize) -> Out {
            un(a, $name, $fr, $fc, $dom)
        }
    };
}

macro_rules! rounder {
    ($fname:ident, $name:literal, $f:expr) => {
        fn $fname(_: &mut Interp, a: &[Value], _: usize) -> Out {
            round_like(a, $name, $f)
        }
    };
}

math!(f_sin, "sin", f64::sin, C::sin, all);
math!(f_cos, "cos", f64::cos, C::cos, all);
math!(f_tan, "tan", f64::tan, C::tan, all);
math!(f_asin, "asin", f64::asin, C::asin, |x| x.abs() <= 1.0);
math!(f_acos, "acos", f64::acos, C::acos, |x| x.abs() <= 1.0);
math!(f_atan, "atan", f64::atan, C::atan, all);
math!(f_sinh, "sinh", f64::sinh, C::sinh, all);
math!(f_cosh, "cosh", f64::cosh, C::cosh, all);
math!(f_tanh, "tanh", f64::tanh, C::tanh, all);
math!(f_asinh, "asinh", f64::asinh, C::asinh, all);
math!(f_acosh, "acosh", f64::acosh, C::acosh, |x| x >= 1.0);
math!(f_atanh, "atanh", f64::atanh, C::atanh, |x| x.abs() < 1.0);
math!(f_exp, "exp", f64::exp, C::exp, all);
math!(f_expm1, "expm1", f64::exp_m1, |z| z.exp() - 1.0, all);
math!(f_log, "log", f64::ln, C::ln, |x| x >= 0.0);
math!(
    f_log2,
    "log2",
    f64::log2,
    |z| z.ln() / std::f64::consts::LN_2,
    |x| x >= 0.0
);
math!(f_log10, "log10", f64::log10, C::log10_c, |x| x >= 0.0);
math!(f_log1p, "log1p", f64::ln_1p, |z| (z + 1.0).ln(), |x| x
    >= -1.0);
math!(f_sqrt, "sqrt", f64::sqrt, C::sqrt, |x| x >= 0.0);
math!(f_sec, "sec", |x| 1.0 / x.cos(), |z| 1.0 / z.cos(), all);
math!(f_csc, "csc", |x| 1.0 / x.sin(), |z| 1.0 / z.sin(), all);
math!(f_cot, "cot", |x| 1.0 / x.tan(), |z| 1.0 / z.tan(), all);

trait Log10C {
    fn log10_c(self) -> C;
}

impl Log10C for C {
    fn log10_c(self) -> C {
        self.ln() / std::f64::consts::LN_10
    }
}

rounder!(f_floor, "floor", f64::floor);
rounder!(f_ceil, "ceil", f64::ceil);
rounder!(f_round, "round", f64::round);
rounder!(f_fix, "fix", f64::trunc);

fn f_sign(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "sign")?;
    match arg(args, 0, "sign")? {
        Value::Cplx(a) => one(Value::cplx(a.map(|z| {
            if z.norm() == 0.0 {
                C::new(0.0, 0.0)
            } else {
                z / z.norm()
            }
        }))),
        v => real_out(real_arg(std::slice::from_ref(v), 0, "sign")?.map(|&x| {
            if x == 0.0 || x.is_nan() {
                x
            } else {
                x.signum()
            }
        })),
    }
}

fn f_abs(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "abs")?;
    match arg(args, 0, "abs")? {
        Value::Cplx(a) => real_out(a.map(|z| z.norm())),
        v => real_out(real_arg(std::slice::from_ref(v), 0, "abs")?.map(|x| x.abs())),
    }
}

fn f_real(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "real")?;
    match arg(args, 0, "real")? {
        Value::Cplx(a) => real_out(a.map(|z| z.re)),
        v => real_out(real_arg(std::slice::from_ref(v), 0, "real")?),
    }
}

fn f_imag(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "imag")?;
    match arg(args, 0, "imag")? {
        Value::Cplx(a) => real_out(a.map(|z| z.im)),
        v => real_out(real_arg(std::slice::from_ref(v), 0, "imag")?.map(|_| 0.0)),
    }
}

fn f_conj(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "conj")?;
    match arg(args, 0, "conj")? {
        Value::Cplx(a) => one(Value::Cplx(a.map(|z| z.conj()))),
        v => real_out(real_arg(std::slice::from_ref(v), 0, "conj")?),
    }
}

fn f_angle(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "angle")?;
    match arg(args, 0, "angle")? {
        Value::Cplx(a) => real_out(a.map(|z| z.arg())),
        v => real_out(real_arg(std::slice::from_ref(v), 0, "angle")?.map(|&x| {
            if x < 0.0 {
                std::f64::consts::PI
            } else {
                0.0
            }
        })),
    }
}

fn f_complex(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 2, 2, "complex")?;
    let (a, b) = (real_arg(args, 0, "complex")?, real_arg(args, 1, "complex")?);
    let z = zip(&a, &b, |&x, &y| C::new(x, y))?;
    // `complex` keeps the complex class even for zero imaginary parts.
    one(Value::Cplx(z))
}

fn real2(args: &[Value], name: &str, f: impl Fn(f64, f64) -> f64) -> Out {
    arity(args, 2, 2, name)?;
    let (a, b) = (real_arg(args, 0, name)?, real_arg(args, 1, name)?);
    real_out(zip(&a, &b, |&x, &y| f(x, y))?)
}

fn f_atan2(_: &mut Interp, a: &[Value], _: usize) -> Out {
    real2(a, "atan2", f64::atan2)
}

fn f_hypot(_: &mut Interp, a: &[Value], _: usize) -> Out {
    real2(a, "hypot", f64::hypot)
}

fn mod_f(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return x;
    }
    let r = x % y;
    if r != 0.0 && (r < 0.0) != (y < 0.0) {
        r + y
    } else {
        r
    }
}

fn f_mod(_: &mut Interp, a: &[Value], _: usize) -> Out {
    real2(a, "mod", mod_f)
}

fn f_rem(_: &mut Interp, a: &[Value], _: usize) -> Out {
    real2(a, "rem", |x, y| if y == 0.0 { x } else { x % y })
}

fn f_nthroot(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "nthroot")?;
    let (x, n) = (real_arg(a, 0, "nthroot")?, real_arg(a, 1, "nthroot")?);
    for &k in &n.d {
        if k.fract() != 0.0 || k == 0.0 {
            return err(
                "Abaco:domain",
                "`nthroot` needs a non-zero whole-number root",
            );
        }
    }
    for &v in &x.d {
        if v < 0.0 && n.d.iter().any(|k| k.rem_euclid(2.0) == 0.0) {
            return err(
                "Abaco:domain",
                "`nthroot` of a negative number needs an odd root",
            );
        }
    }
    real_out(zip(&x, &n, |&v, &k| {
        if v < 0.0 {
            -((-v).powf(1.0 / k))
        } else {
            v.powf(1.0 / k)
        }
    })?)
}

fn f_cbrt(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "cbrt")?;
    real_out(real_arg(a, 0, "cbrt")?.map(|x| x.cbrt()))
}

fn f_deg2rad(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "deg2rad")?;
    real_out(real_arg(a, 0, "deg2rad")?.map(|x| x.to_radians()))
}

fn f_rad2deg(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "rad2deg")?;
    real_out(real_arg(a, 0, "rad2deg")?.map(|x| x.to_degrees()))
}

fn deg_reduce(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

fn f_sind(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "sind")?;
    real_out(real_arg(a, 0, "sind")?.map(|&x| {
        let r = deg_reduce(x);
        if r % 180.0 == 0.0 {
            0.0
        } else if r == 90.0 {
            1.0
        } else if r == 270.0 {
            -1.0
        } else {
            r.to_radians().sin()
        }
    }))
}

fn f_cosd(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "cosd")?;
    real_out(real_arg(a, 0, "cosd")?.map(|&x| {
        let r = deg_reduce(x);
        if r == 90.0 || r == 270.0 {
            0.0
        } else if r == 0.0 {
            1.0
        } else if r == 180.0 {
            -1.0
        } else {
            r.to_radians().cos()
        }
    }))
}

fn f_tand(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "tand")?;
    real_out(real_arg(a, 0, "tand")?.map(|&x| {
        let r = deg_reduce(x);
        if r % 180.0 == 0.0 {
            0.0
        } else if r == 90.0 {
            f64::INFINITY
        } else if r == 270.0 {
            f64::NEG_INFINITY
        } else {
            r.to_radians().tan()
        }
    }))
}

// ---------- predicates ----------

fn pred(args: &[Value], name: &str, f: fn(f64) -> bool) -> Out {
    arity(args, 1, 1, name)?;
    match arg(args, 0, name)? {
        Value::Cplx(a) => bool_arr(a.map(|z| f(z.re) || f(z.im))),
        Value::Cell(_) | Value::Struct(_) | Value::Fn(_) => bool_arr(Arr::scalar(false)),
        v => bool_arr(real_arg(std::slice::from_ref(v), 0, name)?.map(|&x| f(x))),
    }
}

fn f_isnan(_: &mut Interp, a: &[Value], _: usize) -> Out {
    pred(a, "isnan", f64::is_nan)
}

fn f_isinf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    pred(a, "isinf", f64::is_infinite)
}

fn f_isfinite(_: &mut Interp, args: &[Value], _: usize) -> Out {
    arity(args, 1, 1, "isfinite")?;
    match arg(args, 0, "isfinite")? {
        Value::Cplx(a) => bool_arr(a.map(|z| z.re.is_finite() && z.im.is_finite())),
        v => bool_arr(real_arg(std::slice::from_ref(v), 0, "isfinite")?.map(|x| x.is_finite())),
    }
}

// ---------- special functions ----------

const LANCZOS: [f64; 9] = [
    0.999_999_999_999_809_9,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_311_6e-7,
];

/// The gamma function.
pub fn gamma(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == f64::INFINITY {
        return f64::INFINITY;
    }
    if x <= 0.0 && x.fract() == 0.0 {
        return if x == 0.0 { f64::INFINITY } else { f64::NAN };
    }
    if x.fract() == 0.0 && x <= 171.0 {
        let mut p = 1.0;
        let mut k = 2.0;
        while k < x {
            p *= k;
            k += 1.0;
        }
        return p;
    }
    if x > 171.7 {
        return f64::INFINITY;
    }
    if x < 0.5 {
        return std::f64::consts::PI / ((std::f64::consts::PI * x).sin() * gamma(1.0 - x));
    }
    let x = x - 1.0;
    let mut a = LANCZOS[0];
    let t = x + 7.5;
    for (i, c) in LANCZOS.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    (2.0 * std::f64::consts::PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
}

/// The natural log of the absolute value of the gamma function.
pub fn lgamma(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        if x.fract() == 0.0 {
            return f64::INFINITY;
        }
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin().abs()).ln()
            - lgamma(1.0 - x);
    }
    if x < 0.5 {
        return lgamma(x + 1.0) - x.ln();
    }
    let x = x - 1.0;
    let mut a = LANCZOS[0];
    let t = x + 7.5;
    for (i, c) in LANCZOS.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The error function.
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x.abs() < 2.5 {
        // erf(x) = 2/sqrt(pi) e^(-x^2) sum 2^n x^(2n+1) / (2n+1)!!, all terms positive
        let x2 = x * x;
        let mut term = x;
        let mut sum = x;
        for n in 1..200 {
            term *= 2.0 * x2 / (2.0 * f64::from(n) + 1.0);
            sum += term;
            if term.abs() < 1e-17 * sum.abs() {
                break;
            }
        }
        2.0 / std::f64::consts::PI.sqrt() * (-x2).exp() * sum
    } else {
        x.signum() * (1.0 - erfc(x.abs()))
    }
}

/// The complementary error function.
pub fn erfc(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x < 2.5 {
        return 1.0 - erf(x);
    }
    // continued fraction, evaluated from the tail
    let mut f = x;
    for k in (1..=80).rev() {
        f = x + (f64::from(k) / 2.0) / f;
    }
    (-x * x).exp() / (std::f64::consts::PI.sqrt() * f)
}

fn f_gamma(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "gamma")?;
    real_out(real_arg(a, 0, "gamma")?.map(|&x| gamma(x)))
}

fn f_lgamma(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "gammaln")?;
    real_out(real_arg(a, 0, "gammaln")?.map(|&x| lgamma(x)))
}

fn f_erf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "erf")?;
    real_out(real_arg(a, 0, "erf")?.map(|&x| erf(x)))
}

fn f_erfc(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "erfc")?;
    real_out(real_arg(a, 0, "erfc")?.map(|&x| erfc(x)))
}

fn f_factorial(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "factorial")?;
    let x = real_arg(a, 0, "factorial")?;
    if x.d.iter().any(|&n| n < 0.0 || n.fract() != 0.0) {
        return err(
            "Abaco:domain",
            "`factorial` needs non-negative whole numbers",
        );
    }
    real_out(x.map(|&n| gamma(n + 1.0)))
}

fn binom(n: f64, k: f64) -> f64 {
    let k = k.min(n - k);
    if k < 0.0 {
        return 0.0;
    }
    let mut r = 1.0;
    let mut i = 1.0;
    while i <= k {
        r = r * (n - k + i) / i;
        i += 1.0;
    }
    r.round()
}

fn f_nchoosek(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "nchoosek")?;
    let k = super::scalar_arg(a, 1, "nchoosek")?;
    if k < 0.0 || k.fract() != 0.0 {
        return err("Abaco:domain", "`nchoosek` needs a non-negative whole k");
    }
    let n = real_arg(a, 0, "nchoosek")?;
    if n.is_scalar() {
        let nn = n.d.first().copied().unwrap_or(0.0);
        if nn < 0.0 || nn.fract() != 0.0 {
            return err("Abaco:domain", "`nchoosek` needs a non-negative whole n");
        }
        return super::num(binom(nn, k));
    }
    // all k-element combinations of the vector, one per row
    let v = n.d;
    let (len, k) = (v.len(), k as usize);
    if k > len {
        return one(Value::mat(0, k, Vec::new()));
    }
    let total = binom(len as f64, k as f64);
    if total * k as f64 > 5e6 {
        return err(
            "Abaco:limit",
            "`nchoosek` would produce too many combinations",
        );
    }
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        rows.push(idx.iter().filter_map(|&i| v.get(i).copied()).collect());
        let mut i = k;
        loop {
            if i == 0 {
                let m = rows.len();
                let mut d = vec![0.0; m * k];
                for (r, row) in rows.iter().enumerate() {
                    for (c, x) in row.iter().enumerate() {
                        d[c * m + r] = *x;
                    }
                }
                return one(Value::mat(m, k, d));
            }
            i -= 1;
            if idx.get(i).is_some_and(|&x| x < len - k + i) {
                break;
            }
        }
        let base = idx.get(i).copied().unwrap_or(0) + 1;
        for (off, slot) in idx.iter_mut().enumerate().skip(i) {
            *slot = base + (off - i);
        }
    }
}

// ---------- number theory ----------

fn whole(x: f64, name: &str) -> Result<u64> {
    if x.is_finite() && x >= 0.0 && x.fract() == 0.0 && x < 1.8e19 {
        Ok(x as u64)
    } else {
        err(
            "Abaco:domain",
            format!("`{name}` needs non-negative whole numbers"),
        )
    }
}

fn gcd_u(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn f_gcd(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "gcd")?;
    let (x, y) = (real_arg(a, 0, "gcd")?, real_arg(a, 1, "gcd")?);
    for &v in x.d.iter().chain(&y.d) {
        whole(v.abs(), "gcd")?;
    }
    real_out(zip(&x, &y, |&p, &q| {
        gcd_u(p.abs() as u64, q.abs() as u64) as f64
    })?)
}

fn f_lcm(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "lcm")?;
    let (x, y) = (real_arg(a, 0, "lcm")?, real_arg(a, 1, "lcm")?);
    for &v in x.d.iter().chain(&y.d) {
        whole(v.abs(), "lcm")?;
    }
    real_out(zip(&x, &y, |&p, &q| {
        let (p, q) = (p.abs() as u64, q.abs() as u64);
        let g = gcd_u(p, q);
        p.checked_div(g).map_or(0.0, |d| d as f64 * q as f64)
    })?)
}

fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    if n < 4 {
        return true;
    }
    if n.is_multiple_of(2) || n.is_multiple_of(3) {
        return false;
    }
    let mut i = 5u64;
    while i.saturating_mul(i) <= n {
        if n.is_multiple_of(i) || n.is_multiple_of(i + 2) {
            return false;
        }
        i += 6;
    }
    true
}

fn f_isprime(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "isprime")?;
    let x = real_arg(a, 0, "isprime")?;
    for &v in &x.d {
        whole(v, "isprime")?;
    }
    bool_arr(x.map(|&v| is_prime(v as u64)))
}

fn f_primes(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "primes")?;
    let n = super::scalar_arg(a, 0, "primes")?;
    if !(n.is_finite()) || n > 1e8 {
        return err("Abaco:limit", "`primes` is limited to 1e8");
    }
    let n = n.max(0.0) as usize;
    let mut sieve = vec![true; n + 1];
    let mut out = Vec::new();
    for i in 2..=n {
        if sieve.get(i).copied().unwrap_or(false) {
            out.push(i as f64);
            let mut j = i.saturating_mul(i);
            while j <= n {
                if let Some(s) = sieve.get_mut(j) {
                    *s = false;
                }
                j += i;
            }
        }
    }
    one(Value::row(out))
}

fn f_factor(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "factor")?;
    let mut n = whole(super::scalar_arg(a, 0, "factor")?, "factor")?;
    let mut out = Vec::new();
    if n < 4 {
        return one(Value::row(vec![n as f64]));
    }
    let mut p = 2u64;
    while p.saturating_mul(p) <= n {
        while n % p == 0 {
            out.push(p as f64);
            n /= p;
        }
        p += if p == 2 { 1 } else { 2 };
    }
    if n > 1 {
        out.push(n as f64);
    }
    one(Value::row(out))
}

fn f_power(_: &mut Interp, a: &[Value], _: usize) -> Out {
    binop(a, "power", crate::ast::BinOp::ElemPow)
}

fn f_times(_: &mut Interp, a: &[Value], _: usize) -> Out {
    binop(a, "times", crate::ast::BinOp::ElemMul)
}

fn f_plus(_: &mut Interp, a: &[Value], _: usize) -> Out {
    binop(a, "plus", crate::ast::BinOp::Add)
}

fn f_minus(_: &mut Interp, a: &[Value], _: usize) -> Out {
    binop(a, "minus", crate::ast::BinOp::Sub)
}

fn f_rdivide(_: &mut Interp, a: &[Value], _: usize) -> Out {
    binop(a, "rdivide", crate::ast::BinOp::ElemDiv)
}

fn f_mtimes(_: &mut Interp, a: &[Value], _: usize) -> Out {
    binop(a, "mtimes", crate::ast::BinOp::Mul)
}

fn binop(args: &[Value], name: &str, op: crate::ast::BinOp) -> Out {
    arity(args, 2, 2, name)?;
    one(crate::ops::binary(
        op,
        arg(args, 0, name)?,
        arg(args, 1, name)?,
    )?)
}

pub const TABLE: &[(&str, Bf)] = &[
    ("sin", f_sin),
    ("cos", f_cos),
    ("tan", f_tan),
    ("asin", f_asin),
    ("acos", f_acos),
    ("atan", f_atan),
    ("sinh", f_sinh),
    ("cosh", f_cosh),
    ("tanh", f_tanh),
    ("asinh", f_asinh),
    ("acosh", f_acosh),
    ("atanh", f_atanh),
    ("exp", f_exp),
    ("expm1", f_expm1),
    ("log", f_log),
    ("log2", f_log2),
    ("log10", f_log10),
    ("log1p", f_log1p),
    ("sqrt", f_sqrt),
    ("sec", f_sec),
    ("csc", f_csc),
    ("cot", f_cot),
    ("floor", f_floor),
    ("ceil", f_ceil),
    ("round", f_round),
    ("fix", f_fix),
    ("sign", f_sign),
    ("abs", f_abs),
    ("real", f_real),
    ("imag", f_imag),
    ("conj", f_conj),
    ("angle", f_angle),
    ("arg", f_angle),
    ("complex", f_complex),
    ("atan2", f_atan2),
    ("hypot", f_hypot),
    ("mod", f_mod),
    ("rem", f_rem),
    ("nthroot", f_nthroot),
    ("cbrt", f_cbrt),
    ("deg2rad", f_deg2rad),
    ("rad2deg", f_rad2deg),
    ("sind", f_sind),
    ("cosd", f_cosd),
    ("tand", f_tand),
    ("isnan", f_isnan),
    ("isinf", f_isinf),
    ("isfinite", f_isfinite),
    ("gamma", f_gamma),
    ("gammaln", f_lgamma),
    ("lgamma", f_lgamma),
    ("erf", f_erf),
    ("erfc", f_erfc),
    ("factorial", f_factorial),
    ("nchoosek", f_nchoosek),
    ("gcd", f_gcd),
    ("lcm", f_lcm),
    ("isprime", f_isprime),
    ("primes", f_primes),
    ("factor", f_factor),
    ("power", f_power),
    ("times", f_times),
    ("plus", f_plus),
    ("minus", f_minus),
    ("rdivide", f_rdivide),
    ("mtimes", f_mtimes),
];
