// SPDX-License-Identifier: MIT OR Apache-2.0
//! Built-in functions.
//!
//! Angles: trigonometric functions take radians (a plain number) or an angle quantity
//! (`30 deg`); inverse functions return radians as a plain number (`asin(0.5) -> deg`).
//! `log(x)` is base 10 and `ln(x)` natural. `sqrt(m)` is `m^(1/2)` and `sqrt(m^2)` is `m`.
//! Real functions reject arguments outside their real domain instead of returning
//! complex numbers; pass a complex argument (`sqrt(-4 + 0i)`) to get a complex result.

use crate::ast::BinOp;
use crate::error::{ErrorKind, ExprError};
use crate::linalg;
use crate::ops::{self, err, finite_complex, finite_real, from_q, map_scalars, scalar_op, Ctx, R};
use crate::value::{Mat, Value};
use nodalia_units::Quantity;
use num_complex::Complex64;
use std::cmp::Ordering;
use std::f64::consts::PI;

const MANY: usize = usize::MAX;

/// Name, minimum and maximum number of arguments of every built-in function.
pub const BUILTINS: &[(&str, usize, usize)] = &[
    ("abs", 1, 1),
    ("acos", 1, 1),
    ("acosh", 1, 1),
    ("arg", 1, 1),
    ("asin", 1, 1),
    ("asinh", 1, 1),
    ("atan", 1, 1),
    ("atan2", 2, 2),
    ("atanh", 1, 1),
    ("cbrt", 1, 1),
    ("ceil", 1, 1),
    ("complex", 2, 2),
    ("conj", 1, 1),
    ("cos", 1, 1),
    ("cosh", 1, 1),
    ("cross", 2, 2),
    ("det", 1, 1),
    ("dot", 2, 2),
    ("exp", 1, 1),
    ("eye", 1, 2),
    ("factorial", 1, 1),
    ("floor", 1, 1),
    ("gamma", 1, 1),
    ("hypot", 2, 2),
    ("if", 3, 3),
    ("im", 1, 1),
    ("inv", 1, 1),
    ("length", 1, 1),
    ("linspace", 3, 3),
    ("ln", 1, 1),
    ("log", 1, 2),
    ("log10", 1, 1),
    ("log2", 1, 1),
    ("logspace", 3, 3),
    ("max", 1, MANY),
    ("mean", 1, MANY),
    ("median", 1, MANY),
    ("min", 1, MANY),
    ("mod", 2, 2),
    ("norm", 1, 1),
    ("ones", 1, 2),
    ("prod", 1, MANY),
    ("range", 2, 3),
    ("re", 1, 1),
    ("round", 1, 1),
    ("sign", 1, 1),
    ("sin", 1, 1),
    ("sinh", 1, 1),
    ("size", 1, 1),
    ("solve", 2, 2),
    ("sqrt", 1, 1),
    ("std", 1, MANY),
    ("sum", 1, MANY),
    ("tan", 1, 1),
    ("tanh", 1, 1),
    ("transpose", 1, 1),
    ("trunc", 1, 1),
    ("var", 1, MANY),
    ("zeros", 1, 2),
];

/// `true` when `name` is a built-in function.
pub fn is_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|(n, _, _)| *n == name)
}

fn arity(name: &str) -> Option<(usize, usize)> {
    BUILTINS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, a, b)| (*a, *b))
}

/// Calls built-in `name`. `None` if there is no such function.
pub fn call(ctx: &mut Ctx, name: &str, args: &[Value]) -> Option<R<Value>> {
    let (lo, hi) = arity(name)?;
    if args.len() < lo || args.len() > hi {
        let want = if lo == hi {
            format!("{lo} argument{}", if lo == 1 { "" } else { "s" })
        } else if hi == MANY {
            format!("at least {lo} argument{}", if lo == 1 { "" } else { "s" })
        } else {
            format!("{lo} to {hi} arguments")
        };
        return Some(Err(err(
            ErrorKind::Arity,
            format!("`{name}` takes {want}, got {}", args.len()),
        )));
    }
    Some(dispatch(ctx, name, args))
}

fn arg0(args: &[Value]) -> R<&Value> {
    args.first()
        .ok_or_else(|| err(ErrorKind::Arity, "missing argument"))
}

fn arg1(args: &[Value]) -> R<&Value> {
    args.get(1)
        .ok_or_else(|| err(ErrorKind::Arity, "missing second argument"))
}

fn dispatch(ctx: &mut Ctx, name: &str, args: &[Value]) -> R<Value> {
    match name {
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "asinh"
        | "acosh" | "atanh" | "exp" | "ln" | "log10" | "log2" => math1(ctx, name, arg0(args)?),
        "log" => {
            if args.len() == 2 {
                let x = math1(ctx, "ln", arg0(args)?)?;
                let b = math1(ctx, "ln", arg1(args)?)?;
                ops::binary(ctx, BinOp::Div, &x, &b)
            } else {
                math1(ctx, "log10", arg0(args)?)
            }
        }
        "sqrt" | "cbrt" | "abs" | "sign" | "floor" | "ceil" | "round" | "trunc" => {
            unary_q(ctx, name, arg0(args)?)
        }
        "re" | "im" | "conj" | "arg" => complex_parts(ctx, name, arg0(args)?),
        "complex" => {
            let re = real_arg(arg0(args)?, "complex")?;
            let im = real_arg(arg1(args)?, "complex")?;
            finite_complex(Complex64::new(re, im))
        }
        "gamma" => map_scalars(ctx, arg0(args)?, &mut |x| {
            finite_real(gamma(real_arg(x, "gamma")?)?)
        }),
        "factorial" => map_scalars(ctx, arg0(args)?, &mut |x| {
            finite_real(factorial(real_arg(x, "factorial")?)?)
        }),
        "atan2" => {
            let (y, x) = same_dim_pair(arg0(args)?, arg1(args)?, "atan2")?;
            finite_real(y.atan2(x))
        }
        "hypot" => hypot(arg0(args)?, arg1(args)?),
        "mod" => modulo(arg0(args)?, arg1(args)?),
        "sum" => fold(ctx, args, "sum", BinOp::Add),
        "prod" => fold(ctx, args, "prod", BinOp::Mul),
        "mean" => {
            let items = gather(ctx, args, "mean")?;
            let s = fold_items(&items, BinOp::Add, "mean")?;
            scalar_op(BinOp::Div, &s, &Value::Real(items.len() as f64))
        }
        "median" => median(ctx, args),
        "min" => extreme(ctx, args, "min", Ordering::Less),
        "max" => extreme(ctx, args, "max", Ordering::Greater),
        "var" => variance(ctx, args, "var").map(|(v, _)| v),
        "std" => {
            let (v, _) = variance(ctx, args, "std")?;
            unary_q(ctx, "sqrt", &v)
        }
        "norm" => norm(ctx, arg0(args)?),
        "dot" => dot(ctx, arg0(args)?, arg1(args)?),
        "cross" => cross(arg0(args)?, arg1(args)?),
        "transpose" => transpose(arg0(args)?),
        "det" => linalg::det(ctx, arg0(args)?),
        "inv" => linalg::inverse(ctx, arg0(args)?),
        "solve" => linalg::solve(ctx, arg0(args)?, arg1(args)?),
        "zeros" => filled(ctx, args, "zeros", 0.0),
        "ones" => filled(ctx, args, "ones", 1.0),
        "eye" => eye(ctx, args),
        "length" => Ok(Value::Real(match arg0(args)? {
            Value::Vector(v) => v.len() as f64,
            Value::Matrix(m) => m.rows.max(m.cols) as f64,
            Value::Str(s) => s.chars().count() as f64,
            _ => 1.0,
        })),
        "size" => {
            let (r, c) = match arg0(args)? {
                Value::Vector(v) => (v.len(), 1),
                Value::Matrix(m) => (m.rows, m.cols),
                _ => (1, 1),
            };
            Ok(Value::Vector(vec![
                Value::Real(r as f64),
                Value::Real(c as f64),
            ]))
        }
        "linspace" => linspace(ctx, args, false),
        "logspace" => linspace(ctx, args, true),
        "range" => {
            if args.len() == 3 {
                range(ctx, arg0(args)?, args.get(2), arg1(args)?)
            } else {
                range(ctx, arg0(args)?, None, arg1(args)?)
            }
        }
        "if" => Err(err(
            ErrorKind::Type,
            "internal: `if` is handled by the evaluator",
        )),
        other => Err(err(ErrorKind::Name, format!("unknown function `{other}`"))),
    }
}

// ---------- argument helpers ----------

fn describe_q(q: &Quantity) -> String {
    format!(
        "{} ({})",
        q.display_unit()
            .map_or_else(|| q.dimension().to_string(), ToString::to_string),
        q.dimension().describe()
    )
}

/// A real number from a number or a dimensionless quantity.
pub(crate) fn real_arg(v: &Value, fname: &str) -> R<f64> {
    match v {
        Value::Real(x) => Ok(*x),
        Value::Quantity(q) if q.dimension().is_dimensionless() => Ok(q.si_value()),
        Value::Quantity(q) => Err(err(
            ErrorKind::Dimension,
            format!(
                "`{fname}` needs a dimensionless argument, got {}",
                describe_q(q)
            ),
        )),
        Value::Complex(_) => Err(err(
            ErrorKind::Type,
            format!("`{fname}` needs a real number, got a complex number"),
        )),
        other => Err(err(
            ErrorKind::Type,
            format!("`{fname}` needs a number, got {}", other.type_name()),
        )),
    }
}

fn int_arg(v: &Value, fname: &str, max: usize) -> R<usize> {
    let x = real_arg(v, fname)?;
    if x.fract() != 0.0 || x < 0.0 {
        return Err(err(
            ErrorKind::Domain,
            format!("`{fname}` needs a whole number of at least 0, got {x}"),
        ));
    }
    if x > max as f64 {
        return Err(err(
            ErrorKind::Limit,
            format!("`{fname}` was asked for {x}, but the limit is {max} elements"),
        ));
    }
    Ok(x as usize)
}

/// Two reals or two quantities of one dimension, as SI values.
fn same_dim_pair(a: &Value, b: &Value, fname: &str) -> R<(f64, f64)> {
    match (a.as_quantity(), b.as_quantity()) {
        (Some(x), Some(y)) => {
            if x.dimension() != y.dimension() {
                x.compare(&y).map_err(ExprError::from)?;
            }
            Ok((x.si_value(), y.si_value()))
        }
        _ => Err(err(
            ErrorKind::Type,
            format!("`{fname}` needs two numbers or quantities of one dimension"),
        )),
    }
}

// ---------- elementwise math ----------

type RealFn = fn(f64) -> Result<f64, &'static str>;
type CplxFn = fn(Complex64) -> Complex64;

fn lookup1(name: &str) -> Option<(RealFn, Option<CplxFn>)> {
    Some(match name {
        "sin" => (|x| Ok(x.sin()), Some(|z| z.sin())),
        "cos" => (|x| Ok(x.cos()), Some(|z| z.cos())),
        "tan" => (|x| Ok(x.tan()), Some(|z| z.tan())),
        "atan" => (|x| Ok(x.atan()), Some(|z| z.atan())),
        "sinh" => (|x| Ok(x.sinh()), Some(|z| z.sinh())),
        "cosh" => (|x| Ok(x.cosh()), Some(|z| z.cosh())),
        "tanh" => (|x| Ok(x.tanh()), Some(|z| z.tanh())),
        "asinh" => (|x| Ok(x.asinh()), Some(|z| z.asinh())),
        "exp" => (|x| Ok(x.exp()), Some(|z| z.exp())),
        "asin" => (
            |x| {
                if x.abs() > 1.0 {
                    Err("of a value outside -1..1 (use a complex argument)")
                } else {
                    Ok(x.asin())
                }
            },
            Some(|z| z.asin()),
        ),
        "acos" => (
            |x| {
                if x.abs() > 1.0 {
                    Err("of a value outside -1..1 (use a complex argument)")
                } else {
                    Ok(x.acos())
                }
            },
            Some(|z| z.acos()),
        ),
        "acosh" => (
            |x| {
                if x < 1.0 {
                    Err("of a value below 1 (use a complex argument)")
                } else {
                    Ok(x.acosh())
                }
            },
            Some(|z| z.acosh()),
        ),
        "atanh" => (
            |x| {
                if x.abs() >= 1.0 {
                    Err("of a value outside -1..1 (use a complex argument)")
                } else {
                    Ok(x.atanh())
                }
            },
            Some(|z| z.atanh()),
        ),
        "ln" => (
            |x| {
                if x < 0.0 {
                    Err("of a negative real (use a complex argument)")
                } else if x == 0.0 {
                    Err("of zero (minus infinity)")
                } else {
                    Ok(x.ln())
                }
            },
            Some(|z| z.ln()),
        ),
        "log10" => (
            |x| {
                if x < 0.0 {
                    Err("of a negative real (use a complex argument)")
                } else if x == 0.0 {
                    Err("of zero (minus infinity)")
                } else {
                    Ok(x.log10())
                }
            },
            Some(|z| z.ln() / std::f64::consts::LN_10),
        ),
        "log2" => (
            |x| {
                if x < 0.0 {
                    Err("of a negative real (use a complex argument)")
                } else if x == 0.0 {
                    Err("of zero (minus infinity)")
                } else {
                    Ok(x.log2())
                }
            },
            Some(|z| z.ln() / std::f64::consts::LN_2),
        ),
        "sqrt" => (
            |x| {
                if x < 0.0 {
                    Err("of a negative real (use a complex argument)")
                } else {
                    Ok(x.sqrt())
                }
            },
            Some(|z| z.sqrt()),
        ),
        "cbrt" => (|x| Ok(x.cbrt()), Some(|z| z.cbrt())),
        _ => return None,
    })
}

fn math1(ctx: &mut Ctx, name: &str, v: &Value) -> R<Value> {
    let Some((rf, cf)) = lookup1(name) else {
        return Err(err(ErrorKind::Name, format!("unknown function `{name}`")));
    };
    map_scalars(ctx, v, &mut |x| match x {
        Value::Complex(z) => match cf {
            Some(f) => finite_complex(f(*z)),
            None => Err(err(
                ErrorKind::Type,
                format!("`{name}` is not defined for complex numbers"),
            )),
        },
        other => {
            let r = real_arg(other, name).map_err(|e| {
                if matches!(name, "sin" | "cos" | "tan") && e.kind == ErrorKind::Dimension {
                    ExprError::new(
                        ErrorKind::Dimension,
                        format!("{} (an angle is a plain number in radians or a quantity like `30 deg`)", e.message),
                    )
                } else {
                    e
                }
            })?;
            match rf(r) {
                Ok(y) => finite_real(y),
                Err(detail) => Err(err(
                    ErrorKind::Domain,
                    format!("domain error: {name} {detail}"),
                )),
            }
        }
    })
}

fn unary_q(ctx: &mut Ctx, name: &str, v: &Value) -> R<Value> {
    map_scalars(ctx, v, &mut |x| {
        match x {
        Value::Real(r) => match name {
            "sqrt" if *r < 0.0 => Err(err(
                ErrorKind::Domain,
                "domain error: sqrt of a negative real (use a complex argument, e.g. `sqrt(-4 + 0i)`)",
            )),
            "sqrt" => finite_real(r.sqrt()),
            "cbrt" => finite_real(r.cbrt()),
            "abs" => finite_real(r.abs()),
            "sign" => Ok(Value::Real(if *r == 0.0 { 0.0 } else { r.signum() })),
            "floor" => finite_real(r.floor()),
            "ceil" => finite_real(r.ceil()),
            "round" => finite_real(r.round()),
            _ => finite_real(r.trunc()),
        },
        Value::Complex(z) => match name {
            "abs" => finite_real(z.norm()),
            "sqrt" => finite_complex(z.sqrt()),
            "cbrt" => finite_complex(z.cbrt()),
            _ => Err(err(
                ErrorKind::Type,
                format!("`{name}` is not defined for complex numbers"),
            )),
        },
        Value::Quantity(q) => match name {
            "abs" => from_q(q.abs()),
            "sign" => Ok(Value::Real(if q.si_value() == 0.0 {
                0.0
            } else {
                q.si_value().signum()
            })),
            "sqrt" => from_q(q.sqrt()),
            "cbrt" => from_q(q.root(3)),
            _ => {
                let f: fn(f64) -> f64 = match name {
                    "floor" => f64::floor,
                    "ceil" => f64::ceil,
                    "round" => f64::round,
                    _ => f64::trunc,
                };
                match q.display_unit() {
                    Some(u) => {
                        let r = f(q.display_value());
                        if !r.is_finite() {
                            return Err(err(ErrorKind::Numeric, "the result is not finite"));
                        }
                        Ok(Value::quantity(Quantity::new(r, u)))
                    }
                    None => Ok(Value::quantity(Quantity::from_si(
                        f(q.si_value()),
                        *q.dimension(),
                    ))),
                }
            }
        },
        other => Err(err(
            ErrorKind::Type,
            format!("`{name}` needs a number, got {}", other.type_name()),
        )),
    }
    })
}

fn complex_parts(ctx: &mut Ctx, name: &str, v: &Value) -> R<Value> {
    map_scalars(ctx, v, &mut |x| {
        let z = match x {
            Value::Real(r) => Complex64::new(*r, 0.0),
            Value::Complex(z) => *z,
            other => {
                return Err(err(
                    ErrorKind::Type,
                    format!(
                        "`{name}` needs a real or complex number, got {}",
                        other.type_name()
                    ),
                ))
            }
        };
        match name {
            "re" => Ok(Value::Real(z.re)),
            "im" => Ok(Value::Real(z.im)),
            "arg" => Ok(Value::Real(z.arg())),
            _ => {
                if matches!(x, Value::Real(_)) {
                    Ok(x.clone())
                } else {
                    Ok(Value::Complex(z.conj()))
                }
            }
        }
    })
}

// ---------- special functions ----------

fn factorial(x: f64) -> R<f64> {
    if x.fract() != 0.0 || x < 0.0 {
        return Err(err(
            ErrorKind::Domain,
            format!("domain error: factorial needs a non-negative whole number, got {x}"),
        ));
    }
    if x > 170.0 {
        return Err(err(
            ErrorKind::Numeric,
            format!("factorial({x}) is too large to represent (the limit is 170!)"),
        ));
    }
    let mut f = 1.0f64;
    let mut k = 2.0;
    while k <= x {
        f *= k;
        k += 1.0;
    }
    Ok(f)
}

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

/// The gamma function: exact factorials at integers up to 171, Lanczos (g = 7, n = 9)
/// elsewhere with the reflection formula for `x < 0.5`; about 1e-15 relative error.
fn gamma(x: f64) -> R<f64> {
    if x.fract() == 0.0 {
        if x <= 0.0 {
            return Err(err(
                ErrorKind::Domain,
                format!("domain error: gamma is undefined at the non-positive integer {x}"),
            ));
        }
        if x <= 171.0 {
            return factorial(x - 1.0);
        }
        return Err(err(
            ErrorKind::Numeric,
            format!("gamma({x}) is too large to represent"),
        ));
    }
    Ok(gamma_lanczos(x))
}

fn gamma_lanczos(x: f64) -> f64 {
    if x < 0.5 {
        return PI / ((PI * x).sin() * gamma_lanczos(1.0 - x));
    }
    let x = x - 1.0;
    let mut a = LANCZOS[0];
    let t = x + 7.5;
    for (i, c) in LANCZOS.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    let half = t.powf((x + 0.5) / 2.0);
    (2.0 * PI).sqrt() * half * (-t).exp() * half * a
}

// ---------- aggregates ----------

/// Flattens the arguments into scalars (a container argument contributes its elements).
fn gather(ctx: &mut Ctx, args: &[Value], fname: &str) -> R<Vec<Value>> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Value::Vector(v) => out.extend(v.iter().cloned()),
            Value::Matrix(m) => out.extend(m.data.iter().cloned()),
            s if s.is_scalar() => out.push(s.clone()),
            other => {
                return Err(err(
                    ErrorKind::Type,
                    format!("`{fname}` needs numbers, got {}", other.type_name()),
                ))
            }
        }
    }
    ctx.charge(out.len() as u64)?;
    if out.is_empty() {
        return Err(err(ErrorKind::Domain, format!("`{fname}` of no values")));
    }
    Ok(out)
}

fn fold_items(items: &[Value], op: BinOp, fname: &str) -> R<Value> {
    let mut it = items.iter();
    let Some(first) = it.next() else {
        return Err(err(ErrorKind::Domain, format!("`{fname}` of no values")));
    };
    let mut acc = first.clone();
    for x in it {
        acc = scalar_op(op, &acc, x)?;
    }
    Ok(acc)
}

fn fold(ctx: &mut Ctx, args: &[Value], fname: &str, op: BinOp) -> R<Value> {
    let items = gather(ctx, args, fname)?;
    fold_items(&items, op, fname)
}

fn extreme(ctx: &mut Ctx, args: &[Value], fname: &str, want: Ordering) -> R<Value> {
    let items = gather(ctx, args, fname)?;
    let mut best = items
        .first()
        .cloned()
        .ok_or_else(|| err(ErrorKind::Domain, format!("`{fname}` of no values")))?;
    for x in items.iter().skip(1) {
        if ops::compare(x, &best)? == want {
            best = x.clone();
        }
    }
    if !matches!(best, Value::Real(_) | Value::Quantity(_)) {
        return Err(err(
            ErrorKind::Type,
            format!("`{fname}` is not defined for complex numbers"),
        ));
    }
    Ok(best)
}

fn median(ctx: &mut Ctx, args: &[Value]) -> R<Value> {
    let mut items = gather(ctx, args, "median")?;
    let mut failure: Option<ExprError> = None;
    items.sort_by(|a, b| match ops::compare(a, b) {
        Ok(o) => o,
        Err(e) => {
            failure.get_or_insert(e);
            Ordering::Equal
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    let n = items.len();
    let mid = n / 2;
    let (Some(hi), lo) = (items.get(mid), items.get(mid.wrapping_sub(1))) else {
        return Err(err(ErrorKind::Domain, "`median` of no values"));
    };
    if n % 2 == 1 {
        Ok(hi.clone())
    } else {
        let lo = lo.ok_or_else(|| err(ErrorKind::Domain, "`median` of no values"))?;
        let s = scalar_op(BinOp::Add, lo, hi)?;
        scalar_op(BinOp::Div, &s, &Value::Real(2.0))
    }
}

/// Sample variance (divides by n - 1). Returns the variance and the count.
fn variance(ctx: &mut Ctx, args: &[Value], fname: &str) -> R<(Value, usize)> {
    let items = gather(ctx, args, fname)?;
    let n = items.len();
    if n < 2 {
        return Err(err(
            ErrorKind::Domain,
            format!("`{fname}` needs at least 2 values (it divides by n - 1)"),
        ));
    }
    if items.iter().any(|x| matches!(x, Value::Complex(_))) {
        return Err(err(
            ErrorKind::Type,
            format!("`{fname}` is not defined for complex numbers"),
        ));
    }
    let sum = fold_items(&items, BinOp::Add, fname)?;
    let mean = scalar_op(BinOp::Div, &sum, &Value::Real(n as f64))?;
    let mut sq = Vec::with_capacity(n);
    for x in &items {
        let d = scalar_op(BinOp::Sub, x, &mean)?;
        sq.push(scalar_op(BinOp::Mul, &d, &d)?);
    }
    let total = fold_items(&sq, BinOp::Add, fname)?;
    Ok((
        scalar_op(BinOp::Div, &total, &Value::Real((n - 1) as f64))?,
        n,
    ))
}

fn norm(ctx: &mut Ctx, v: &Value) -> R<Value> {
    if v.is_scalar() {
        return unary_q(ctx, "abs", v);
    }
    let items = gather(ctx, std::slice::from_ref(v), "norm")?;
    let mut sq = Vec::with_capacity(items.len());
    for x in &items {
        sq.push(match x {
            Value::Complex(z) => Value::Real(z.norm_sqr()),
            other => scalar_op(BinOp::Mul, other, other)?,
        });
    }
    let total = fold_items(&sq, BinOp::Add, "norm")?;
    unary_q(ctx, "sqrt", &total)
}

fn dot(ctx: &mut Ctx, a: &Value, b: &Value) -> R<Value> {
    let (Value::Vector(x), Value::Vector(y)) = (a, b) else {
        return Err(err(ErrorKind::Type, "`dot` needs two vectors"));
    };
    if x.len() != y.len() {
        return Err(err(
            ErrorKind::Index,
            format!(
                "`dot` needs vectors of equal length, got {} and {}",
                x.len(),
                y.len()
            ),
        ));
    }
    ctx.charge(x.len() as u64)?;
    let mut acc: Option<Value> = None;
    for (p, q) in x.iter().zip(y) {
        let t = scalar_op(BinOp::Mul, p, q)?;
        acc = Some(match acc {
            None => t,
            Some(s) => scalar_op(BinOp::Add, &s, &t)?,
        });
    }
    acc.ok_or_else(|| err(ErrorKind::Domain, "`dot` of empty vectors"))
}

fn cross(a: &Value, b: &Value) -> R<Value> {
    let (Value::Vector(x), Value::Vector(y)) = (a, b) else {
        return Err(err(ErrorKind::Type, "`cross` needs two vectors"));
    };
    let (Some(x1), Some(x2), Some(x3), Some(y1), Some(y2), Some(y3), 3, 3) = (
        x.first(),
        x.get(1),
        x.get(2),
        y.first(),
        y.get(1),
        y.get(2),
        x.len(),
        y.len(),
    ) else {
        return Err(err(
            ErrorKind::Index,
            "`cross` needs two vectors of length 3",
        ));
    };
    let m = |p: &Value, q: &Value, r: &Value, s: &Value| -> R<Value> {
        scalar_op(
            BinOp::Sub,
            &scalar_op(BinOp::Mul, p, q)?,
            &scalar_op(BinOp::Mul, r, s)?,
        )
    };
    Ok(Value::Vector(vec![
        m(x2, y3, x3, y2)?,
        m(x3, y1, x1, y3)?,
        m(x1, y2, x2, y1)?,
    ]))
}

fn transpose(v: &Value) -> R<Value> {
    match v {
        Value::Vector(x) => Ok(Value::Matrix(Mat {
            rows: 1,
            cols: x.len(),
            data: x.clone(),
        })),
        Value::Matrix(m) => {
            let mut data = Vec::with_capacity(m.data.len());
            for c in 0..m.cols {
                for r in 0..m.rows {
                    if let Some(x) = m.get(r, c) {
                        data.push(x.clone());
                    }
                }
            }
            Ok(Value::Matrix(Mat {
                rows: m.cols,
                cols: m.rows,
                data,
            }))
        }
        s if s.is_scalar() => Ok(s.clone()),
        other => Err(err(
            ErrorKind::Type,
            format!("cannot transpose {}", other.type_name()),
        )),
    }
}

fn dims_arg(ctx: &mut Ctx, args: &[Value], fname: &str) -> R<(usize, usize)> {
    let max = ctx.limits.max_elements;
    let r = int_arg(arg0(args)?, fname, max)?;
    let c = match args.get(1) {
        Some(v) => int_arg(v, fname, max)?,
        None => 1,
    };
    let total = r
        .checked_mul(c)
        .ok_or_else(|| err(ErrorKind::Limit, "the size is too large"))?;
    ctx.check_elements(total)?;
    ctx.charge(total as u64)?;
    Ok((r, c))
}

fn filled(ctx: &mut Ctx, args: &[Value], fname: &str, x: f64) -> R<Value> {
    let (r, c) = dims_arg(ctx, args, fname)?;
    if r == 0 || c == 0 {
        return Err(err(
            ErrorKind::Domain,
            format!("`{fname}` needs positive sizes"),
        ));
    }
    if args.len() == 1 {
        return Ok(Value::Vector(vec![Value::Real(x); r]));
    }
    Ok(Value::Matrix(Mat {
        rows: r,
        cols: c,
        data: vec![Value::Real(x); r * c],
    }))
}

fn eye(ctx: &mut Ctx, args: &[Value]) -> R<Value> {
    let max = ctx.limits.max_elements;
    let r = int_arg(arg0(args)?, "eye", max)?;
    let c = match args.get(1) {
        Some(v) => int_arg(v, "eye", max)?,
        None => r,
    };
    let total = r
        .checked_mul(c)
        .ok_or_else(|| err(ErrorKind::Limit, "the size is too large"))?;
    ctx.check_elements(total)?;
    ctx.charge(total as u64)?;
    if r == 0 || c == 0 {
        return Err(err(ErrorKind::Domain, "`eye` needs positive sizes"));
    }
    let mut data = vec![Value::Real(0.0); total];
    for i in 0..r.min(c) {
        if let Some(x) = data.get_mut(i * c + i) {
            *x = Value::Real(1.0);
        }
    }
    Ok(Value::Matrix(Mat {
        rows: r,
        cols: c,
        data,
    }))
}

fn hypot(a: &Value, b: &Value) -> R<Value> {
    let (x, y) = same_dim_pair(a, b, "hypot")?;
    let h = x.hypot(y);
    match (a.as_quantity(), b) {
        (Some(qa), _) if !qa.dimension().is_dimensionless() || qa.display_unit().is_some() => {
            let v = Quantity::from_si(h, *qa.dimension());
            match qa.display_unit() {
                Some(u) => from_q(v.to(u)),
                None => Ok(Value::quantity(v)),
            }
        }
        _ => finite_real(h),
    }
}

fn modulo(a: &Value, b: &Value) -> R<Value> {
    let (x, y) = same_dim_pair(a, b, "mod")?;
    if y == 0.0 {
        return Err(err(ErrorKind::Domain, "division by zero in `mod`"));
    }
    let r = x - y * (x / y).floor();
    match a.as_quantity() {
        Some(qa) if !qa.dimension().is_dimensionless() || qa.display_unit().is_some() => {
            let v = Quantity::from_si(r, *qa.dimension());
            match qa.display_unit() {
                Some(u) => from_q(v.to(u)),
                None => Ok(Value::quantity(v)),
            }
        }
        _ => finite_real(r),
    }
}

/// A scalar `start + k * step` in the dimension and display unit of `like`.
fn scalar_like(like: &Value, si: f64) -> R<Value> {
    match like {
        Value::Quantity(q) => {
            let v = Quantity::from_si(si, *q.dimension());
            match q.display_unit() {
                Some(u) => from_q(v.to(u)),
                None => Ok(Value::quantity(v)),
            }
        }
        _ => finite_real(si),
    }
}

fn si_of(v: &Value, what: &str) -> R<f64> {
    match v {
        Value::Real(x) => Ok(*x),
        Value::Quantity(q) => Ok(q.si_value()),
        other => Err(err(
            ErrorKind::Type,
            format!(
                "the {what} of a range must be a number or quantity, got {}",
                other.type_name()
            ),
        )),
    }
}

/// `start:step:stop`, inclusive of `stop` when it is reached (to within rounding).
pub fn range(ctx: &mut Ctx, start: &Value, step: Option<&Value>, stop: &Value) -> R<Value> {
    let a = si_of(start, "start")?;
    let c = si_of(stop, "stop")?;
    let dim_a = start.as_quantity().map(|q| *q.dimension());
    let dim_c = stop.as_quantity().map(|q| *q.dimension());
    if dim_a != dim_c {
        // reuse the unit error text
        if let (Some(x), Some(y)) = (start.as_quantity(), stop.as_quantity()) {
            x.compare(&y).map_err(ExprError::from)?;
        }
    }
    let s = match step {
        Some(v) => {
            if v.as_quantity().map(|q| *q.dimension()) != dim_a {
                if let (Some(x), Some(y)) = (start.as_quantity(), v.as_quantity()) {
                    x.compare(&y).map_err(ExprError::from)?;
                }
            }
            si_of(v, "step")?
        }
        None => {
            if matches!(start, Value::Quantity(_)) {
                return Err(err(
                    ErrorKind::Dimension,
                    "a range of quantities needs an explicit step: start:step:stop",
                ));
            }
            1.0
        }
    };
    if s == 0.0 {
        return Err(err(ErrorKind::Domain, "the step of a range cannot be zero"));
    }
    let span = (c - a) / s;
    if !span.is_finite() || span < -1e-9 {
        return Err(err(
            ErrorKind::Domain,
            "the range is empty (the step points away from the stop value)",
        ));
    }
    let n_f = (span + 1e-9 * span.abs().max(1.0)).floor() + 1.0;
    if n_f > ctx.limits.max_elements as f64 {
        return Err(err(
            ErrorKind::Limit,
            format!(
                "the range would have {} elements; the limit is {}",
                n_f, ctx.limits.max_elements
            ),
        ));
    }
    let n = n_f as usize;
    ctx.charge(n as u64)?;
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        out.push(scalar_like(start, a + (k as f64) * s)?);
    }
    Ok(Value::Vector(out))
}

fn linspace(ctx: &mut Ctx, args: &[Value], log: bool) -> R<Value> {
    let a = arg0(args)?;
    let b = arg1(args)?;
    let n = int_arg(
        args.get(2)
            .ok_or_else(|| err(ErrorKind::Arity, "missing count"))?,
        "linspace",
        ctx.limits.max_elements,
    )?;
    if n == 0 {
        return Err(err(ErrorKind::Domain, "the count must be at least 1"));
    }
    ctx.charge(n as u64)?;
    let (x, y) = same_dim_pair(a, b, if log { "logspace" } else { "linspace" })?;
    if log && (matches!(a, Value::Quantity(_)) || matches!(b, Value::Quantity(_))) {
        return Err(err(
            ErrorKind::Dimension,
            "`logspace` takes plain numbers (the exponents)",
        ));
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let t = if n == 1 {
            1.0
        } else {
            k as f64 / (n - 1) as f64
        };
        let v = if k + 1 == n { y } else { x + (y - x) * t };
        out.push(if log {
            finite_real(10f64.powf(v))?
        } else {
            scalar_like(a, v)?
        });
    }
    Ok(Value::Vector(out))
}
