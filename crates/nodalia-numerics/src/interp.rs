// SPDX-License-Identifier: MIT OR Apache-2.0
//! One-dimensional interpolation: linear, nearest, shape-preserving cubic (PCHIP) and cubic
//! splines with not-a-knot ends, all as piecewise polynomials that can be evaluated anywhere
//! (outside the data the first or last piece is extended).

use crate::error::{NumError, Result};

/// A piecewise cubic `p_i(t) = c0 + c1 t + c2 t² + c3 t³` with `t = x - breaks[i]`.
#[derive(Debug, Clone, PartialEq)]
pub struct PiecewiseCubic {
    /// The `n` break points, strictly increasing.
    pub breaks: Vec<f64>,
    /// Four coefficients per piece (`n - 1` pieces), lowest degree first.
    pub coefs: Vec<[f64; 4]>,
}

impl PiecewiseCubic {
    /// Builds the polynomial pieces from values and slopes at each knot.
    fn from_slopes(x: &[f64], y: &[f64], d: &[f64]) -> Self {
        let mut coefs = Vec::with_capacity(x.len().saturating_sub(1));
        for i in 0..x.len().saturating_sub(1) {
            let (xi, xj) = (x[i], x[i + 1]);
            let h = xj - xi;
            let delta = (y[i + 1] - y[i]) / h;
            let c2 = (3.0 * delta - 2.0 * d[i] - d[i + 1]) / h;
            let c3 = (d[i] - 2.0 * delta + d[i + 1]) / (h * h);
            coefs.push([y[i], d[i], c2, c3]);
        }
        Self {
            breaks: x.to_vec(),
            coefs,
        }
    }

    /// The piece that contains `xq` (clamped to the first or last).
    fn piece(&self, xq: f64) -> usize {
        let n = self.coefs.len();
        if n == 0 {
            return 0;
        }
        // Largest i with breaks[i] <= xq, clamped to 0..n-1.
        let idx = self.breaks.partition_point(|&b| b <= xq);
        idx.saturating_sub(1).min(n - 1)
    }

    /// The value at `xq`. NaN gives NaN.
    pub fn eval(&self, xq: f64) -> f64 {
        if xq.is_nan() {
            return f64::NAN;
        }
        let i = self.piece(xq);
        match (self.coefs.get(i), self.breaks.get(i)) {
            (Some(c), Some(&b)) => {
                let t = xq - b;
                c[0] + t * (c[1] + t * (c[2] + t * c[3]))
            }
            _ => f64::NAN,
        }
    }

    /// The first derivative at `xq`.
    pub fn deriv(&self, xq: f64) -> f64 {
        if xq.is_nan() {
            return f64::NAN;
        }
        let i = self.piece(xq);
        match (self.coefs.get(i), self.breaks.get(i)) {
            (Some(c), Some(&b)) => {
                let t = xq - b;
                c[1] + t * (2.0 * c[2] + t * 3.0 * c[3])
            }
            _ => f64::NAN,
        }
    }
}

fn check(x: &[f64], y: &[f64]) -> Result<()> {
    if x.len() != y.len() {
        return Err(NumError::DimensionMismatch(format!(
            "x has {} points and y has {}",
            x.len(),
            y.len()
        )));
    }
    if x.len() < 2 {
        return Err(NumError::InvalidInput(
            "interpolation needs at least two points".into(),
        ));
    }
    if x.iter().chain(y).any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("the data"));
    }
    if x.windows(2).any(|w| w[1] <= w[0]) {
        return Err(NumError::InvalidInput(
            "the x values must be strictly increasing".into(),
        ));
    }
    Ok(())
}

/// Shape-preserving piecewise cubic Hermite interpolation (Fritsch–Carlson slopes with the
/// three-point end conditions), so the interpolant never overshoots monotone data.
pub fn pchip(x: &[f64], y: &[f64]) -> Result<PiecewiseCubic> {
    check(x, y)?;
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let delta: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
    let mut d = vec![0.0; n];
    if n == 2 {
        d[0] = delta[0];
        d[1] = delta[0];
    } else {
        for k in 1..n - 1 {
            if delta[k - 1] * delta[k] > 0.0 {
                let w1 = 2.0 * h[k] + h[k - 1];
                let w2 = h[k] + 2.0 * h[k - 1];
                d[k] = (w1 + w2) / (w1 / delta[k - 1] + w2 / delta[k]);
            }
        }
        d[0] = end_slope(h[0], h[1], delta[0], delta[1]);
        d[n - 1] = end_slope(h[n - 2], h[n - 3], delta[n - 2], delta[n - 3]);
    }
    Ok(PiecewiseCubic::from_slopes(x, y, &d))
}

fn end_slope(h0: f64, h1: f64, del0: f64, del1: f64) -> f64 {
    let mut d = ((2.0 * h0 + h1) * del0 - h0 * del1) / (h0 + h1);
    if d.signum() != del0.signum() {
        d = 0.0;
    } else if del0.signum() != del1.signum() && d.abs() > 3.0 * del0.abs() {
        d = 3.0 * del0;
    }
    d
}

/// The cubic spline with not-a-knot end conditions (the parabola through three points,
/// a line through two).
pub fn spline(x: &[f64], y: &[f64]) -> Result<PiecewiseCubic> {
    check(x, y)?;
    let n = x.len();
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let delta: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
    let mut d = vec![0.0; n];
    if n == 2 {
        d[0] = delta[0];
        d[1] = delta[0];
    } else if n == 3 {
        let f012 = (delta[1] - delta[0]) / (x[2] - x[0]);
        for k in 0..3 {
            d[k] = delta[0] + f012 * (2.0 * x[k] - x[0] - x[1]);
        }
    } else {
        // Slopes d satisfy a tridiagonal system; the first and last rows encode not-a-knot.
        let mut a = vec![0.0; n]; // sub-diagonal: a[i] multiplies d[i-1] in row i
        let mut b = vec![0.0; n];
        let mut c = vec![0.0; n]; // super-diagonal: c[i] multiplies d[i+1] in row i
        let mut r = vec![0.0; n];
        b[0] = h[1];
        c[0] = h[0] + h[1];
        r[0] = ((h[0] + 2.0 * c[0]) * h[1] * delta[0] + h[0] * h[0] * delta[1]) / c[0];
        for i in 1..n - 1 {
            a[i] = h[i];
            b[i] = 2.0 * (h[i] + h[i - 1]);
            c[i] = h[i - 1];
            r[i] = 3.0 * (h[i] * delta[i - 1] + h[i - 1] * delta[i]);
        }
        // Row i couples d[i-1] (weight h[i]), d[i] and d[i+1] (weight h[i-1]).
        a[n - 1] = h[n - 2] + h[n - 3];
        b[n - 1] = h[n - 3];
        let an = a[n - 1];
        r[n - 1] = (h[n - 2] * h[n - 2] * delta[n - 3]
            + (2.0 * an + h[n - 2]) * h[n - 3] * delta[n - 2])
            / an;
        d = tridiagonal(&a, &b, &c, &r)?;
    }
    Ok(PiecewiseCubic::from_slopes(x, y, &d))
}

/// Thomas algorithm for a tridiagonal system.
fn tridiagonal(a: &[f64], b: &[f64], c: &[f64], r: &[f64]) -> Result<Vec<f64>> {
    let n = b.len();
    let mut cp = vec![0.0; n];
    let mut dp = vec![0.0; n];
    if b[0] == 0.0 {
        return Err(NumError::Singular { column: 0 });
    }
    cp[0] = c[0] / b[0];
    dp[0] = r[0] / b[0];
    for i in 1..n {
        let m = b[i] - a[i] * cp[i - 1];
        if m == 0.0 || !m.is_finite() {
            return Err(NumError::Singular { column: i });
        }
        cp[i] = c[i] / m;
        dp[i] = (r[i] - a[i] * dp[i - 1]) / m;
    }
    let mut x = vec![0.0; n];
    x[n - 1] = dp[n - 1];
    for i in (0..n - 1).rev() {
        x[i] = dp[i] - cp[i] * x[i + 1];
    }
    Ok(x)
}

/// Piecewise-linear interpolation (the ends extend the first and last segment).
pub fn linear(x: &[f64], y: &[f64], xq: f64) -> Result<f64> {
    check(x, y)?;
    if xq.is_nan() {
        return Ok(f64::NAN);
    }
    let i = x
        .partition_point(|&b| b <= xq)
        .saturating_sub(1)
        .min(x.len() - 2);
    let t = (xq - x[i]) / (x[i + 1] - x[i]);
    Ok(y[i] + t * (y[i + 1] - y[i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_reproduces_a_cubic_exactly() {
        let f = |t: f64| 2.0 - t + 0.5 * t * t - 0.25 * t * t * t;
        let x: Vec<f64> = vec![0.0, 0.4, 1.1, 1.5, 2.6, 3.0, 4.2];
        let y: Vec<f64> = x.iter().map(|&t| f(t)).collect();
        let p = spline(&x, &y).unwrap();
        for k in 0..=40 {
            let t = k as f64 * 0.1;
            assert!((p.eval(t) - f(t)).abs() < 1e-10, "t={t}");
        }
        assert!((p.deriv(2.0) - (-1.0 + 2.0 - 0.75 * 4.0)).abs() < 1e-9);
    }

    #[test]
    fn spline_of_a_sine_is_accurate_between_knots() {
        let x: Vec<f64> = (0..=20).map(|k| k as f64 * 0.3).collect();
        let y: Vec<f64> = x.iter().map(|t| t.sin()).collect();
        let p = spline(&x, &y).unwrap();
        // Error of a cubic spline is O(h^4); h = 0.3 gives well below 1e-3 away from the ends.
        for k in 5..55 {
            let t = k as f64 * 0.1;
            assert!((p.eval(t) - t.sin()).abs() < 1e-3, "t={t}");
        }
    }

    #[test]
    fn spline_passes_through_the_knots_with_three_and_two_points() {
        let p = spline(&[0.0, 1.0, 3.0], &[1.0, 2.0, 0.0]).unwrap();
        assert!((p.eval(0.0) - 1.0).abs() < 1e-12);
        assert!((p.eval(1.0) - 2.0).abs() < 1e-12);
        assert!((p.eval(3.0) - 0.0).abs() < 1e-12);
        let q = spline(&[0.0, 2.0], &[1.0, 5.0]).unwrap();
        assert!((q.eval(1.0) - 3.0).abs() < 1e-12);
    }

    #[test]
    fn pchip_never_overshoots_monotone_data() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [0.0, 0.0, 0.1, 1.0, 1.0, 1.0];
        let p = pchip(&x, &y).unwrap();
        let mut prev = f64::NEG_INFINITY;
        for k in 0..=100 {
            let v = p.eval(k as f64 * 0.05);
            assert!((-1e-12..=1.0 + 1e-12).contains(&v), "overshoot {v}");
            assert!(v >= prev - 1e-12, "not monotone");
            prev = v;
        }
    }

    #[test]
    fn linear_interpolates_and_extends() {
        let x = [0.0, 1.0, 2.0];
        let y = [0.0, 2.0, 2.0];
        assert!((linear(&x, &y, 0.5).unwrap() - 1.0).abs() < 1e-15);
        assert!((linear(&x, &y, -1.0).unwrap() + 2.0).abs() < 1e-15);
        assert!((linear(&x, &y, 3.0).unwrap() - 2.0).abs() < 1e-15);
    }

    #[test]
    fn rejects_bad_data() {
        assert!(spline(&[0.0, 0.0, 1.0], &[1.0, 2.0, 3.0]).is_err());
        assert!(pchip(&[0.0], &[1.0]).is_err());
        assert!(spline(&[0.0, 1.0], &[1.0, f64::NAN]).is_err());
    }
}
