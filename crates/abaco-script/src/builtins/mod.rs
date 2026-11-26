// SPDX-License-Identifier: MIT OR Apache-2.0
//! The built-in function library. Each submodule owns a table of `(name, function)` pairs;
//! the registry merges them. A built-in gets the interpreter, its arguments and how many
//! outputs the caller wants, and returns that many values (at least one when it has any).

use crate::error::{err, Result};
use crate::interp::Interp;
use crate::value::{Arr, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

mod control;
mod create;
mod elementwise;
mod files;
mod interpolate;
mod linear;
mod numeric;
mod odesuite;
mod optimize;
mod plotting;
mod poly;
mod reduce;
mod signal;
mod stats;
mod structs;
mod system;
mod text;

/// What a built-in returns.
pub type Out = Result<Vec<Value>>;
/// A built-in function.
pub type Bf = fn(&mut Interp, &[Value], usize) -> Out;

fn registry() -> &'static HashMap<&'static str, Bf> {
    static REG: OnceLock<HashMap<&'static str, Bf>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut m: HashMap<&'static str, Bf> = HashMap::new();
        for table in [
            control::TABLE,
            create::TABLE,
            elementwise::TABLE,
            files::TABLE,
            interpolate::TABLE,
            linear::TABLE,
            numeric::TABLE,
            odesuite::TABLE,
            optimize::TABLE,
            plotting::TABLE,
            poly::TABLE,
            reduce::TABLE,
            signal::TABLE,
            stats::TABLE,
            structs::TABLE,
            system::TABLE,
            text::TABLE,
        ] {
            for (name, f) in table {
                m.insert(name, *f);
            }
        }
        m
    })
}

/// The built-in called `name`.
pub fn lookup(name: &str) -> Option<Bf> {
    registry().get(name).copied()
}

/// Every built-in name, sorted.
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = registry().keys().copied().collect();
    v.sort_unstable();
    v
}

/// Calls the built-in `name`; `None` when there is no such built-in.
pub fn call(
    interp: &mut Interp,
    name: &str,
    args: &[Value],
    nargout: usize,
) -> Result<Option<Vec<Value>>> {
    match lookup(name) {
        Some(f) => f(interp, args, nargout).map(|mut out| {
            // A call never yields more values than were asked for (at least one).
            out.truncate(nargout.max(1));
            Some(out)
        }),
        None => Ok(None),
    }
}

// ---------- helpers shared by the submodules ----------

pub(crate) fn one(v: Value) -> Out {
    Ok(vec![v])
}

pub(crate) fn num(x: f64) -> Out {
    one(Value::num(x))
}

pub(crate) fn boolean(b: bool) -> Out {
    one(Value::boolean(b))
}

pub(crate) fn bool_arr(a: Arr<bool>) -> Out {
    one(Value::Bool(a))
}

pub(crate) fn real_out(a: Arr<f64>) -> Out {
    one(Value::Num(a))
}

/// Checks the argument count.
pub(crate) fn arity(args: &[Value], min: usize, max: usize, name: &str) -> Result<()> {
    if args.len() < min {
        return err(
            "Abaco:arity",
            format!(
                "`{name}` needs at least {min} argument{}, got {}",
                if min == 1 { "" } else { "s" },
                args.len()
            ),
        );
    }
    if args.len() > max {
        return err(
            "Abaco:arity",
            format!(
                "`{name}` takes at most {max} argument{}, got {}",
                if max == 1 { "" } else { "s" },
                args.len()
            ),
        );
    }
    Ok(())
}

/// Argument `i`, or an error naming the function.
pub(crate) fn arg<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a Value> {
    match args.get(i) {
        Some(v) => Ok(v),
        None => err(
            "Abaco:arity",
            format!("`{name}` is missing argument {}", i + 1),
        ),
    }
}

pub(crate) fn real_arg(args: &[Value], i: usize, name: &str) -> Result<Arr<f64>> {
    arg(args, i, name)?
        .to_real()
        .map_err(|e| crate::error::Error::new(&e.id, format!("`{name}`: {}", e.message)))
}

pub(crate) fn scalar_arg(args: &[Value], i: usize, name: &str) -> Result<f64> {
    arg(args, i, name)?.scalar_f64(&format!("argument {} of `{name}`", i + 1))
}

pub(crate) fn usize_arg(args: &[Value], i: usize, name: &str) -> Result<usize> {
    arg(args, i, name)?.scalar_usize(&format!("argument {} of `{name}`", i + 1))
}

pub(crate) fn str_arg<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str> {
    arg(args, i, name)?.as_str(&format!("argument {} of `{name}`", i + 1))
}

/// `true` when argument `i` is the text `s` (case-insensitive).
pub(crate) fn is_flag(args: &[Value], i: usize, s: &str) -> bool {
    matches!(args.get(i), Some(Value::Str(t)) if t.eq_ignore_ascii_case(s))
}

/// Dimensions from arguments such as `(n)`, `(r, c)` or `([r c])`.
pub(crate) fn dims_from(args: &[Value], name: &str) -> Result<(usize, usize)> {
    let limit = crate::ops::MAX_ELEMENTS;
    let (r, c) = match args {
        [] => (1, 1),
        [Value::Str(_)] => (1, 1),
        [v] => {
            let a = v.to_real()?;
            match a.d.as_slice() {
                [n] => (to_dim(*n, name)?, to_dim(*n, name)?),
                [r, c, rest @ ..] => {
                    if rest.iter().any(|&k| k != 1.0) {
                        return err(
                            "Abaco:size",
                            format!("`{name}`: only two-dimensional arrays are supported"),
                        );
                    }
                    (to_dim(*r, name)?, to_dim(*c, name)?)
                }
                _ => return err("Abaco:size", format!("`{name}`: expected a size")),
            }
        }
        [r, c, rest @ ..] => {
            for extra in rest {
                if matches!(extra, Value::Str(_)) {
                    break;
                }
                if extra.scalar_f64("a size")? != 1.0 {
                    return err(
                        "Abaco:size",
                        format!("`{name}`: only two-dimensional arrays are supported"),
                    );
                }
            }
            (
                to_dim(r.scalar_f64("a size")?, name)?,
                to_dim(c.scalar_f64("a size")?, name)?,
            )
        }
    };
    if r.saturating_mul(c) > limit {
        return err(
            "Abaco:limit",
            format!(
                "`{name}`: the result would have {} elements; the limit is {limit}",
                r.saturating_mul(c)
            ),
        );
    }
    Ok((r, c))
}

fn to_dim(x: f64, name: &str) -> Result<usize> {
    if x.is_nan() {
        return err("Abaco:size", format!("`{name}`: a size cannot be NaN"));
    }
    if x <= 0.0 {
        return Ok(0);
    }
    if !x.is_finite() || x > 1e9 {
        return err("Abaco:size", format!("`{name}`: the size {x} is too large"));
    }
    Ok(x.floor() as usize)
}

/// The leading size arguments (everything before the first text argument).
pub(crate) fn size_args(args: &[Value]) -> &[Value] {
    let n = args
        .iter()
        .position(|a| matches!(a, Value::Str(_)))
        .unwrap_or(args.len());
    &args[..n]
}

/// Picks the dimension to work along: the explicit one or the first non-singleton.
pub(crate) fn pick_dim(args: &[Value], i: usize, r: usize, name: &str) -> Result<usize> {
    match args.get(i) {
        Some(v) => {
            let d = v.scalar_usize(&format!("the dimension of `{name}`"))?;
            if d == 0 {
                return err(
                    "Abaco:size",
                    format!("`{name}`: the dimension must be 1 or 2"),
                );
            }
            Ok(d)
        }
        None => Ok(if r == 1 { 2 } else { 1 }),
    }
}

/// Reduces along `dim` (1 = down columns, 2 = along rows); a dimension above 2 reduces
/// nothing and applies `f` to single elements.
pub(crate) fn reduce_dim<T: Clone, U: Clone>(
    a: &Arr<T>,
    dim: usize,
    f: impl Fn(&[T]) -> U,
) -> Arr<U> {
    match dim {
        1 => {
            let d: Vec<U> = (0..a.c)
                .map(|j| f(a.d.get(j * a.r..(j + 1) * a.r).unwrap_or(&[])))
                .collect();
            Arr { r: 1, c: a.c, d }
        }
        2 => {
            let d: Vec<U> = (0..a.r).map(|i| f(&a.row_vec(i))).collect();
            Arr { r: a.r, c: 1, d }
        }
        _ => Arr {
            r: a.r,
            c: a.c,
            d: a.d.iter().map(|x| f(std::slice::from_ref(x))).collect(),
        },
    }
}

/// Applies a length-preserving scan along `dim`.
pub(crate) fn scan_dim<T: Clone>(a: &Arr<T>, dim: usize, f: impl Fn(&[T]) -> Vec<T>) -> Arr<T> {
    match dim {
        1 => {
            let mut d = Vec::with_capacity(a.d.len());
            for j in 0..a.c {
                d.extend(f(a.d.get(j * a.r..(j + 1) * a.r).unwrap_or(&[])));
            }
            Arr { r: a.r, c: a.c, d }
        }
        2 => {
            let mut out = a.clone();
            for i in 0..a.r {
                let row = f(&a.row_vec(i));
                for (j, x) in row.into_iter().enumerate() {
                    if let Some(slot) = out.d.get_mut(j * a.r + i) {
                        *slot = x;
                    }
                }
            }
            out
        }
        _ => a.clone(),
    }
}

/// The values of a vector-like argument as a plain `Vec`.
pub(crate) fn vec_arg(args: &[Value], i: usize, name: &str) -> Result<Vec<f64>> {
    Ok(real_arg(args, i, name)?.d)
}

/// A real vector shaped like `like` (row when it is a row).
pub(crate) fn shaped_like(v: Vec<f64>, like: &Arr<f64>) -> Value {
    if like.r == 1 {
        Value::row(v)
    } else {
        Value::col(v)
    }
}
