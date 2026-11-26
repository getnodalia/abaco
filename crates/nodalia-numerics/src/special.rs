// SPDX-License-Identifier: MIT OR Apache-2.0
//! Special functions: log-gamma, the regularised incomplete gamma and beta functions, and the
//! error function with its inverse. Series and continued fractions are iterated to a fixed
//! relative tolerance of about 1e-15 and return NaN rather than a wrong number when the
//! arguments are outside the domain or the iteration does not settle.

use std::f64::consts::PI;

const EPS: f64 = 1e-16;
const MAX_ITER: usize = 10_000;

/// `ln |Γ(x)|` (Lanczos approximation, reflection for `x < 0.5`).
pub fn ln_gamma(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 && x == x.floor() {
        return f64::INFINITY;
    }
    if x < 0.5 {
        // Γ(x) Γ(1-x) = π / sin(πx)
        return (PI / (PI * x).sin().abs()).ln() - ln_gamma(1.0 - x);
    }
    const G: f64 = 7.0;
    const C: [f64; 9] = [
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
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, &c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The regularised lower incomplete gamma function `P(a, x)` for `a > 0`, `x >= 0`.
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if a.is_nan() || x.is_nan() || a <= 0.0 || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return 1.0;
    }
    if x < a + 1.0 {
        gamma_series(a, x)
    } else {
        1.0 - gamma_cf(a, x)
    }
}

/// The regularised upper incomplete gamma function `Q(a, x) = 1 - P(a, x)`.
pub fn gamma_q(a: f64, x: f64) -> f64 {
    if a.is_nan() || x.is_nan() || a <= 0.0 || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return 1.0;
    }
    if x.is_infinite() {
        return 0.0;
    }
    if x < a + 1.0 {
        1.0 - gamma_series(a, x)
    } else {
        gamma_cf(a, x)
    }
}

fn gamma_series(a: f64, x: f64) -> f64 {
    let mut ap = a;
    let mut term = 1.0 / a;
    let mut sum = term;
    for _ in 0..MAX_ITER {
        ap += 1.0;
        term *= x / ap;
        sum += term;
        if term.abs() < sum.abs() * EPS {
            return sum * (-x + a * x.ln() - ln_gamma(a)).exp();
        }
    }
    f64::NAN
}

/// `Q(a, x)` by the modified Lentz continued fraction.
fn gamma_cf(a: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..MAX_ITER {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            return (-x + a * x.ln() - ln_gamma(a)).exp() * h;
        }
    }
    f64::NAN
}

/// The regularised incomplete beta function `I_x(a, b)` for `a, b > 0` and `0 <= x <= 1`.
pub fn beta_inc(x: f64, a: f64, b: f64) -> f64 {
    if x.is_nan() || a.is_nan() || b.is_nan() || a <= 0.0 || b <= 0.0 || !(0.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    if x == 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln();
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_cf(x, a, b) / a
    } else {
        1.0 - front * beta_cf(1.0 - x, b, a) / b
    }
}

fn beta_cf(x: f64, a: f64, b: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..MAX_ITER {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            return h;
        }
    }
    f64::NAN
}

/// The error function.
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let p = gamma_p(0.5, x * x);
    if x < 0.0 {
        -p
    } else {
        p
    }
}

/// The complementary error function, accurate in the tails.
pub fn erfc(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x >= 0.0 {
        gamma_q(0.5, x * x)
    } else {
        1.0 + gamma_p(0.5, x * x)
    }
}

/// The inverse error function on `(-1, 1)`; `±1` give `±∞`, anything else NaN.
pub fn erfinv(y: f64) -> f64 {
    if y.is_nan() || y.abs() > 1.0 {
        return f64::NAN;
    }
    if y == 1.0 {
        return f64::INFINITY;
    }
    if y == -1.0 {
        return f64::NEG_INFINITY;
    }
    if y == 0.0 {
        return 0.0;
    }
    if y.abs() > 0.5 {
        let x = erfcinv(1.0 - y.abs());
        return x.copysign(y);
    }
    let mut x = y * PI.sqrt() / 2.0;
    for _ in 0..6 {
        let f = erf(x) - y;
        let d = 2.0 / PI.sqrt() * (-x * x).exp();
        let step = f / d;
        // Halley correction: f'' = -2x f'.
        x -= step / (1.0 + x * step);
    }
    x
}

/// The inverse complementary error function on `[0, 2]`.
pub fn erfcinv(p: f64) -> f64 {
    if p.is_nan() || !(0.0..=2.0).contains(&p) {
        return f64::NAN;
    }
    if p == 0.0 {
        return f64::INFINITY;
    }
    if p == 2.0 {
        return f64::NEG_INFINITY;
    }
    if p > 1.0 {
        return -erfcinv(2.0 - p);
    }
    if p == 1.0 {
        return 0.0;
    }
    // Abramowitz & Stegun 26.2.23 for the normal quantile, then Newton–Halley on erfc.
    let q = p / 2.0;
    let t = (-2.0 * q.ln()).sqrt();
    let z = t
        - (2.515_517 + 0.802_853 * t + 0.010_328 * t * t)
            / (1.0 + 1.432_788 * t + 0.189_269 * t * t + 0.001_308 * t * t * t);
    let mut x = z / std::f64::consts::SQRT_2;
    for _ in 0..8 {
        let f = erfc(x) - p;
        let d = -2.0 / PI.sqrt() * (-x * x).exp();
        if d == 0.0 {
            break;
        }
        let step = f / d;
        x -= step / (1.0 + x * step);
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * (1.0 + b.abs()), "{a} vs {b}");
    }

    #[test]
    fn ln_gamma_matches_factorials_and_half() {
        close(ln_gamma(5.0), 24f64.ln(), 1e-13);
        close(ln_gamma(0.5), PI.sqrt().ln(), 1e-13);
        close(ln_gamma(10.0), 362_880f64.ln(), 1e-13);
        close(ln_gamma(-0.5), (2.0 * PI.sqrt()).ln(), 1e-12);
    }

    #[test]
    fn incomplete_gamma_has_closed_forms() {
        for &x in &[0.1, 0.5, 1.0, 3.0, 10.0] {
            close(gamma_p(1.0, x), 1.0 - (-x).exp(), 1e-13);
            close(gamma_p(0.5, x), erf(x.sqrt()), 1e-12);
            close(gamma_p(2.0, x), 1.0 - (-x).exp() * (1.0 + x), 1e-13);
            close(gamma_p(3.0, x) + gamma_q(3.0, x), 1.0, 1e-14);
        }
    }

    #[test]
    fn incomplete_beta_has_closed_forms() {
        for &x in &[0.05, 0.3, 0.5, 0.9] {
            close(beta_inc(x, 1.0, 1.0), x, 1e-13);
            close(beta_inc(x, 2.0, 2.0), 3.0 * x * x - 2.0 * x * x * x, 1e-13);
            close(beta_inc(x, 1.0, 3.0), 1.0 - (1.0 - x).powi(3), 1e-13);
            close(
                beta_inc(x, 2.5, 4.0) + beta_inc(1.0 - x, 4.0, 2.5),
                1.0,
                1e-13,
            );
        }
    }

    #[test]
    fn erf_values_and_inverse() {
        close(erf(1.0), 0.842_700_792_949_714_9, 1e-14);
        close(erfc(3.0), 2.209_049_699_858_544e-5, 1e-11);
        for &y in &[-0.999, -0.7, -0.2, 0.0, 0.01, 0.3, 0.5, 0.8, 0.99999] {
            close(erf(erfinv(y)), y, 1e-12);
        }
        for &p in &[1e-10, 1e-4, 0.1, 0.9, 1.5, 1.99] {
            close(erfc(erfcinv(p)), p, 1e-11);
        }
    }

    #[test]
    fn domain_errors_are_nan() {
        assert!(gamma_p(-1.0, 1.0).is_nan());
        assert!(beta_inc(1.5, 1.0, 1.0).is_nan());
        assert!(erfinv(1.5).is_nan());
        assert!(erfinv(1.0).is_infinite());
    }
}
