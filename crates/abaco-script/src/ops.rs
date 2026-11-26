// SPDX-License-Identifier: MIT OR Apache-2.0
//! Operators: element-wise arithmetic with implicit expansion, comparison, logic, matrix
//! product, powers, division and transposes.

use crate::ast::{BinOp, UnOp};
use crate::error::{err, Result};
use crate::linalg;
use crate::value::{Arr, Value};
use num_complex::Complex64;

/// Largest array the operators will build.
pub const MAX_ELEMENTS: usize = 20_000_000;

/// The shape of `a op b` under implicit expansion.
pub fn broadcast_shape(a: (usize, usize), b: (usize, usize)) -> Result<(usize, usize)> {
    let dim = |x: usize, y: usize| -> Option<usize> {
        if x == y {
            Some(x)
        } else if x == 1 {
            Some(y)
        } else if y == 1 {
            Some(x)
        } else {
            None
        }
    };
    match (dim(a.0, b.0), dim(a.1, b.1)) {
        (Some(r), Some(c)) => {
            if r.saturating_mul(c) > MAX_ELEMENTS {
                return err(
                    "Abaco:limit",
                    format!(
                        "the result would have {} elements; the limit is {MAX_ELEMENTS}",
                        r.saturating_mul(c)
                    ),
                );
            }
            Ok((r, c))
        }
        _ => err(
            "Abaco:size",
            format!(
                "arrays have incompatible sizes for this operation: {}x{} and {}x{}",
                a.0, a.1, b.0, b.1
            ),
        ),
    }
}

/// Applies `f` over two arrays with implicit expansion.
pub fn zip<A: Clone, B: Clone, T: Clone>(
    a: &Arr<A>,
    b: &Arr<B>,
    f: impl Fn(&A, &B) -> T,
) -> Result<Arr<T>> {
    // fast paths
    if a.r == b.r && a.c == b.c {
        let d = a.d.iter().zip(&b.d).map(|(x, y)| f(x, y)).collect();
        return Ok(Arr { r: a.r, c: a.c, d });
    }
    if b.is_scalar() {
        if let Some(y) = b.d.first() {
            return Ok(Arr {
                r: a.r,
                c: a.c,
                d: a.d.iter().map(|x| f(x, y)).collect(),
            });
        }
    }
    if a.is_scalar() {
        if let Some(x) = a.d.first() {
            return Ok(Arr {
                r: b.r,
                c: b.c,
                d: b.d.iter().map(|y| f(x, y)).collect(),
            });
        }
    }
    let (r, c) = broadcast_shape((a.r, a.c), (b.r, b.c))?;
    let mut d = Vec::with_capacity(r * c);
    for j in 0..c {
        for i in 0..r {
            let (ai, aj) = (if a.r == 1 { 0 } else { i }, if a.c == 1 { 0 } else { j });
            let (bi, bj) = (if b.r == 1 { 0 } else { i }, if b.c == 1 { 0 } else { j });
            match (a.at(ai, aj), b.at(bi, bj)) {
                (Some(x), Some(y)) => d.push(f(x, y)),
                _ => {
                    return err(
                        "Abaco:size",
                        "arrays have incompatible sizes for this operation",
                    )
                }
            }
        }
    }
    Ok(Arr { r, c, d })
}

fn is_cplx(v: &Value) -> bool {
    matches!(v, Value::Cplx(_))
}

fn check_operand(op: BinOp, v: &Value) -> Result<()> {
    if v.is_numeric_like() {
        Ok(())
    } else {
        err(
            "Abaco:type",
            format!("`{}` does not work on {}", op_symbol(op), v.describe()),
        )
    }
}

/// The operator as written.
pub fn op_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::LDiv => "\\",
        BinOp::Pow => "^",
        BinOp::ElemMul => ".*",
        BinOp::ElemDiv => "./",
        BinOp::ElemLDiv => ".\\",
        BinOp::ElemPow => ".^",
        BinOp::Eq => "==",
        BinOp::Ne => "~=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&",
        BinOp::Or => "|",
    }
}

/// `a op b`.
pub fn binary(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    check_operand(op, a)?;
    check_operand(op, b)?;
    match op {
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => compare(op, a, b),
        BinOp::And | BinOp::Or => logic(op, a, b),
        BinOp::Mul => {
            if a.numel() == 1 || b.numel() == 1 {
                elementwise(BinOp::ElemMul, a, b)
            } else {
                matmul(a, b)
            }
        }
        BinOp::Div => {
            if b.numel() == 1 {
                elementwise(BinOp::ElemDiv, a, b)
            } else {
                // a / b = (b' \ a')'
                let bt = transpose(b, true)?;
                let at = transpose(a, true)?;
                let x = linalg::solve(&bt, &at)?;
                transpose(&x, true)
            }
        }
        BinOp::LDiv => {
            if a.numel() == 1 {
                elementwise(BinOp::ElemLDiv, a, b)
            } else {
                linalg::solve(a, b)
            }
        }
        BinOp::Pow => {
            if a.numel() == 1 && b.numel() == 1 {
                elementwise(BinOp::ElemPow, a, b)
            } else {
                linalg::mpower(a, b)
            }
        }
        BinOp::Add
        | BinOp::Sub
        | BinOp::ElemMul
        | BinOp::ElemDiv
        | BinOp::ElemLDiv
        | BinOp::ElemPow => elementwise(op, a, b),
    }
}

fn elementwise(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    let needs_complex =
        is_cplx(a) || is_cplx(b) || (matches!(op, BinOp::ElemPow) && real_pow_leaves_reals(a, b));
    if needs_complex {
        let x = a.to_complex()?;
        let y = b.to_complex()?;
        let r = match op {
            BinOp::Add => zip(&x, &y, |p, q| p + q)?,
            BinOp::Sub => zip(&x, &y, |p, q| p - q)?,
            BinOp::ElemMul => zip(&x, &y, |p, q| p * q)?,
            BinOp::ElemDiv => zip(&x, &y, |p, q| p / q)?,
            BinOp::ElemLDiv => zip(&x, &y, |p, q| q / p)?,
            BinOp::ElemPow => zip(&x, &y, cpow)?,
            _ => return err("Abaco:internal", "unexpected operator"),
        };
        return Ok(Value::cplx(r));
    }
    let x = a.to_real()?;
    let y = b.to_real()?;
    let r = match op {
        BinOp::Add => zip(&x, &y, |p, q| p + q)?,
        BinOp::Sub => zip(&x, &y, |p, q| p - q)?,
        BinOp::ElemMul => zip(&x, &y, |p, q| p * q)?,
        BinOp::ElemDiv => zip(&x, &y, |p, q| p / q)?,
        BinOp::ElemLDiv => zip(&x, &y, |p, q| q / p)?,
        BinOp::ElemPow => zip(&x, &y, |p, q| p.powf(*q))?,
        _ => return err("Abaco:internal", "unexpected operator"),
    };
    Ok(Value::Num(r))
}

/// `true` when some `x.^y` has a negative base and a fractional exponent.
fn real_pow_leaves_reals(a: &Value, b: &Value) -> bool {
    let (Ok(x), Ok(y)) = (a.to_real(), b.to_real()) else {
        return false;
    };
    let neg = x.d.iter().any(|&v| v < 0.0);
    neg && y.d.iter().any(|&v| v.fract() != 0.0 && v.is_finite())
}

/// Complex power with exact results for small whole exponents.
pub fn cpow(p: &Complex64, q: &Complex64) -> Complex64 {
    if q.im == 0.0 && q.re.fract() == 0.0 && q.re.abs() <= 64.0 {
        return p.powi(q.re as i32);
    }
    if p.re == 0.0 && p.im == 0.0 {
        return if q.re == 0.0 && q.im == 0.0 {
            Complex64::new(1.0, 0.0)
        } else {
            Complex64::new(0.0, 0.0)
        };
    }
    p.powc(*q)
}

fn compare(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    if is_cplx(a) || is_cplx(b) {
        let x = a.to_complex()?;
        let y = b.to_complex()?;
        let r = match op {
            BinOp::Eq => zip(&x, &y, |p, q| p == q)?,
            BinOp::Ne => zip(&x, &y, |p, q| p != q)?,
            BinOp::Lt => zip(&x, &y, |p, q| p.re < q.re)?,
            BinOp::Le => zip(&x, &y, |p, q| p.re <= q.re)?,
            BinOp::Gt => zip(&x, &y, |p, q| p.re > q.re)?,
            _ => zip(&x, &y, |p, q| p.re >= q.re)?,
        };
        return Ok(Value::Bool(r));
    }
    let x = a.to_real()?;
    let y = b.to_real()?;
    let r = match op {
        BinOp::Eq => zip(&x, &y, |p, q| p == q)?,
        BinOp::Ne => zip(&x, &y, |p, q| p != q)?,
        BinOp::Lt => zip(&x, &y, |p, q| p < q)?,
        BinOp::Le => zip(&x, &y, |p, q| p <= q)?,
        BinOp::Gt => zip(&x, &y, |p, q| p > q)?,
        _ => zip(&x, &y, |p, q| p >= q)?,
    };
    Ok(Value::Bool(r))
}

/// Element truth values.
pub fn truth(v: &Value) -> Result<Arr<bool>> {
    match v {
        Value::Bool(a) => Ok(a.clone()),
        Value::Num(a) => Ok(a.map(|&x| x != 0.0)),
        Value::Cplx(a) => Ok(a.map(|z| z.re != 0.0 || z.im != 0.0)),
        Value::Str(_) => Ok(v.to_real()?.map(|&x| x != 0.0)),
        other => err(
            "Abaco:type",
            format!("expected logical values, got {}", other.describe()),
        ),
    }
}

fn logic(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    let x = truth(a)?;
    let y = truth(b)?;
    let r = if op == BinOp::And {
        zip(&x, &y, |&p, &q| p && q)?
    } else {
        zip(&x, &y, |&p, &q| p || q)?
    };
    Ok(Value::Bool(r))
}

/// The matrix product.
pub fn matmul(a: &Value, b: &Value) -> Result<Value> {
    let (ar, ac) = a.dims();
    let (br, bc) = b.dims();
    if ac != br {
        return err(
            "Abaco:size",
            format!(
                "inner matrix dimensions must agree for `*`: {ar}x{ac} times {br}x{bc} (use `.*` for element-wise)"
            ),
        );
    }
    if ar.saturating_mul(bc) > MAX_ELEMENTS {
        return err("Abaco:limit", "the matrix product is too large");
    }
    if is_cplx(a) || is_cplx(b) {
        let x = a.to_complex()?;
        let y = b.to_complex()?;
        let mut d = vec![Complex64::new(0.0, 0.0); ar * bc];
        for j in 0..bc {
            for k in 0..ac {
                let Some(&bk) = y.at(k, j) else { continue };
                for i in 0..ar {
                    if let (Some(&aik), Some(slot)) = (x.at(i, k), d.get_mut(j * ar + i)) {
                        *slot += aik * bk;
                    }
                }
            }
        }
        return Ok(Value::cplx(Arr::new(ar, bc, d)));
    }
    let x = a.to_real()?;
    let y = b.to_real()?;
    let mut d = vec![0.0; ar * bc];
    for j in 0..bc {
        for k in 0..ac {
            let bk = y.d.get(j * br + k).copied().unwrap_or(0.0);
            if bk == 0.0 {
                continue;
            }
            let col = x.d.get(k * ar..(k + 1) * ar).unwrap_or(&[]);
            if let Some(out) = d.get_mut(j * ar..(j + 1) * ar) {
                for (o, &v) in out.iter_mut().zip(col) {
                    *o += v * bk;
                }
            }
        }
    }
    // keep NaN/Inf propagation honest when a zero entry skipped a non-finite one
    if x.d.iter().any(|v| !v.is_finite()) {
        for j in 0..bc {
            for i in 0..ar {
                let mut s = 0.0;
                for k in 0..ac {
                    s += x.d.get(k * ar + i).copied().unwrap_or(0.0)
                        * y.d.get(j * br + k).copied().unwrap_or(0.0);
                }
                if let Some(slot) = d.get_mut(j * ar + i) {
                    *slot = s;
                }
            }
        }
    }
    Ok(Value::Num(Arr::new(ar, bc, d)))
}

/// A unary operator.
pub fn unary(op: UnOp, v: &Value) -> Result<Value> {
    match op {
        UnOp::Neg => match v {
            Value::Cplx(a) => Ok(Value::Cplx(a.map(|z| -z))),
            other if other.is_numeric_like() => Ok(Value::Num(other.to_real()?.map(|&x| -x))),
            other => err("Abaco:type", format!("cannot negate {}", other.describe())),
        },
        UnOp::Pos => {
            if v.is_numeric_like() {
                match v {
                    Value::Bool(_) | Value::Str(_) => Ok(Value::Num(v.to_real()?)),
                    other => Ok(other.clone()),
                }
            } else {
                err(
                    "Abaco:type",
                    format!("unary `+` does not work on {}", v.describe()),
                )
            }
        }
        UnOp::Not => Ok(Value::Bool(truth(v)?.map(|&b| !b))),
    }
}

/// The (conjugate) transpose.
pub fn transpose(v: &Value, conj: bool) -> Result<Value> {
    Ok(match v {
        Value::Num(a) => Value::Num(a.transposed()),
        Value::Bool(a) => Value::Bool(a.transposed()),
        Value::Cplx(a) => {
            let t = a.transposed();
            Value::Cplx(if conj { t.map(Complex64::conj) } else { t })
        }
        Value::Cell(a) => Value::Cell(a.transposed()),
        Value::Str(s) => {
            // a column of characters is not representable; keep a 1xN string
            if s.chars().count() <= 1 {
                Value::Str(s.clone())
            } else {
                Value::Num(v.to_real()?.transposed())
            }
        }
        other => {
            return err(
                "Abaco:type",
                format!("cannot transpose {}", other.describe()),
            )
        }
    })
}
