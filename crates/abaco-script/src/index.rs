// SPDX-License-Identifier: MIT OR Apache-2.0
//! Indexing, indexed assignment, deletion and concatenation.

use crate::error::{err, Result};
use crate::ops::MAX_ELEMENTS;
use crate::value::{Arr, Value};
use num_complex::Complex64;

/// One resolved index argument: 0-based positions and the shape of the index itself.
#[derive(Clone, Debug)]
pub struct Sel {
    /// Positions, 0-based.
    pub idx: Vec<usize>,
    /// Rows and columns of the index as written.
    pub shape: (usize, usize),
    /// `true` for `:`.
    pub colon: bool,
}

impl Sel {
    /// A single position.
    pub fn one(k: usize) -> Sel {
        Sel {
            idx: vec![k],
            shape: (1, 1),
            colon: false,
        }
    }

    /// Resolves an index value against a dimension of length `n`.
    pub fn from_value(v: &Value, n: usize) -> Result<Sel> {
        match v {
            Value::Bool(m) => {
                let idx: Vec<usize> =
                    m.d.iter()
                        .enumerate()
                        .filter(|(_, &b)| b)
                        .map(|(i, _)| i)
                        .collect();
                let k = idx.len();
                let shape = if m.r == 1 { (1, k) } else { (k, 1) };
                Ok(Sel {
                    idx,
                    shape,
                    colon: false,
                })
            }
            Value::Num(a) => {
                let mut idx = Vec::with_capacity(a.d.len());
                for &x in &a.d {
                    if !x.is_finite() || x.fract() != 0.0 {
                        return err(
                            "Abaco:index",
                            format!(
                                "indices must be positive whole numbers or logicals, got {}",
                                crate::display::short(x)
                            ),
                        );
                    }
                    if x < 1.0 {
                        return err(
                            "Abaco:index",
                            if x == 0.0 {
                                "index 0 is out of range: indices start at 1".to_string()
                            } else {
                                format!("index {x} is out of range: indices start at 1")
                            },
                        );
                    }
                    if x > 1e9 {
                        return err("Abaco:index", format!("index {x} is too large"));
                    }
                    idx.push(x as usize - 1);
                }
                Ok(Sel {
                    idx,
                    shape: (a.r, a.c),
                    colon: false,
                })
            }
            Value::Str(s) if s == ":" => Ok(Sel::colon(n)),
            Value::Str(_) => Sel::from_value(&Value::Num(v.to_real()?), n),
            other => err(
                "Abaco:index",
                format!("cannot index with {}", other.describe()),
            ),
        }
    }

    /// The whole dimension.
    pub fn colon(n: usize) -> Sel {
        Sel {
            idx: (0..n).collect(),
            shape: (n, 1),
            colon: true,
        }
    }

    fn max(&self) -> Option<usize> {
        self.idx.iter().copied().max()
    }
}

fn oob(pos: usize, len: usize) -> crate::error::Error {
    crate::error::Error::new(
        "Abaco:index",
        format!(
            "index {} is out of range: the array has {len} element{}",
            pos + 1,
            if len == 1 { "" } else { "s" }
        ),
    )
}

/// `a(sel)` with one index.
pub fn get1<T: Clone>(a: &Arr<T>, s: &Sel) -> Result<Arr<T>> {
    let n = a.d.len();
    let mut d = Vec::with_capacity(s.idx.len());
    for &k in &s.idx {
        match a.d.get(k) {
            Some(x) => d.push(x.clone()),
            None => return Err(oob(k, n)),
        }
    }
    let k = d.len();
    let (r, c) = if s.colon {
        (k, 1)
    } else if a.r == 1 && a.c != 1 && (s.shape.0 == 1 || s.shape.1 == 1) {
        (1, k)
    } else if a.c == 1 && a.r != 1 && (s.shape.0 == 1 || s.shape.1 == 1) {
        (k, 1)
    } else if s.shape.0 * s.shape.1 == k {
        s.shape
    } else {
        (k, 1)
    };
    Ok(Arr { r, c, d })
}

/// `a(rows, cols)`.
pub fn get2<T: Clone>(a: &Arr<T>, rs: &Sel, cs: &Sel) -> Result<Arr<T>> {
    let mut d = Vec::with_capacity(rs.idx.len() * cs.idx.len());
    for &j in &cs.idx {
        if j >= a.c {
            return err(
                "Abaco:index",
                format!(
                    "column index {} is out of range: the array has {} column{}",
                    j + 1,
                    a.c,
                    if a.c == 1 { "" } else { "s" }
                ),
            );
        }
        for &i in &rs.idx {
            if i >= a.r {
                return err(
                    "Abaco:index",
                    format!(
                        "row index {} is out of range: the array has {} row{}",
                        i + 1,
                        a.r,
                        if a.r == 1 { "" } else { "s" }
                    ),
                );
            }
            if let Some(x) = a.d.get(j * a.r + i) {
                d.push(x.clone());
            }
        }
    }
    Ok(Arr {
        r: rs.idx.len(),
        c: cs.idx.len(),
        d,
    })
}

/// Grows `a` to at least `r`×`c`, filling with `fill`.
pub fn resize<T: Clone>(a: &mut Arr<T>, r: usize, c: usize, fill: &T) -> Result<()> {
    let (nr, nc) = (a.r.max(r), a.c.max(c));
    if (nr, nc) == (a.r, a.c) {
        return Ok(());
    }
    if nr.saturating_mul(nc) > MAX_ELEMENTS {
        return err(
            "Abaco:limit",
            format!(
                "the array would have {} elements; the limit is {MAX_ELEMENTS}",
                nr.saturating_mul(nc)
            ),
        );
    }
    if nr == a.r {
        a.d.resize(nr * nc, fill.clone());
        a.c = nc;
        return Ok(());
    }
    let mut d = vec![fill.clone(); nr * nc];
    for j in 0..a.c {
        for i in 0..a.r {
            if let (Some(src), Some(dst)) = (a.d.get(j * a.r + i), d.get_mut(j * nr + i)) {
                *dst = src.clone();
            }
        }
    }
    a.r = nr;
    a.c = nc;
    a.d = d;
    Ok(())
}

/// `a(sel) = rhs`.
pub fn set1<T: Clone>(a: &mut Arr<T>, s: &Sel, rhs: &Arr<T>, fill: &T) -> Result<()> {
    let k = s.idx.len();
    if rhs.d.len() != 1 && rhs.d.len() != k {
        return err(
            "Abaco:size",
            format!(
                "the assignment has {k} position{} on the left but {} value{} on the right",
                if k == 1 { "" } else { "s" },
                rhs.d.len(),
                if rhs.d.len() == 1 { "" } else { "s" }
            ),
        );
    }
    if let Some(m) = s.max() {
        if m >= a.d.len() {
            // grow a vector; a matrix cannot grow by linear index
            if a.d.is_empty() {
                let (r, c) = if rhs.c == 1 && rhs.r > 1 && !s.colon {
                    (m + 1, 1)
                } else {
                    (1, m + 1)
                };
                *a = Arr {
                    r,
                    c,
                    d: Vec::new(),
                };
                resize(a, r, c, fill)?;
                a.d.resize(r * c, fill.clone());
            } else if a.r == 1 {
                resize(a, 1, m + 1, fill)?;
            } else if a.c == 1 {
                resize(a, m + 1, 1, fill)?;
            } else {
                return err(
                    "Abaco:index",
                    format!("index {} is out of range for a {}x{} matrix; a matrix cannot grow with a single index", m + 1, a.r, a.c),
                );
            }
        }
    }
    for (n, &pos) in s.idx.iter().enumerate() {
        let v = if rhs.d.len() == 1 {
            rhs.d.first()
        } else {
            rhs.d.get(n)
        };
        if let (Some(v), Some(slot)) = (v, a.d.get_mut(pos)) {
            *slot = v.clone();
        }
    }
    Ok(())
}

/// `a(rows, cols) = rhs`.
pub fn set2<T: Clone>(a: &mut Arr<T>, rs: &Sel, cs: &Sel, rhs: &Arr<T>, fill: &T) -> Result<()> {
    let (nr, nc) = (rs.idx.len(), cs.idx.len());
    if rhs.d.len() != 1
        && !(rhs.r == nr && rhs.c == nc)
        && !(rhs.d.len() == nr * nc && (rhs.r == 1 || rhs.c == 1) && (nr == 1 || nc == 1))
    {
        return err(
            "Abaco:size",
            format!(
                "the assignment region is {nr}x{nc} but the right side is {}x{}",
                rhs.r, rhs.c
            ),
        );
    }
    // `:` on an empty or shorter array adopts the shape of the right side
    let rows_needed = if rs.colon && a.r == 0 {
        rhs.r.max(1)
    } else {
        rs.max().map_or(0, |m| m + 1)
    };
    let cols_needed = if cs.colon && a.c == 0 {
        rhs.c.max(1)
    } else {
        cs.max().map_or(0, |m| m + 1)
    };
    let rs = if rs.colon && a.r == 0 {
        Sel::colon(rows_needed)
    } else {
        rs.clone()
    };
    let cs = if cs.colon && a.c == 0 {
        Sel::colon(cols_needed)
    } else {
        cs.clone()
    };
    resize(a, rows_needed, cols_needed, fill)?;
    let mut n = 0usize;
    for &j in &cs.idx {
        for &i in &rs.idx {
            let v = if rhs.d.len() == 1 {
                rhs.d.first()
            } else {
                rhs.d.get(n)
            };
            if let (Some(v), Some(slot)) = (v, a.d.get_mut(j * a.r + i)) {
                *slot = v.clone();
            }
            n += 1;
        }
    }
    Ok(())
}

/// `a(sel) = []`.
pub fn delete1<T: Clone>(a: &mut Arr<T>, s: &Sel) -> Result<()> {
    let n = a.d.len();
    if let Some(m) = s.max() {
        if m >= n {
            return Err(oob(m, n));
        }
    }
    let mut kill = vec![false; n];
    for &k in &s.idx {
        if let Some(x) = kill.get_mut(k) {
            *x = true;
        }
    }
    let keep: Vec<T> =
        a.d.iter()
            .zip(&kill)
            .filter(|(_, &k)| !k)
            .map(|(x, _)| x.clone())
            .collect();
    if s.colon || (a.r != 1 && a.c != 1 && a.r * a.c != 0) {
        // deleting from a matrix by linear index yields a row
        a.r = if keep.is_empty() && s.colon { 0 } else { 1 };
        a.c = keep.len();
    } else if a.c == 1 && a.r != 1 {
        a.r = keep.len();
    } else {
        a.r = 1;
        a.c = keep.len();
    }
    a.d = keep;
    Ok(())
}

/// `a(rows, cols) = []`: one of the two must be `:` (or cover the whole dimension).
pub fn delete2<T: Clone>(a: &mut Arr<T>, rs: &Sel, cs: &Sel) -> Result<()> {
    let all_rows = rs.colon || (0..a.r).all(|i| rs.idx.contains(&i));
    let all_cols = cs.colon || (0..a.c).all(|j| cs.idx.contains(&j));
    if all_rows {
        if let Some(m) = cs.max() {
            if m >= a.c {
                return err(
                    "Abaco:index",
                    format!("column index {} is out of range", m + 1),
                );
            }
        }
        let mut d = Vec::new();
        let mut nc = 0;
        for j in 0..a.c {
            if cs.idx.contains(&j) {
                continue;
            }
            nc += 1;
            d.extend(a.column(j));
        }
        a.c = nc;
        a.d = d;
        Ok(())
    } else if all_cols {
        if let Some(m) = rs.max() {
            if m >= a.r {
                return err(
                    "Abaco:index",
                    format!("row index {} is out of range", m + 1),
                );
            }
        }
        let keep: Vec<usize> = (0..a.r).filter(|i| !rs.idx.contains(i)).collect();
        let mut d = Vec::with_capacity(keep.len() * a.c);
        for j in 0..a.c {
            for &i in &keep {
                if let Some(x) = a.at(i, j) {
                    d.push(x.clone());
                }
            }
        }
        a.r = keep.len();
        a.d = d;
        Ok(())
    } else {
        err(
            "Abaco:index",
            "a null assignment can only delete whole rows or whole columns",
        )
    }
}

// ---------- value level ----------

fn chars_of(s: &str) -> Arr<f64> {
    if s.is_empty() {
        Arr::empty()
    } else {
        Arr::row(s.chars().map(|c| f64::from(u32::from(c))).collect())
    }
}

fn string_from(a: &Arr<f64>) -> String {
    a.d.iter()
        .filter_map(|&x| char::from_u32(x as u32))
        .collect()
}

/// `v(args)`.
pub fn index_value(v: &Value, args: &[Sel]) -> Result<Value> {
    macro_rules! go {
        ($a:expr, $wrap:expr) => {
            match args {
                [s] => Ok($wrap(get1($a, s)?)),
                [r, c] => Ok($wrap(get2($a, r, c)?)),
                [] => Ok(v.clone()),
                [r, c, rest @ ..] => {
                    if rest.iter().all(|s| s.idx.iter().all(|&k| k == 0)) {
                        Ok($wrap(get2($a, r, c)?))
                    } else {
                        err(
                            "Abaco:index",
                            "arrays have two dimensions; extra indices must be 1",
                        )
                    }
                }
            }
        };
    }
    match v {
        Value::Num(a) => go!(a, Value::Num),
        Value::Cplx(a) => go!(a, Value::cplx),
        Value::Bool(a) => go!(a, Value::Bool),
        Value::Cell(a) => go!(a, Value::Cell),
        Value::Str(s) => {
            let a = chars_of(s);
            let r = match args {
                [sel] => get1(&a, sel)?,
                [r, c] => get2(&a, r, c)?,
                [] => a,
                _ => return err("Abaco:index", "a string takes one or two indices"),
            };
            Ok(Value::Str(string_from(&r)))
        }
        Value::Struct(_) => {
            if args.iter().all(|s| s.idx.iter().all(|&k| k == 0)) {
                Ok(v.clone())
            } else {
                err("Abaco:index", "index exceeds the 1x1 size of a struct")
            }
        }
        Value::Fn(_) => err("Abaco:index", "a function handle is called, not indexed"),
    }
}

/// Which kind `a(...) = rhs` produces.
fn assign_kind(target: &Value, rhs: &Value) -> Result<&'static str> {
    Ok(match (target, rhs) {
        (Value::Cell(_), Value::Cell(_)) => "cell",
        (Value::Cell(_), _) => {
            return err("Abaco:type", "to store into a cell array with `()` the right side must be a cell; use `c{i} = value` for a value")
        }
        (_, Value::Cell(_)) if target.is_empty() => "cell",
        (_, Value::Cell(_)) => return err("Abaco:type", "cannot store a cell into a numeric array"),
        (t, _) if t.is_empty() && !matches!(rhs, Value::Struct(_) | Value::Fn(_)) => match rhs {
            Value::Cplx(_) => "cplx",
            Value::Bool(_) => "bool",
            Value::Str(_) => "str",
            _ => "num",
        },
        (Value::Struct(_), Value::Struct(_)) => "struct",
        (Value::Struct(_), _) | (_, Value::Struct(_)) => return err("Abaco:type", "cannot mix a struct with other values in an indexed assignment"),
        (Value::Fn(_), _) | (_, Value::Fn(_)) => return err("Abaco:type", "function handles cannot be stored with `()` indexing; put them in a cell"),
        (Value::Cplx(_), _) | (_, Value::Cplx(_)) => "cplx",
        (Value::Bool(_), Value::Bool(_)) => "bool",
        (Value::Bool(_), _) => "bool",
        (Value::Str(_), Value::Str(_)) => "str",
        (Value::Str(_), _) => "str",
        _ => "num",
    })
}

/// `target(args) = rhs`.
pub fn assign_value(target: &mut Value, args: &[Sel], rhs: &Value) -> Result<()> {
    if let (Value::Struct(_), Value::Struct(_)) = (&*target, rhs) {
        if args.iter().all(|s| s.idx.iter().all(|&k| k == 0)) {
            *target = rhs.clone();
            return Ok(());
        }
    }
    let kind = assign_kind(target, rhs)?;
    macro_rules! run {
        ($arr:ident, $conv:expr, $fill:expr, $back:expr) => {{
            let mut a: Arr<_> = $arr;
            let r = $conv;
            match args {
                [s] => set1(&mut a, s, &r, &$fill)?,
                [rr, c] => set2(&mut a, rr, c, &r, &$fill)?,
                _ => return err("Abaco:index", "an assignment takes one or two indices"),
            }
            *target = $back(a);
            Ok(())
        }};
    }
    match kind {
        "cell" => {
            let a = match &*target {
                Value::Cell(a) => a.clone(),
                _ => Arr::empty(),
            };
            let r = match rhs {
                Value::Cell(r) => r.clone(),
                _ => Arr::scalar(rhs.clone()),
            };
            run!(a, r, Value::empty(), Value::Cell)
        }
        "cplx" => {
            let a = if target.is_empty() {
                Arr::empty()
            } else {
                target.to_complex()?
            };
            let r = rhs.to_complex()?;
            run!(a, r, Complex64::new(0.0, 0.0), Value::cplx)
        }
        "bool" => {
            let a = match &*target {
                Value::Bool(a) => a.clone(),
                _ => Arr::empty(),
            };
            let r = crate::ops::truth(rhs)?;
            run!(a, r, false, Value::Bool)
        }
        "str" => {
            let a = match &*target {
                Value::Str(s) => chars_of(s),
                _ => Arr::empty(),
            };
            let r = rhs.to_real()?;
            let mut a = a;
            match args {
                [s] => set1(&mut a, s, &r, &32.0)?,
                [rr, c] => set2(&mut a, rr, c, &r, &32.0)?,
                _ => return err("Abaco:index", "an assignment takes one or two indices"),
            }
            *target = Value::Str(string_from(&a));
            Ok(())
        }
        _ => {
            let a = if target.is_empty() {
                Arr::empty()
            } else {
                target.to_real()?
            };
            let r = rhs.to_real()?;
            run!(a, r, 0.0, Value::Num)
        }
    }
}

/// `target(args) = []`.
pub fn delete_value(target: &mut Value, args: &[Sel]) -> Result<()> {
    macro_rules! del {
        ($a:expr) => {{
            match args {
                [s] => delete1($a, s),
                [r, c] => delete2($a, r, c),
                _ => err("Abaco:index", "a deletion takes one or two indices"),
            }
        }};
    }
    match target {
        Value::Num(a) => del!(a),
        Value::Cplx(a) => del!(a),
        Value::Bool(a) => del!(a),
        Value::Cell(a) => del!(a),
        Value::Str(s) => {
            let mut a = chars_of(s);
            del!(&mut a)?;
            *s = string_from(&a);
            Ok(())
        }
        other => err(
            "Abaco:type",
            format!("cannot delete elements of {}", other.describe()),
        ),
    }
}

// ---------- concatenation ----------

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Bool,
    Num,
    Cplx,
    Str,
    Cell,
}

fn kind_of(vals: &[Value]) -> Result<Kind> {
    let mut k = Kind::Bool;
    let mut any_str = false;
    let mut any_cell = false;
    let mut all_bool = true;
    let mut any = false;
    for v in vals {
        match v {
            Value::Cell(_) => any_cell = true,
            Value::Str(_) => {
                any_str = true;
                all_bool = false;
            }
            Value::Num(_) => {
                all_bool = false;
                if k == Kind::Bool {
                    k = Kind::Num;
                }
            }
            Value::Cplx(_) => {
                all_bool = false;
                k = Kind::Cplx;
            }
            Value::Bool(_) => {}
            Value::Struct(_) | Value::Fn(_) => {
                return err("Abaco:type", "structs and function handles cannot be joined with `[ ]`; use a cell array `{ }`");
            }
        }
        any = true;
    }
    if !any {
        return Ok(Kind::Num);
    }
    if any_cell {
        return Ok(Kind::Cell);
    }
    if any_str {
        return Ok(Kind::Str);
    }
    if all_bool {
        return Ok(Kind::Bool);
    }
    Ok(k)
}

fn cat_generic<T: Clone>(parts: &[Arr<T>], horizontal: bool) -> Result<Arr<T>> {
    let parts: Vec<&Arr<T>> = parts.iter().filter(|p| !(p.r == 0 && p.c == 0)).collect();
    let Some(first) = parts.first() else {
        return Ok(Arr::empty());
    };
    if horizontal {
        let r = first.r;
        let mut c = 0;
        let mut d = Vec::new();
        for p in &parts {
            if p.r != r {
                return err(
                    "Abaco:size",
                    format!(
                        "horizontal concatenation needs the same number of rows, got {r} and {}",
                        p.r
                    ),
                );
            }
            c += p.c;
            d.extend(p.d.iter().cloned());
        }
        if r.saturating_mul(c) > MAX_ELEMENTS {
            return err("Abaco:limit", "the concatenated array is too large");
        }
        Ok(Arr { r, c, d })
    } else {
        let c = first.c;
        let mut r = 0;
        for p in &parts {
            if p.c != c {
                return err(
                    "Abaco:size",
                    format!(
                        "vertical concatenation needs the same number of columns, got {c} and {}",
                        p.c
                    ),
                );
            }
            r += p.r;
        }
        if r.saturating_mul(c) > MAX_ELEMENTS {
            return err("Abaco:limit", "the concatenated array is too large");
        }
        let mut d = Vec::with_capacity(r * c);
        for j in 0..c {
            for p in &parts {
                d.extend(p.column(j));
            }
        }
        Ok(Arr {
            r,
            c: if r == 0 { 0 } else { c },
            d,
        })
    }
}

/// Joins values side by side (`horizontal`) or one above the other.
pub fn concat(vals: &[Value], horizontal: bool) -> Result<Value> {
    if vals.len() == 1 {
        if let Some(v) = vals.first() {
            return Ok(v.clone());
        }
    }
    match kind_of(vals)? {
        Kind::Cell => {
            let parts: Vec<Arr<Value>> = vals
                .iter()
                .map(|v| match v {
                    Value::Cell(a) => a.clone(),
                    other => Arr::scalar(other.clone()),
                })
                .collect();
            Ok(Value::Cell(cat_generic(&parts, horizontal)?))
        }
        Kind::Str => {
            if !horizontal && vals.iter().filter(|v| !v.is_empty()).count() > 1 {
                return err("Abaco:type", "stacking text vertically needs a char matrix, which Ábaco does not have; use a cell array of strings");
            }
            let parts: Vec<Arr<f64>> = vals
                .iter()
                .map(|v| match v {
                    Value::Str(s) => Ok(chars_of(s)),
                    other => other.to_real(),
                })
                .collect::<Result<_>>()?;
            let a = cat_generic(&parts, true)?;
            Ok(Value::Str(string_from(&a)))
        }
        Kind::Cplx => {
            let parts: Vec<Arr<Complex64>> =
                vals.iter().map(Value::to_complex).collect::<Result<_>>()?;
            Ok(Value::cplx(cat_generic(&parts, horizontal)?))
        }
        Kind::Num => {
            let parts: Vec<Arr<f64>> = vals.iter().map(Value::to_real).collect::<Result<_>>()?;
            Ok(Value::Num(cat_generic(&parts, horizontal)?))
        }
        Kind::Bool => {
            let parts: Vec<Arr<bool>> =
                vals.iter().map(crate::ops::truth).collect::<Result<_>>()?;
            Ok(Value::Bool(cat_generic(&parts, horizontal)?))
        }
    }
}
