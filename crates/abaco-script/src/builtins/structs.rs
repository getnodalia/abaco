// SPDX-License-Identifier: MIT OR Apache-2.0
//! Type queries, conversions, cells, structs, higher-order functions and sets.

use super::create::gather;
use super::{arg, arity, bool_arr, boolean, one, real_arg, str_arg, Bf, Out};
use crate::display;
use crate::error::{err, Result};
use crate::index;
use crate::interp::Interp;
use crate::value::{Arr, FnValue, StructV, Value};
use std::cmp::Ordering;
use std::sync::Arc;

// ---------- class predicates ----------

macro_rules! is_class {
    ($fname:ident, $name:literal, $pat:pat) => {
        fn $fname(_: &mut Interp, a: &[Value], _: usize) -> Out {
            arity(a, 1, 1, $name)?;
            boolean(matches!(a[0], $pat))
        }
    };
}

is_class!(f_ischar, "ischar", Value::Str(_));
is_class!(f_iscell, "iscell", Value::Cell(_));
is_class!(f_isstruct, "isstruct", Value::Struct(_));
is_class!(f_islogical, "islogical", Value::Bool(_));
is_class!(f_isfunc, "is_function_handle", Value::Fn(_));
is_class!(f_isnumeric, "isnumeric", Value::Num(_) | Value::Cplx(_));
is_class!(f_isfloat, "isfloat", Value::Num(_) | Value::Cplx(_));

fn f_isreal(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "isreal")?;
    boolean(matches!(
        a[0],
        Value::Num(_) | Value::Bool(_) | Value::Str(_)
    ))
}

fn f_iscomplex(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "iscomplex")?;
    boolean(matches!(a[0], Value::Cplx(_)))
}

fn f_iscellstr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "iscellstr")?;
    boolean(matches!(&a[0], Value::Cell(c) if c.d.iter().all(|v| matches!(v, Value::Str(_)))))
}

fn f_class(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "class")?;
    one(Value::text(a[0].class()))
}

fn f_isa(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "isa")?;
    let want = str_arg(a, 1, "isa")?;
    let class = a[0].class();
    let numeric = matches!(a[0], Value::Num(_) | Value::Cplx(_));
    boolean(class == want || (want == "numeric" && numeric) || (want == "float" && numeric))
}

// ---------- conversions ----------

fn f_double(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "double")?;
    match &a[0] {
        Value::Cplx(_) => one(a[0].clone()),
        v => one(Value::Num(v.to_real()?)),
    }
}

fn f_logical(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "logical")?;
    let x = real_arg(a, 0, "logical")?;
    if x.d.iter().any(|v| v.is_nan()) {
        return err("Abaco:type", "NaN cannot be converted to logical");
    }
    bool_arr(x.map(|&v| v != 0.0))
}

fn sat(name: &'static str, lo: f64, hi: f64) -> impl Fn(&[Value]) -> Out {
    move |a: &[Value]| {
        arity(a, 1, 1, name)?;
        let x = real_arg(a, 0, name)?;
        super::real_out(x.map(|&v| {
            if v.is_nan() {
                0.0
            } else {
                v.round().clamp(lo, hi)
            }
        }))
    }
}

macro_rules! int_conv {
    ($fname:ident, $name:literal, $lo:expr, $hi:expr) => {
        fn $fname(_: &mut Interp, a: &[Value], _: usize) -> Out {
            sat($name, $lo, $hi)(a)
        }
    };
}

int_conv!(f_int8, "int8", -128.0, 127.0);
int_conv!(f_int16, "int16", -32_768.0, 32_767.0);
int_conv!(f_int32, "int32", -2_147_483_648.0, 2_147_483_647.0);
int_conv!(
    f_int64,
    "int64",
    -9_223_372_036_854_775_808.0,
    9_223_372_036_854_775_807.0
);
int_conv!(f_uint8, "uint8", 0.0, 255.0);
int_conv!(f_uint16, "uint16", 0.0, 65_535.0);
int_conv!(f_uint32, "uint32", 0.0, 4_294_967_295.0);
int_conv!(f_uint64, "uint64", 0.0, 1.844_674_407_370_955_2e19);

fn f_single(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "single")?;
    super::real_out(real_arg(a, 0, "single")?.map(|&v| f64::from(v as f32)))
}

// ---------- cells ----------

fn f_num2cell(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "num2cell")?;
    let v = &a[0];
    let (r, c) = v.dims();
    let mut out = Vec::with_capacity(r * c);
    for k in 0..r * c {
        out.push(gather(v, 1, 1, &[k])?);
    }
    one(Value::Cell(Arr { r, c, d: out }))
}

fn f_cell2mat(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "cell2mat")?;
    let Value::Cell(c) = &a[0] else {
        return err("Abaco:type", "`cell2mat` needs a cell array");
    };
    if c.d.is_empty() {
        return one(Value::empty());
    }
    let mut rows = Vec::new();
    for i in 0..c.r {
        rows.push(index::concat(&c.row_vec(i), true)?);
    }
    one(index::concat(&rows, false)?)
}

fn f_deal(_: &mut Interp, a: &[Value], nargout: usize) -> Out {
    let n = nargout.max(1);
    if a.len() == 1 {
        return Ok(vec![a[0].clone(); n]);
    }
    if a.len() != n {
        return err(
            "Abaco:arity",
            format!("`deal` needs one input or {n} inputs, got {}", a.len()),
        );
    }
    Ok(a.to_vec())
}

fn f_struct(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if !a.len().is_multiple_of(2) {
        return err(
            "Abaco:arity",
            "`struct` needs field names and values in pairs",
        );
    }
    let mut s = StructV::default();
    for pair in a.chunks(2) {
        let name = pair[0].as_str("a field name")?;
        let v = match &pair[1] {
            Value::Cell(c) if c.d.len() == 1 => c.d[0].clone(),
            Value::Cell(c) if c.d.is_empty() => Value::empty(),
            Value::Cell(_) => return err("Abaco:type", "struct arrays are not supported; wrap a cell in a second pair of braces to store it as a value"),
            other => other.clone(),
        };
        s.set(name, v);
    }
    one(Value::Struct(Box::new(s)))
}

fn f_fieldnames(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "fieldnames")?;
    let Value::Struct(s) = &a[0] else {
        return err(
            "Abaco:type",
            format!("`fieldnames` needs a struct, got {}", a[0].describe()),
        );
    };
    one(Value::Cell(Arr::col(
        s.fields
            .iter()
            .map(|(n, _)| Value::text(n.clone()))
            .collect(),
    )))
}

fn f_isfield(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "isfield")?;
    let Value::Struct(s) = &a[0] else {
        return boolean(false);
    };
    match &a[1] {
        Value::Str(n) => boolean(s.get(n).is_some()),
        Value::Cell(c) => bool_arr(c.map(|v| matches!(v, Value::Str(n) if s.get(n).is_some()))),
        _ => boolean(false),
    }
}

fn f_rmfield(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "rmfield")?;
    let Value::Struct(s) = &a[0] else {
        return err("Abaco:type", "`rmfield` needs a struct");
    };
    let mut s = s.clone();
    let n = str_arg(a, 1, "rmfield")?;
    if !s.remove(n) {
        return err("Abaco:field", format!("the struct has no field `{n}`"));
    }
    one(Value::Struct(s))
}

fn f_setfield(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 3, "setfield")?;
    let Value::Struct(s) = &a[0] else {
        return err("Abaco:type", "`setfield` needs a struct");
    };
    let mut s = s.clone();
    s.set(str_arg(a, 1, "setfield")?, a[2].clone());
    one(Value::Struct(s))
}

fn f_getfield(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "getfield")?;
    let Value::Struct(s) = &a[0] else {
        return err("Abaco:type", "`getfield` needs a struct");
    };
    let n = str_arg(a, 1, "getfield")?;
    match s.get(n) {
        Some(v) => one(v.clone()),
        None => err("Abaco:field", format!("the struct has no field `{n}`")),
    }
}

fn f_struct2cell(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "struct2cell")?;
    let Value::Struct(s) = &a[0] else {
        return err("Abaco:type", "`struct2cell` needs a struct");
    };
    one(Value::Cell(Arr::col(
        s.fields.iter().map(|(_, v)| v.clone()).collect(),
    )))
}

fn f_cell2struct(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "cell2struct")?;
    let (Value::Cell(c), Value::Cell(f)) = (&a[0], &a[1]) else {
        return err(
            "Abaco:type",
            "`cell2struct` needs a cell of values and a cell of field names",
        );
    };
    if c.d.len() != f.d.len() {
        return err(
            "Abaco:size",
            "`cell2struct` needs as many field names as values",
        );
    }
    let mut s = StructV::default();
    for (n, v) in f.d.iter().zip(&c.d) {
        s.set(n.as_str("a field name")?, v.clone());
    }
    one(Value::Struct(Box::new(s)))
}

// ---------- function handles and higher-order functions ----------

fn f_feval(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    let f = arg(a, 0, "feval")?.clone();
    i.call_value(&f, a[1..].to_vec(), nargout.max(1))
}

fn f_func2str(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "func2str")?;
    match &a[0] {
        Value::Fn(f) => one(Value::text(match &**f {
            FnValue::Named(n) => n.clone(),
            other => display::fn_text(other),
        })),
        other => err(
            "Abaco:type",
            format!(
                "`func2str` needs a function handle, got {}",
                other.describe()
            ),
        ),
    }
}

fn f_str2func(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "str2func")?;
    let s = str_arg(a, 0, "str2func")?.trim();
    if s.starts_with('@') {
        let v = i.eval_str(s)?;
        return one(v);
    }
    one(Value::Fn(Arc::new(FnValue::Named(s.to_string()))))
}

fn uniform_flag(a: &[Value]) -> (usize, bool) {
    // returns the number of leading data arguments and whether output is uniform
    let mut n = a.len();
    let mut uniform = true;
    while n >= 3 {
        if let Value::Str(k) = &a[n - 2] {
            if k.eq_ignore_ascii_case("UniformOutput") {
                uniform = a[n - 1].is_true().unwrap_or(true);
                n -= 2;
                continue;
            }
            if k.eq_ignore_ascii_case("ErrorHandler") {
                n -= 2;
                continue;
            }
        }
        break;
    }
    (n, uniform)
}

fn higher_order(i: &mut Interp, a: &[Value], nargout: usize, name: &str, over_cells: bool) -> Out {
    if a.len() < 2 {
        return err(
            "Abaco:arity",
            format!("`{name}` needs a function and at least one array"),
        );
    }
    let (n, uniform) = uniform_flag(a);
    let f = a[0].clone();
    let inputs = &a[1..n];
    let shape = inputs[0].dims();
    for v in inputs {
        if v.dims() != shape {
            return err(
                "Abaco:size",
                format!("`{name}` needs every array to have the same size"),
            );
        }
        if over_cells && !matches!(v, Value::Cell(_)) {
            return err(
                "Abaco:type",
                "`cellfun` needs cell arrays; use `arrayfun` for numeric arrays",
            );
        }
    }
    let count = shape.0 * shape.1;
    let nout = nargout.max(1);
    let mut results: Vec<Vec<Value>> = vec![Vec::with_capacity(count); nout];
    // legacy string names such as cellfun('isempty', c)
    let f = match &f {
        Value::Str(s) if over_cells => match s.as_str() {
            "isempty" | "length" | "numel" | "ndims" | "isreal" | "isclass" => {
                Value::text(s.clone())
            }
            _ => f.clone(),
        },
        _ => f,
    };
    for k in 0..count {
        i.charge(1)?;
        let mut args = Vec::with_capacity(inputs.len());
        for v in inputs {
            args.push(if over_cells {
                match v {
                    Value::Cell(c) => c.d.get(k).cloned().unwrap_or_default(),
                    _ => Value::empty(),
                }
            } else {
                gather(v, 1, 1, &[k])?
            });
        }
        let out = i.call_value(&f, args, nout)?;
        if out.len() < nout {
            return err(
                "Abaco:value",
                format!(
                    "the function given to `{name}` returned {} value(s) but {nout} were requested",
                    out.len()
                ),
            );
        }
        for (slot, v) in results.iter_mut().zip(out) {
            slot.push(v);
        }
    }
    let mut outs = Vec::with_capacity(nout);
    for vals in results {
        if !uniform {
            outs.push(Value::Cell(Arr {
                r: shape.0,
                c: shape.1,
                d: vals,
            }));
            continue;
        }
        if vals
            .iter()
            .all(|v| matches!(v, Value::Bool(b) if b.d.len() == 1))
            && !vals.is_empty()
        {
            let d = vals
                .iter()
                .map(|v| matches!(v, Value::Bool(b) if b.d.first() == Some(&true)))
                .collect();
            outs.push(Value::Bool(Arr {
                r: shape.0,
                c: shape.1,
                d,
            }));
        } else if vals.iter().any(|v| matches!(v, Value::Cplx(_))) {
            let mut d = Vec::with_capacity(vals.len());
            for v in &vals {
                let z = v.to_complex()?;
                if z.d.len() != 1 {
                    return err("Abaco:value", format!("`{name}` with UniformOutput needs scalar results; use 'UniformOutput', false"));
                }
                d.push(z.d[0]);
            }
            outs.push(Value::cplx(Arr {
                r: shape.0,
                c: shape.1,
                d,
            }));
        } else {
            let mut d = Vec::with_capacity(vals.len());
            for v in &vals {
                match v {
                    Value::Num(x) if x.d.len() == 1 => d.push(x.d[0]),
                    Value::Bool(x) if x.d.len() == 1 => d.push(f64::from(u8::from(x.d[0]))),
                    Value::Str(s) if s.chars().count() == 1 => d.push(s.chars().next().map_or(0.0, |c| f64::from(u32::from(c)))),
                    _ => return err("Abaco:value", format!("`{name}` with UniformOutput needs scalar results; use 'UniformOutput', false")),
                }
            }
            outs.push(Value::Num(Arr {
                r: shape.0,
                c: shape.1,
                d,
            }));
        }
    }
    Ok(outs)
}

fn f_cellfun(i: &mut Interp, a: &[Value], n: usize) -> Out {
    // a few classic string forms avoid a user call
    if let (Some(Value::Str(s)), Some(Value::Cell(c))) = (a.first(), a.get(1)) {
        match s.as_str() {
            "isempty" => return bool_arr(c.map(Value::is_empty)),
            "length" => {
                return super::real_out(c.map(|v| {
                    let (r, k) = v.dims();
                    if r == 0 || k == 0 {
                        0.0
                    } else {
                        r.max(k) as f64
                    }
                }))
            }
            "numel" | "prodofsize" => return super::real_out(c.map(|v| v.numel() as f64)),
            _ => {}
        }
    }
    higher_order(i, a, n, "cellfun", true)
}

fn f_arrayfun(i: &mut Interp, a: &[Value], n: usize) -> Out {
    higher_order(i, a, n, "arrayfun", false)
}

fn f_structfun(i: &mut Interp, a: &[Value], n: usize) -> Out {
    arity(a, 2, 4, "structfun")?;
    let Value::Struct(s) = &a[1] else {
        return err(
            "Abaco:type",
            "`structfun` needs a struct as its second argument",
        );
    };
    let cell = Value::Cell(Arr::col(s.fields.iter().map(|(_, v)| v.clone()).collect()));
    let mut args = vec![a[0].clone(), cell];
    args.extend(a[2..].iter().cloned());
    higher_order(i, &args, n, "structfun", true)
}

// ---------- equality and sets ----------

/// Deep equality that ignores the numeric class, like `isequal`.
pub(crate) fn values_equal(a: &Value, b: &Value) -> bool {
    equal_with(a, b, false)
}

/// Equality of values; with `nan_equal`, NaN equals NaN (`isequaln`).
fn equal_with(a: &Value, b: &Value, nan_equal: bool) -> bool {
    let same = |x: f64, y: f64| x == y || (nan_equal && x.is_nan() && y.is_nan());
    match (a, b) {
        (Value::Cell(x), Value::Cell(y)) => {
            x.r == y.r
                && x.c == y.c
                && x.d
                    .iter()
                    .zip(&y.d)
                    .all(|(p, q)| equal_with(p, q, nan_equal))
        }
        (Value::Struct(x), Value::Struct(y)) => {
            x.fields.len() == y.fields.len()
                && x.fields
                    .iter()
                    .all(|(n, v)| y.get(n).is_some_and(|w| equal_with(v, w, nan_equal)))
        }
        (Value::Fn(x), Value::Fn(y)) => x == y,
        (Value::Cplx(_), _) | (_, Value::Cplx(_)) => match (a.to_complex(), b.to_complex()) {
            (Ok(x), Ok(y)) => {
                x.r == y.r
                    && x.c == y.c
                    && x.d
                        .iter()
                        .zip(&y.d)
                        .all(|(p, q)| same(p.re, q.re) && same(p.im, q.im))
            }
            _ => false,
        },
        _ if a.is_numeric_like() && b.is_numeric_like() => match (a.to_real(), b.to_real()) {
            (Ok(x), Ok(y)) => {
                x.r == y.r && x.c == y.c && x.d.iter().zip(&y.d).all(|(p, q)| same(*p, *q))
            }
            _ => false,
        },
        _ => false,
    }
}

fn f_isequal(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if a.len() < 2 {
        return err("Abaco:arity", "`isequal` needs at least two arguments");
    }
    boolean(a[1..].iter().all(|v| values_equal(&a[0], v)))
}

fn f_isequaln(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if a.len() < 2 {
        return err("Abaco:arity", "`isequaln` needs at least two arguments");
    }
    boolean(a[1..].iter().all(|v| equal_with(&a[0], v, true)))
}

fn cellstr(v: &Value, what: &str) -> Result<Vec<String>> {
    match v {
        Value::Str(s) => Ok(vec![s.clone()]),
        Value::Cell(c) => {
            c.d.iter()
                .map(|x| match x {
                    Value::Str(s) => Ok(s.clone()),
                    _ => err("Abaco:type", format!("{what} needs text in every cell")),
                })
                .collect()
        }
        _ => err(
            "Abaco:type",
            format!("{what} needs text or a cell array of text"),
        ),
    }
}

fn is_text_like(v: &Value) -> bool {
    matches!(v, Value::Cell(_)) || matches!(v, Value::Str(s) if !s.is_empty())
}

fn f_ismember(_: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 2, 2, "ismember")?;
    if matches!(a[1], Value::Cell(_)) {
        let set = cellstr(&a[1], "`ismember`")?;
        let probes = cellstr(&a[0], "`ismember`")?;
        let idx: Vec<f64> = probes
            .iter()
            .map(|p| {
                set.iter()
                    .position(|s| s == p)
                    .map_or(0.0, |k| k as f64 + 1.0)
            })
            .collect();
        let tf: Vec<bool> = idx.iter().map(|&k| k > 0.0).collect();
        let shape = match &a[0] {
            Value::Cell(c) => (c.r, c.c),
            _ => (1, 1),
        };
        return Ok(vec![
            Value::Bool(Arr::new(shape.0, shape.1, tf)),
            Value::Num(Arr::new(shape.0, shape.1, idx)),
        ]);
    }
    let (x, s) = (real_arg(a, 0, "ismember")?, real_arg(a, 1, "ismember")?);
    let idx = x.map(|v| {
        s.d.iter()
            .position(|t| t == v)
            .map_or(0.0, |k| k as f64 + 1.0)
    });
    let tf = idx.map(|&k| k > 0.0);
    if nargout > 1 {
        Ok(vec![Value::Bool(tf), Value::Num(idx)])
    } else {
        bool_arr(tf)
    }
}

fn sorted_unique(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    v.dedup();
    v
}

fn set_op(a: &[Value], name: &str, keep: fn(bool, bool) -> bool) -> Out {
    arity(a, 2, 2, name)?;
    if is_text_like(&a[0]) || is_text_like(&a[1]) {
        let (x, y) = (cellstr(&a[0], name)?, cellstr(&a[1], name)?);
        let mut all: Vec<String> = x.iter().chain(&y).cloned().collect();
        all.sort();
        all.dedup();
        let out: Vec<Value> = all
            .into_iter()
            .filter(|s| keep(x.contains(s), y.contains(s)))
            .map(Value::Str)
            .collect();
        return one(Value::Cell(Arr::row(out)));
    }
    let (x, y) = (real_arg(a, 0, name)?, real_arg(a, 1, name)?);
    let all = sorted_unique(x.d.iter().chain(&y.d).copied().collect());
    let out: Vec<f64> = all
        .into_iter()
        .filter(|v| keep(x.d.contains(v), y.d.contains(v)))
        .collect();
    let col = x.c == 1 && x.r > 1 && (y.c == 1 || y.d.is_empty());
    one(if col {
        Value::col(out)
    } else {
        Value::row(out)
    })
}

fn f_union(_: &mut Interp, a: &[Value], _: usize) -> Out {
    set_op(a, "union", |p, q| p || q)
}

fn f_intersect(_: &mut Interp, a: &[Value], _: usize) -> Out {
    set_op(a, "intersect", |p, q| p && q)
}

fn f_setdiff(_: &mut Interp, a: &[Value], _: usize) -> Out {
    set_op(a, "setdiff", |p, q| p && !q)
}

fn f_setxor(_: &mut Interp, a: &[Value], _: usize) -> Out {
    set_op(a, "setxor", |p, q| p != q)
}

pub const TABLE: &[(&str, Bf)] = &[
    ("ischar", f_ischar),
    ("iscell", f_iscell),
    ("isstruct", f_isstruct),
    ("islogical", f_islogical),
    ("isbool", f_islogical),
    ("is_function_handle", f_isfunc),
    ("isnumeric", f_isnumeric),
    ("isfloat", f_isfloat),
    ("isreal", f_isreal),
    ("iscomplex", f_iscomplex),
    ("iscellstr", f_iscellstr),
    ("class", f_class),
    ("isa", f_isa),
    ("double", f_double),
    ("logical", f_logical),
    ("single", f_single),
    ("int8", f_int8),
    ("int16", f_int16),
    ("int32", f_int32),
    ("int64", f_int64),
    ("uint8", f_uint8),
    ("uint16", f_uint16),
    ("uint32", f_uint32),
    ("uint64", f_uint64),
    ("num2cell", f_num2cell),
    ("cell2mat", f_cell2mat),
    ("deal", f_deal),
    ("struct", f_struct),
    ("fieldnames", f_fieldnames),
    ("isfield", f_isfield),
    ("rmfield", f_rmfield),
    ("setfield", f_setfield),
    ("getfield", f_getfield),
    ("struct2cell", f_struct2cell),
    ("cell2struct", f_cell2struct),
    ("feval", f_feval),
    ("func2str", f_func2str),
    ("str2func", f_str2func),
    ("cellfun", f_cellfun),
    ("arrayfun", f_arrayfun),
    ("structfun", f_structfun),
    ("isequal", f_isequal),
    ("isequaln", f_isequaln),
    ("ismember", f_ismember),
    ("union", f_union),
    ("intersect", f_intersect),
    ("setdiff", f_setdiff),
    ("setxor", f_setxor),
];
