// SPDX-License-Identifier: MIT OR Apache-2.0
//! Text rendering of values, in the style of `format short` and `format long`.

use crate::value::{Arr, FnValue, Value};
use num_complex::Complex64;

/// A number as a short string with up to 5 significant digits, for messages.
pub fn short(x: f64) -> String {
    fmt_scalar(x, false)
}

fn is_int(x: f64) -> bool {
    x.is_finite() && x.fract() == 0.0 && x.abs() < 1e15
}

/// Formats one real number.
pub fn fmt_scalar(x: f64, long: bool) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    if is_int(x) && x.abs() < 1e10 {
        return format!("{}", x as i64);
    }
    let a = x.abs();
    if long {
        if (1e-5..1e15).contains(&a) {
            // 15 significant digits, no trailing zeros beyond the shortest round trip
            let s = format!("{x}");
            if s.len() <= 18 {
                return s;
            }
            return format!("{x:.15}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string();
        }
        return fmt_sci(x, 15);
    }
    if !(1e-3..1e5).contains(&a) {
        fmt_sci(x, 4)
    } else {
        format!("{x:.4}")
    }
}

fn fmt_sci(x: f64, digits: usize) -> String {
    let s = format!("{x:.digits$e}");
    // Rust prints `1.2346e5`; MATLAB prints `1.2346e+05`
    match s.split_once('e') {
        Some((m, e)) => {
            let (sign, num) = match e.strip_prefix('-') {
                Some(n) => ('-', n),
                None => ('+', e),
            };
            format!("{m}e{sign}{num:0>2}")
        }
        None => s,
    }
}

fn fmt_complex(z: Complex64, long: bool, parts: (bool, bool)) -> String {
    let re = if parts.0 {
        fmt_scalar(z.re, long)
    } else {
        fmt_fixed(z.re, long)
    };
    let im_abs = if parts.1 {
        fmt_scalar(z.im.abs(), long)
    } else {
        fmt_fixed(z.im.abs(), long)
    };
    let sign = if z.im < 0.0 || (z.im == 0.0 && z.im.is_sign_negative()) {
        '-'
    } else {
        '+'
    };
    format!("{re} {sign} {im_abs}i")
}

fn fmt_fixed(x: f64, long: bool) -> String {
    fmt_scalar(x, long)
}

/// Formats the elements of a real array consistently: all integers, or a common precision.
fn real_cells(a: &Arr<f64>, long: bool) -> Vec<String> {
    let finite: Vec<f64> = a.d.iter().copied().filter(|x| x.is_finite()).collect();
    let all_int = finite.iter().all(|&x| is_int(x));
    if all_int {
        return a.d.iter().map(|&x| fmt_scalar(x, long)).collect();
    }
    let max = finite.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    let min_nz = finite
        .iter()
        .filter(|x| **x != 0.0)
        .fold(f64::INFINITY, |m, x| m.min(x.abs()));
    let sci = !long && (max >= 1e5 || (min_nz.is_finite() && min_nz < 1e-3 && max / min_nz > 1e5))
        || (long && max >= 1e15);
    a.d.iter()
        .map(|&x| {
            if x.is_nan() {
                "NaN".to_string()
            } else if x.is_infinite() {
                if x > 0.0 {
                    "Inf".into()
                } else {
                    "-Inf".into()
                }
            } else if sci {
                fmt_sci(x, if long { 15 } else { 4 })
            } else if long {
                let s = format!("{x:.15}");
                s.trim_end_matches('0').to_string()
                    + if s.trim_end_matches('0').ends_with('.') {
                        "0"
                    } else {
                        ""
                    }
            } else {
                format!("{x:.4}")
            }
        })
        .collect()
}

fn grid(rows: usize, cols: usize, cells: &[String], gap: usize) -> String {
    let mut width = vec![0usize; cols];
    for j in 0..cols {
        for i in 0..rows {
            if let (Some(c), Some(w)) = (cells.get(j * rows + i), width.get_mut(j)) {
                *w = (*w).max(c.chars().count());
            }
        }
    }
    let mut out = String::new();
    for i in 0..rows {
        for j in 0..cols {
            let c = cells.get(j * rows + i).map_or("", String::as_str);
            let w = width.get(j).copied().unwrap_or(0);
            out.push_str(&" ".repeat(gap + w.saturating_sub(c.chars().count())));
            out.push_str(c);
        }
        out.push('\n');
    }
    out
}

/// Longest line count shown for large matrices; wider ones are split into column blocks.
const MAX_SHOWN_COLS: usize = 12;

fn matrix_body(v: &Value, long: bool) -> String {
    let (cells, rows, cols, gap): (Vec<String>, usize, usize, usize) = match v {
        Value::Num(a) => (real_cells(a, long), a.r, a.c, 3),
        Value::Bool(a) => (
            a.d.iter()
                .map(|&b| if b { "1" } else { "0" }.to_string())
                .collect(),
            a.r,
            a.c,
            3,
        ),
        Value::Cplx(a) => {
            let re = Arr {
                r: a.r,
                c: a.c,
                d: a.d.iter().map(|z| z.re).collect::<Vec<_>>(),
            };
            let im = Arr {
                r: a.r,
                c: a.c,
                d: a.d.iter().map(|z| z.im.abs()).collect::<Vec<_>>(),
            };
            let rc = real_cells(&re, long);
            let ic = real_cells(&im, long);
            let cells =
                a.d.iter()
                    .enumerate()
                    .map(|(k, z)| {
                        let sign = if z.im < 0.0 { '-' } else { '+' };
                        format!(
                            "{} {sign} {}i",
                            rc.get(k).cloned().unwrap_or_default(),
                            ic.get(k).cloned().unwrap_or_default()
                        )
                    })
                    .collect();
            (cells, a.r, a.c, 3)
        }
        _ => return String::new(),
    };
    if cols <= MAX_SHOWN_COLS {
        return grid(rows, cols, &cells, gap);
    }
    let mut out = String::new();
    let mut start = 0;
    while start < cols {
        let end = (start + MAX_SHOWN_COLS).min(cols);
        out.push_str(&format!(" Columns {} through {}\n\n", start + 1, end));
        let sub: Vec<String> = (start..end)
            .flat_map(|j| (0..rows).map(move |i| (i, j)))
            .filter_map(|(i, j)| cells.get(j * rows + i).cloned())
            .collect();
        out.push_str(&grid(rows, end - start, &sub, gap));
        out.push('\n');
        start = end;
    }
    out
}

/// A one-line summary of a value inside a cell or struct display.
pub fn summary(v: &Value, long: bool) -> String {
    match v {
        Value::Num(a) if a.is_scalar() => {
            fmt_scalar(a.d.first().copied().unwrap_or(0.0), long).to_string()
        }
        Value::Bool(a) if a.is_scalar() => if a.d.first().copied().unwrap_or(false) {
            "1"
        } else {
            "0"
        }
        .to_string(),
        Value::Cplx(a) if a.is_scalar() => fmt_complex(
            a.d.first().copied().unwrap_or(Complex64::new(0.0, 0.0)),
            long,
            (true, true),
        ),
        Value::Str(s) => format!("'{s}'"),
        Value::Num(a) if a.is_empty() => "[]".to_string(),
        Value::Cell(a) if a.is_empty() => "{}".to_string(),
        Value::Struct(s) => format!(
            "1x1 struct with {} field{}",
            s.fields.len(),
            if s.fields.len() == 1 { "" } else { "s" }
        ),
        Value::Fn(f) => fn_text(f),
        other => {
            let (r, c) = other.dims();
            let class = match other {
                Value::Cell(_) => "cell",
                _ => other.class(),
            };
            format!("{r}x{c} {class}")
        }
    }
}

/// The text of a function handle.
pub fn fn_text(f: &FnValue) -> String {
    match f {
        FnValue::Named(n) => format!("@{n}"),
        FnValue::Anon { src, .. } => src.clone(),
    }
}

/// What `disp(v)` prints.
pub fn disp(v: &Value, long: bool) -> String {
    match v {
        Value::Str(s) => format!("{s}\n"),
        Value::Num(a) if a.is_scalar() => format!(
            "{}\n",
            fmt_scalar(a.d.first().copied().unwrap_or(0.0), long)
        ),
        Value::Bool(a) if a.is_scalar() => {
            format!("{}\n", u8::from(a.d.first().copied().unwrap_or(false)))
        }
        Value::Cplx(a) if a.is_scalar() => format!(
            "{}\n",
            fmt_complex(
                a.d.first().copied().unwrap_or(Complex64::new(0.0, 0.0)),
                long,
                (true, true)
            )
        ),
        Value::Num(a) if a.is_empty() => String::new(),
        Value::Num(_) | Value::Bool(_) | Value::Cplx(_) => matrix_body(v, long),
        Value::Cell(a) => cell_body(a, long),
        Value::Struct(s) => {
            let mut out = String::new();
            for (n, x) in &s.fields {
                out.push_str(&format!("    {n}: {}\n", summary(x, long)));
            }
            out
        }
        Value::Fn(f) => format!("{}\n", fn_text(f)),
    }
}

fn cell_body(a: &Arr<Value>, long: bool) -> String {
    let cells: Vec<String> =
        a.d.iter()
            .map(|v| format!("{{{}}}", summary(v, long)))
            .collect();
    let mut out = String::new();
    out.push_str(
        &grid(a.r, a.c, &cells, 4)
            .lines()
            .map(|l| format!("  {l}\n"))
            .collect::<String>(),
    );
    out
}

/// What the interpreter prints for an unsuppressed result named `name`.
pub fn show(name: &str, v: &Value, long: bool) -> String {
    match v {
        Value::Str(s) => format!("{name} = {s}\n\n"),
        Value::Num(a) if a.is_scalar() => format!(
            "{name} = {}\n\n",
            fmt_scalar(a.d.first().copied().unwrap_or(0.0), long)
        ),
        Value::Bool(a) if a.is_scalar() => format!(
            "{name} = {}\n\n",
            u8::from(a.d.first().copied().unwrap_or(false))
        ),
        Value::Cplx(a) if a.is_scalar() => {
            format!(
                "{name} = {}\n\n",
                fmt_complex(
                    a.d.first().copied().unwrap_or(Complex64::new(0.0, 0.0)),
                    long,
                    (true, true)
                )
            )
        }
        Value::Num(a) if a.r == 0 && a.c == 0 => format!("{name} = []\n\n"),
        Value::Num(a) if a.is_empty() => {
            format!("{name} =\n\n  Empty matrix: {}-by-{}\n\n", a.r, a.c)
        }
        Value::Bool(a) if a.is_empty() => {
            format!("{name} =\n\n  Empty matrix: {}-by-{}\n\n", a.r, a.c)
        }
        Value::Num(_) | Value::Bool(_) | Value::Cplx(_) => {
            format!("{name} =\n\n{}\n", matrix_body(v, long))
        }
        Value::Cell(a) if a.is_empty() => format!("{name} = {{}}\n\n"),
        Value::Cell(a) => format!(
            "{name} =\n\n  {}x{} cell array\n\n{}\n",
            a.r,
            a.c,
            cell_body(a, long)
        ),
        Value::Struct(s) => {
            let mut out = format!("{name} =\n\n  struct with fields:\n\n");
            for (n, x) in &s.fields {
                out.push_str(&format!("    {n}: {}\n", summary(x, long)));
            }
            out.push('\n');
            out
        }
        Value::Fn(f) => format!(
            "{name} =\n\n  function_handle with value:\n\n    {}\n\n",
            fn_text(f)
        ),
    }
}
