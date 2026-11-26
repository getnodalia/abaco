// SPDX-License-Identifier: MIT OR Apache-2.0
//! Root finding: bisection, Brent's method, Newton's method and polynomial roots.

use crate::eigen;
use crate::error::{NumError, Result};
use crate::matrix::Matrix;
use num_complex::Complex64;

/// A root and how it was found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Root {
    /// The root.
    pub x: f64,
    /// `f(x)` at the root.
    pub fx: f64,
    /// Iterations used.
    pub iterations: usize,
}

fn eval(f: &mut impl FnMut(f64) -> f64, x: f64) -> Result<f64> {
    let y = f(x);
    if y.is_finite() {
        Ok(y)
    } else {
        Err(NumError::NonFinite("function value"))
    }
}

fn check_bracket(f: &mut impl FnMut(f64) -> f64, a: f64, b: f64, tol: f64) -> Result<(f64, f64)> {
    if !(a.is_finite() && b.is_finite() && tol.is_finite() && tol > 0.0) || a == b {
        return Err(NumError::InvalidInput(
            "the bracket must be two different finite numbers and the tolerance positive".into(),
        ));
    }
    let (fa, fb) = (eval(f, a)?, eval(f, b)?);
    if fa != 0.0 && fb != 0.0 && (fa > 0.0) == (fb > 0.0) {
        return Err(NumError::NoBracket);
    }
    Ok((fa, fb))
}

/// Bisection on `[a, b]`, which must contain a sign change. Converges linearly but always.
pub fn bisect(
    mut f: impl FnMut(f64) -> f64,
    a: f64,
    b: f64,
    tol: f64,
    max_iter: usize,
) -> Result<Root> {
    let (mut fa, fb) = check_bracket(&mut f, a, b, tol)?;
    let (mut a, mut b) = (a, b);
    if fa == 0.0 {
        return Ok(Root {
            x: a,
            fx: 0.0,
            iterations: 0,
        });
    }
    if fb == 0.0 {
        return Ok(Root {
            x: b,
            fx: 0.0,
            iterations: 0,
        });
    }
    let bound = fa.abs().max(fb.abs());
    for it in 1..=max_iter {
        let m = 0.5 * (a + b);
        let fm = eval(&mut f, m)?;
        if fm == 0.0 || (b - a).abs() * 0.5 <= tol {
            return finish(m, fm, it, bound);
        }
        if (fm > 0.0) == (fa > 0.0) {
            a = m;
            fa = fm;
        } else {
            b = m;
        }
    }
    Err(NumError::NoConvergence {
        iterations: max_iter,
        residual: (b - a).abs(),
    })
}

/// A sign change can also be a pole (`tan` at `π/2`). The bracket's end values bound `|f|` near a
/// genuine root, so a result with a larger value is rejected instead of being reported as a root.
fn finish(x: f64, fx: f64, iterations: usize, bound: f64) -> Result<Root> {
    if fx.abs() > bound {
        return Err(NumError::InvalidInput(
            "the sign change is a discontinuity, not a root: |f| grew towards it".into(),
        ));
    }
    Ok(Root { x, fx, iterations })
}

/// Brent's method (Brent 1973): bisection's guarantee with inverse-quadratic speed.
pub fn brent(
    mut f: impl FnMut(f64) -> f64,
    a: f64,
    b: f64,
    tol: f64,
    max_iter: usize,
) -> Result<Root> {
    let (mut fa, mut fb) = check_bracket(&mut f, a, b, tol)?;
    let bound = fa.abs().max(fb.abs());
    let (mut a, mut b) = (a, b);
    if fa == 0.0 {
        return Ok(Root {
            x: a,
            fx: 0.0,
            iterations: 0,
        });
    }
    if fb == 0.0 {
        return Ok(Root {
            x: b,
            fx: 0.0,
            iterations: 0,
        });
    }
    let (mut c, mut fc) = (a, fa);
    let mut d = b - a;
    let mut e = d;
    for it in 1..=max_iter {
        if (fb > 0.0) == (fc > 0.0) {
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tol;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return finish(b, fb, it, bound);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            let s = fb / fa;
            let (mut p, mut q);
            if a == c {
                p = 2.0 * xm * s;
                q = 1.0 - s;
            } else {
                let qq = fa / fc;
                let r = fb / fc;
                p = s * (2.0 * xm * qq * (qq - r) - (b - a) * (r - 1.0));
                q = (qq - 1.0) * (r - 1.0) * (s - 1.0);
            }
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            if 2.0 * p < (3.0 * xm * q - (tol1 * q).abs()).min((e * q).abs()) {
                e = d;
                d = p / q;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        a = b;
        fa = fb;
        b += if d.abs() > tol1 {
            d
        } else if xm >= 0.0 {
            tol1
        } else {
            -tol1
        };
        fb = eval(&mut f, b)?;
    }
    Err(NumError::NoConvergence {
        iterations: max_iter,
        residual: fb.abs(),
    })
}

/// Newton's method from `x0`. Fails on a zero or non-finite derivative rather than jumping away.
pub fn newton(
    mut f: impl FnMut(f64) -> f64,
    mut df: impl FnMut(f64) -> f64,
    x0: f64,
    tol: f64,
    max_iter: usize,
) -> Result<Root> {
    if !(x0.is_finite() && tol.is_finite() && tol > 0.0) {
        return Err(NumError::InvalidInput(
            "the start must be finite and the tolerance positive".into(),
        ));
    }
    let mut x = x0;
    for it in 1..=max_iter {
        let fx = eval(&mut f, x)?;
        let d = eval(&mut df, x)?;
        if d == 0.0 {
            return Err(NumError::Singular { column: 0 });
        }
        let dx = fx / d;
        x -= dx;
        if !x.is_finite() {
            return Err(NumError::NonFinite("Newton iterate"));
        }
        if dx.abs() <= tol * (1.0 + x.abs()) {
            let fx = eval(&mut f, x)?;
            return Ok(Root {
                x,
                fx,
                iterations: it,
            });
        }
    }
    Err(NumError::NoConvergence {
        iterations: max_iter,
        residual: eval(&mut f, x)?.abs(),
    })
}

/// All complex roots of a polynomial given by its coefficients, **highest power first**
/// (`[1, -3, 2]` is `x² − 3x + 2`), as eigenvalues of the companion matrix, sorted by real part.
pub fn poly_roots(coeffs: &[f64]) -> Result<Vec<Complex64>> {
    if coeffs.iter().any(|c| !c.is_finite()) {
        return Err(NumError::NonFinite("polynomial coefficient"));
    }
    let first = coeffs.iter().position(|&c| c != 0.0).ok_or_else(|| {
        NumError::InvalidInput("the zero polynomial has every number as a root".into())
    })?;
    let c = &coeffs[first..];
    let degree = c.len() - 1;
    if degree == 0 {
        return Ok(Vec::new());
    }
    let lead = c[0];
    let mut m = Matrix::zeros(degree, degree)?;
    for j in 0..degree {
        m.set(0, j, -c[j + 1] / lead)?;
    }
    for i in 1..degree {
        m.set(i, i - 1, 1.0)?;
    }
    eigen::general(&m)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_two_by_all_three_methods() {
        let f = |x: f64| x * x - 2.0;
        let s = 2f64.sqrt();
        assert!((bisect(f, 0.0, 2.0, 1e-13, 200).unwrap().x - s).abs() < 1e-12);
        let b = brent(f, 0.0, 2.0, 1e-14, 100).unwrap();
        assert!((b.x - s).abs() < 1e-13 && b.iterations < 15, "{b:?}");
        let n = newton(f, |x| 2.0 * x, 1.0, 1e-14, 50).unwrap();
        assert!((n.x - s).abs() < 1e-14 && n.iterations < 10);
    }

    #[test]
    fn brent_beats_bisection_on_a_smooth_function() {
        let f = |x: f64| x.cos() - x;
        let b = brent(f, 0.0, 1.0, 1e-14, 100).unwrap();
        let s = bisect(f, 0.0, 1.0, 1e-14, 200).unwrap();
        assert!((b.x - 0.739_085_133_215_160_6).abs() < 1e-13);
        assert!(b.iterations < s.iterations);
    }

    #[test]
    fn same_sign_is_no_bracket() {
        assert!(matches!(
            brent(|x| x * x + 1.0, -1.0, 1.0, 1e-9, 50),
            Err(NumError::NoBracket)
        ));
        assert!(matches!(
            bisect(|x| x * x + 1.0, -1.0, 1.0, 1e-9, 50),
            Err(NumError::NoBracket)
        ));
    }

    #[test]
    fn a_pole_is_not_reported_as_a_root() {
        let r = brent(f64::tan, 1.0, 2.0, 1e-12, 200);
        assert!(r.is_err(), "{r:?}");
        let r = bisect(f64::tan, 1.0, 2.0, 1e-12, 200);
        assert!(r.is_err(), "{r:?}");
    }

    #[test]
    fn newton_refuses_a_flat_start_and_non_finite_values() {
        assert!(matches!(
            newton(|x| x * x - 2.0, |x| 2.0 * x, 0.0, 1e-9, 50),
            Err(NumError::Singular { .. })
        ));
        assert!(brent(|_| f64::NAN, 0.0, 1.0, 1e-9, 10).is_err());
        assert!(brent(|x| x, f64::NAN, 1.0, 1e-9, 10).is_err());
    }

    #[test]
    fn polynomial_roots() {
        let r = poly_roots(&[1.0, -3.0, 2.0]).unwrap();
        assert!((r[0].re - 1.0).abs() < 1e-13 && (r[1].re - 2.0).abs() < 1e-13);
        let r = poly_roots(&[1.0, 0.0, 1.0]).unwrap();
        assert!((r[0].im + 1.0).abs() < 1e-13 && (r[1].im - 1.0).abs() < 1e-13);
        // Leading zeros are skipped; a zero constant term gives a zero root.
        let r = poly_roots(&[0.0, 2.0, -4.0, 0.0]).unwrap();
        assert!(r[0].norm() < 1e-14 && (r[1].re - 2.0).abs() < 1e-13);
        // x^5 - 15x^4 + 85x^3 - 225x^2 + 274x - 120 = (x-1)(x-2)(x-3)(x-4)(x-5)
        let r = poly_roots(&[1.0, -15.0, 85.0, -225.0, 274.0, -120.0]).unwrap();
        for (z, w) in r.iter().zip(1..=5) {
            assert!(
                (z.re - f64::from(w)).abs() < 1e-9 && z.im.abs() < 1e-9,
                "{z}"
            );
        }
        assert!(poly_roots(&[0.0, 0.0]).is_err());
        assert!(poly_roots(&[f64::INFINITY, 1.0]).is_err());
    }
}
