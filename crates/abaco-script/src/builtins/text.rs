// SPDX-License-Identifier: MIT OR Apache-2.0
//! Text: formatted output, conversion between numbers and text, searching and regular
//! expressions.

use super::{arity, bool_arr, boolean, num, one, real_arg, real_out, str_arg, Bf, Out};
use crate::display;
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::value::{Arr, Value};
use regex::RegexBuilder;

// ---------- the printf engine ----------

enum A {
    Num(f64),
    Text(String),
}

fn flatten_args(args: &[Value]) -> Result<Vec<A>> {
    let mut out = Vec::new();
    for v in args {
        match v {
            Value::Str(s) => out.push(A::Text(s.clone())),
            Value::Num(a) => out.extend(a.d.iter().map(|&x| A::Num(x))),
            Value::Bool(a) => out.extend(a.d.iter().map(|&b| A::Num(f64::from(u8::from(b))))),
            Value::Cplx(a) => {
                for z in &a.d {
                    out.push(A::Num(z.re));
                    out.push(A::Num(z.im));
                }
            }
            Value::Cell(_) => {
                return err(
                    "Abaco:type",
                    "a cell array cannot be printed; print its contents with c{k}",
                )
            }
            other => return err("Abaco:type", format!("cannot print {}", other.describe())),
        }
    }
    Ok(out)
}

/// Interprets the backslash escapes of a format string.
pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\u{7}'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn exp_form(x: f64, prec: usize, upper: bool) -> String {
    let s = format!("{x:.prec$e}");
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    let sign = if e < 0 { '-' } else { '+' };
    let t = format!("{mant}e{sign}{:02}", e.abs());
    if upper {
        t.to_uppercase()
    } else {
        t
    }
}

fn strip_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let t = s.trim_end_matches('0');
    t.trim_end_matches('.').to_string()
}

fn general_form(x: f64, prec: Option<usize>, alt: bool, upper: bool) -> String {
    let p = prec.unwrap_or(6).max(1);
    if x == 0.0 {
        return if alt {
            format!("{:.*}", p - 1, 0.0)
        } else {
            "0".into()
        };
    }
    let probe = format!("{:.*e}", p - 1, x);
    let e: i32 = probe
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    if e < -4 || e >= p as i32 {
        let t = exp_form(x, p - 1, upper);
        if alt {
            return t;
        }
        let (m, ex) = t
            .split_once(if upper { 'E' } else { 'e' })
            .unwrap_or((&t, ""));
        format!("{}{}{}", strip_zeros(m), if upper { 'E' } else { 'e' }, ex)
    } else {
        let decimals = (p as i32 - 1 - e).max(0) as usize;
        let t = format!("{x:.decimals$}");
        if alt {
            t
        } else {
            strip_zeros(&t)
        }
    }
}

struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    zero: bool,
    alt: bool,
    width: Option<usize>,
    prec: Option<usize>,
    conv: char,
}

fn pad(body: String, sp: &Spec, numeric: bool) -> String {
    let len = body.chars().count();
    let Some(w) = sp.width else {
        return body;
    };
    if len >= w {
        return body;
    }
    let fill = w - len;
    if sp.minus {
        format!("{body}{}", " ".repeat(fill))
    } else if sp.zero && numeric && !body.contains("Inf") && !body.contains("NaN") {
        let (sign, digits) = match body.chars().next() {
            Some(c @ ('-' | '+' | ' ')) => (c.to_string(), body[1..].to_string()),
            _ => (String::new(), body),
        };
        format!("{sign}{}{digits}", "0".repeat(fill))
    } else {
        format!("{}{body}", " ".repeat(fill))
    }
}

fn signed(body: String, x: f64, sp: &Spec) -> String {
    if x.is_sign_negative() && !x.is_nan() || body.starts_with('-') {
        body
    } else if sp.plus {
        format!("+{body}")
    } else if sp.space {
        format!(" {body}")
    } else {
        body
    }
}

fn format_number(x: f64, sp: &Spec) -> String {
    if x.is_nan() || x.is_infinite() {
        let t = if x.is_nan() {
            "NaN".to_string()
        } else if x < 0.0 {
            "-Inf".into()
        } else {
            "Inf".into()
        };
        return pad(signed(t, x, sp), &Spec { zero: false, ..*sp }, true);
    }
    let body = match sp.conv {
        'd' | 'i' | 'u' => {
            if x.fract() == 0.0 && x.abs() < 1e18 {
                format!("{}", x as i64)
            } else if let Some(p) = sp.prec {
                format!("{x:.p$}")
            } else {
                // a non-integer under %d prints in the shortest form that round-trips
                let t = format!("{x}");
                if t.len() > 12 {
                    general_form(x, Some(17), false, false)
                } else {
                    t
                }
            }
        }
        'f' | 'F' => format!("{:.*}", sp.prec.unwrap_or(6), x),
        'e' => exp_form(x, sp.prec.unwrap_or(6), false),
        'E' => exp_form(x, sp.prec.unwrap_or(6), true),
        'g' => general_form(x, sp.prec, sp.alt, false),
        'G' => general_form(x, sp.prec, sp.alt, true),
        'x' => format!("{}{:x}", if sp.alt { "0x" } else { "" }, x as i64),
        'X' => format!("{}{:X}", if sp.alt { "0X" } else { "" }, x as i64),
        'o' => format!("{}{:o}", if sp.alt { "0" } else { "" }, x as i64),
        _ => format!("{x}"),
    };
    pad(signed(body, x, sp), sp, true)
}

fn parse_spec(chars: &[char], mut i: usize) -> Option<(Spec, usize, bool)> {
    // chars[i] is the first character after '%'
    let mut sp = Spec {
        minus: false,
        plus: false,
        space: false,
        zero: false,
        alt: false,
        width: None,
        prec: None,
        conv: 'd',
    };
    while let Some(&c) = chars.get(i) {
        match c {
            '-' => sp.minus = true,
            '+' => sp.plus = true,
            ' ' => sp.space = true,
            '0' => sp.zero = true,
            '#' => sp.alt = true,
            _ => break,
        }
        i += 1;
    }
    let mut star_width = false;
    if chars.get(i) == Some(&'*') {
        star_width = true;
        i += 1;
    } else {
        let mut w = String::new();
        while let Some(&c) = chars.get(i) {
            if c.is_ascii_digit() {
                w.push(c);
                i += 1;
            } else {
                break;
            }
        }
        sp.width = w.parse().ok();
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        let mut p = String::new();
        while let Some(&c) = chars.get(i) {
            if c.is_ascii_digit() {
                p.push(c);
                i += 1;
            } else {
                break;
            }
        }
        sp.prec = Some(p.parse().unwrap_or(0));
    }
    // length modifiers are accepted and ignored
    while matches!(chars.get(i), Some('l' | 'h' | 'L')) {
        i += 1;
    }
    sp.conv = *chars.get(i)?;
    Some((sp, i + 1, star_width))
}

/// MATLAB's `sprintf`.
pub(crate) fn sprintf(fmt: &str, args: &[Value]) -> Result<String> {
    let fmt = unescape(fmt);
    let chars: Vec<char> = fmt.chars().collect();
    let mut queue: std::collections::VecDeque<A> = flatten_args(args)?.into();
    let no_args = queue.is_empty();
    let mut out = String::new();
    let has_conv = chars.windows(2).any(|w| w[0] == '%' && w[1] != '%');
    loop {
        let mut i = 0;
        let mut consumed_any = false;
        while i < chars.len() {
            let c = chars[i];
            if c != '%' {
                out.push(c);
                i += 1;
                continue;
            }
            if chars.get(i + 1) == Some(&'%') {
                out.push('%');
                i += 2;
                continue;
            }
            let Some((mut sp, next, star)) = parse_spec(&chars, i + 1) else {
                out.push('%');
                i += 1;
                continue;
            };
            i = next;
            if star {
                match queue.pop_front() {
                    Some(A::Num(w)) => {
                        sp.width = Some(w.abs() as usize);
                        if w < 0.0 {
                            sp.minus = true;
                        }
                    }
                    _ => return Ok(out),
                }
            }
            if no_args {
                // nothing to print: a conversion without data prints nothing
                continue;
            }
            let Some(a) = queue.pop_front() else {
                return Ok(out);
            };
            consumed_any = true;
            match (sp.conv, a) {
                ('s', A::Text(t)) => {
                    let t = match sp.prec {
                        Some(p) => t.chars().take(p).collect(),
                        None => t,
                    };
                    out.push_str(&pad(t, &sp, false));
                }
                ('c', A::Text(t)) => {
                    // %c consumes one character at a time
                    let mut cs = t.chars();
                    if let Some(first) = cs.next() {
                        out.push_str(&pad(first.to_string(), &sp, false));
                        let rest: String = cs.collect();
                        if !rest.is_empty() {
                            queue.push_front(A::Text(rest));
                        }
                    }
                }
                ('s' | 'c', A::Num(x)) => {
                    let t = if x.fract() == 0.0 && (0.0..1_114_112.0).contains(&x) {
                        char::from_u32(x as u32).map_or_else(|| format!("{x}"), |ch| ch.to_string())
                    } else {
                        general_form(
                            x,
                            sp.prec.or(Some(5)).filter(|_| false).or(Some(6)),
                            false,
                            false,
                        )
                    };
                    out.push_str(&pad(t, &sp, false));
                }
                (_, A::Text(t)) => {
                    // text under a numeric conversion prints its character codes
                    let mut codes: Vec<A> = t
                        .chars()
                        .map(|ch| A::Num(f64::from(u32::from(ch))))
                        .collect();
                    if let Some(A::Num(first)) = codes.first() {
                        out.push_str(&format_number(*first, &sp));
                    }
                    for extra in codes.drain(1..).rev() {
                        queue.push_front(extra);
                    }
                }
                (_, A::Num(x)) => out.push_str(&format_number(x, &sp)),
            }
        }
        if queue.is_empty() || !has_conv || !consumed_any {
            break;
        }
    }
    Ok(out)
}

fn f_sprintf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let fmt = str_arg(a, 0, "sprintf")?;
    one(Value::text(sprintf(fmt, &a[1..])?))
}

fn f_fprintf(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let (fid, rest) = match a.first() {
        Some(Value::Num(n)) if n.is_scalar() && a.len() >= 2 => (n.d[0], &a[1..]),
        _ => (1.0, a),
    };
    if fid != 1.0 && fid != 2.0 {
        return err(
            "Abaco:io",
            "`fprintf` writes to the screen only (fid 1 or 2); use writematrix or save for files",
        );
    }
    let fmt = str_arg(rest, 0, "fprintf")?;
    let s = sprintf(fmt, &rest[1..])?;
    i.print(&s);
    Ok(vec![])
}

fn f_printf(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let fmt = str_arg(a, 0, "printf")?;
    let s = sprintf(fmt, &a[1..])?;
    i.print(&s);
    Ok(vec![])
}

fn f_disp(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "disp")?;
    let s = display::disp(&a[0], i.format_long);
    i.print(&s);
    Ok(vec![])
}

fn f_display(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "display")?;
    let s = display::disp(&a[0], i.format_long);
    i.print(&s);
    Ok(vec![])
}

// ---------- numbers to text ----------

fn fmt_g(x: f64, digits: usize) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x < 0.0 {
            "-Inf".into()
        } else {
            "Inf".into()
        }
    } else {
        general_form(x, Some(digits), false, false)
    }
}

fn num2str_real(a: &Arr<f64>, prec: Option<usize>) -> String {
    let finite: Vec<f64> = a.d.iter().copied().filter(|x| x.is_finite()).collect();
    let maxabs = finite.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    let all_int = a.d.iter().all(|x| !x.is_finite() || x.fract() == 0.0);
    let mag = if maxabs == 0.0 {
        0
    } else {
        maxabs.log10().floor() as i32
    };
    let any_neg = a.d.iter().any(|&x| x < 0.0);
    let cells: Vec<String> =
        a.d.iter()
            .map(|&x| match prec {
                Some(p) => fmt_g(x, p),
                None if all_int => {
                    if x.is_finite() {
                        format!("{}", x as i64)
                    } else {
                        fmt_g(x, 6)
                    }
                }
                None => fmt_g(x, (mag + 5).clamp(5, 16) as usize),
            })
            .collect();
    if a.d.len() == 1 {
        return cells.into_iter().next().unwrap_or_default();
    }
    let width = if all_int && prec.is_none() {
        (mag.max(0) as usize + 1) + usize::from(any_neg) + 2
    } else {
        (mag + 5).clamp(5, 16) as usize + 7 + usize::from(any_neg)
    };
    let mut rows: Vec<String> = Vec::new();
    for i in 0..a.r {
        let mut line = String::new();
        for j in 0..a.c {
            let cell = cells.get(j * a.r + i).cloned().unwrap_or_default();
            line.push_str(&format!("{cell:>width$}"));
        }
        rows.push(line);
    }
    // remove the common leading blanks, as MATLAB does
    let lead = rows
        .iter()
        .map(|r| r.len() - r.trim_start().len())
        .min()
        .unwrap_or(0);
    rows.iter()
        .map(|r| r[lead..].trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn f_num2str(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "num2str")?;
    if let Value::Str(s) = &a[0] {
        return one(Value::text(s.clone()));
    }
    if let (Some(Value::Str(f)), true) = (a.get(1), a.len() == 2) {
        return one(Value::text(sprintf(f, &a[..1])?));
    }
    let prec = match a.get(1) {
        Some(v) => Some(v.scalar_usize("the precision")?.max(1)),
        None => None,
    };
    if let Value::Cplx(z) = &a[0] {
        let parts: Vec<String> =
            z.d.iter()
                .map(|c| {
                    let (re, im) = (
                        fmt_g(c.re, prec.unwrap_or(5).max(5)),
                        fmt_g(c.im.abs(), prec.unwrap_or(5).max(5)),
                    );
                    format!("{re}{}{im}i", if c.im < 0.0 { "-" } else { "+" })
                })
                .collect();
        return one(Value::text(parts.join("  ")));
    }
    let x = real_arg(a, 0, "num2str")?;
    if x.d.is_empty() {
        return one(Value::text(""));
    }
    one(Value::text(num2str_real(&x, prec)))
}

fn f_int2str(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "int2str")?;
    let x = real_arg(a, 0, "int2str")?.map(|v| v.round());
    one(Value::text(num2str_real(&x, None)))
}

fn mat2str_num(x: f64, prec: usize) -> String {
    fmt_g(x, prec)
}

fn f_mat2str(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "mat2str")?;
    let prec = if a.len() > 1 {
        a[1].scalar_usize("the precision")?.max(1)
    } else {
        15
    };
    let body = match &a[0] {
        Value::Str(s) => return one(Value::text(format!("\"{s}\""))),
        Value::Bool(b) => grid(b.r, b.c, |i, j| {
            if b.at(i, j) == Some(&true) {
                "true".into()
            } else {
                "false".into()
            }
        }),
        Value::Cplx(z) => grid(z.r, z.c, |i, j| {
            let c = z.at(i, j).copied().unwrap_or_default();
            format!(
                "{}{}{}i",
                mat2str_num(c.re, prec),
                if c.im < 0.0 { "-" } else { "+" },
                mat2str_num(c.im.abs(), prec)
            )
        }),
        v => {
            let m = v.to_real()?;
            grid(m.r, m.c, |i, j| {
                mat2str_num(m.at(i, j).copied().unwrap_or(0.0), prec)
            })
        }
    };
    let (r, c) = a[0].dims();
    one(Value::text(if r * c == 1 {
        body.trim_matches(|ch| ch == '[' || ch == ']').to_string()
    } else {
        body
    }))
}

fn grid(r: usize, c: usize, f: impl Fn(usize, usize) -> String) -> String {
    if r * c == 0 {
        return "zeros(0,0)".into();
    }
    let rows: Vec<String> = (0..r)
        .map(|i| (0..c).map(|j| f(i, j)).collect::<Vec<_>>().join(" "))
        .collect();
    format!("[{}]", rows.join(";"))
}

// ---------- text to numbers ----------

fn parse_number(s: &str) -> f64 {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "inf" | "+inf" => return f64::INFINITY,
        "-inf" => return f64::NEG_INFINITY,
        "nan" => return f64::NAN,
        _ => {}
    }
    if t.is_empty() || t.contains(|c: char| c.is_alphabetic() && c != 'e' && c != 'E') {
        return f64::NAN;
    }
    t.replace(',', "").parse::<f64>().unwrap_or(f64::NAN)
}

fn f_str2double(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "str2double")?;
    match &a[0] {
        Value::Str(s) => num(parse_number(s)),
        Value::Cell(c) => real_out(c.map(|v| {
            if let Value::Str(s) = v {
                parse_number(s)
            } else {
                f64::NAN
            }
        })),
        _ => num(f64::NAN),
    }
}

fn f_str2num(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "str2num")?;
    let s = str_arg(a, 0, "str2num")?;
    match i.eval_str(&format!("[{s}]")) {
        Ok(v) => Ok(vec![v, Value::boolean(true)]),
        Err(_) => Ok(vec![Value::empty(), Value::boolean(false)]),
    }
}

// ---------- string manipulation ----------

/// Applies `f` to a string or to every string of a cell.
fn per_text(v: &Value, name: &str, f: impl Fn(&str) -> String) -> Result<Value> {
    per_text_dyn(v, name, &f)
}

fn per_text_dyn(v: &Value, name: &str, f: &dyn Fn(&str) -> String) -> Result<Value> {
    match v {
        Value::Str(s) => Ok(Value::Str(f(s))),
        Value::Cell(c) => {
            let mut d = Vec::with_capacity(c.d.len());
            for x in &c.d {
                d.push(per_text_dyn(x, name, f)?);
            }
            Ok(Value::Cell(Arr { r: c.r, c: c.c, d }))
        }
        other => err(
            "Abaco:type",
            format!("`{name}` needs text, got {}", other.describe()),
        ),
    }
}

fn f_upper(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "upper")?;
    one(per_text(&a[0], "upper", str::to_uppercase)?)
}

fn f_lower(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "lower")?;
    one(per_text(&a[0], "lower", str::to_lowercase)?)
}

fn f_strtrim(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "strtrim")?;
    one(per_text(&a[0], "strtrim", |s| s.trim().to_string())?)
}

fn f_deblank(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "deblank")?;
    one(per_text(&a[0], "deblank", |s| s.trim_end().to_string())?)
}

fn f_strrep(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 3, "strrep")?;
    let (from, to) = (
        str_arg(a, 1, "strrep")?.to_string(),
        str_arg(a, 2, "strrep")?.to_string(),
    );
    one(per_text(&a[0], "strrep", |s| {
        if from.is_empty() {
            s.to_string()
        } else {
            s.replace(&from, &to)
        }
    })?)
}

fn f_blanks(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let n = super::usize_arg(a, 0, "blanks")?;
    if n > 10_000_000 {
        return err("Abaco:limit", "`blanks` is limited to 10 million");
    }
    one(Value::text(" ".repeat(n)))
}

fn f_strcat(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if a.iter().any(|v| matches!(v, Value::Cell(_))) {
        let n = a
            .iter()
            .filter_map(|v| {
                if let Value::Cell(c) = v {
                    Some(c.d.len())
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(1);
        let shape = a
            .iter()
            .find_map(|v| {
                if let Value::Cell(c) = v {
                    Some((c.r, c.c))
                } else {
                    None
                }
            })
            .unwrap_or((1, 1));
        let mut out = vec![String::new(); n];
        for v in a {
            match v {
                Value::Str(s) => out.iter_mut().for_each(|o| o.push_str(s)),
                Value::Cell(c) if c.d.len() == n => {
                    for (o, x) in out.iter_mut().zip(&c.d) {
                        o.push_str(x.as_str("a cell element")?);
                    }
                }
                Value::Cell(c) if c.d.len() == 1 => {
                    let s = c.d[0].as_str("a cell element")?;
                    out.iter_mut().for_each(|o| o.push_str(s));
                }
                _ => return err("Abaco:size", "`strcat` needs cells of the same size"),
            }
        }
        return one(Value::Cell(Arr {
            r: shape.0,
            c: shape.1,
            d: out.into_iter().map(Value::Str).collect(),
        }));
    }
    let mut s = String::new();
    for v in a {
        // trailing white space of text arguments is dropped, as in MATLAB
        s.push_str(v.as_str("a `strcat` argument")?.trim_end());
    }
    one(Value::Str(s))
}

fn f_strsplit(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "strsplit")?;
    let s = str_arg(a, 0, "strsplit")?;
    let delims: Vec<String> = match a.get(1) {
        None => vec![" ".into()],
        Some(Value::Str(d)) => vec![crate::builtins::text::unescape(d)],
        Some(Value::Cell(c)) => {
            c.d.iter()
                .map(|v| v.as_str("a delimiter").map(unescape))
                .collect::<Result<_>>()?
        }
        Some(other) => {
            return err(
                "Abaco:type",
                format!(
                    "`strsplit` needs a delimiter as text, got {}",
                    other.describe()
                ),
            )
        }
    };
    let mut parts = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    'outer: while i < chars.len() {
        for d in &delims {
            let dc: Vec<char> = d.chars().collect();
            if !dc.is_empty() && chars.get(i..i + dc.len()) == Some(&dc[..]) {
                parts.push(std::mem::take(&mut cur));
                i += dc.len();
                continue 'outer;
            }
        }
        cur.push(chars[i]);
        i += 1;
    }
    parts.push(cur);
    one(Value::Cell(Arr::row(
        parts.into_iter().map(Value::Str).collect(),
    )))
}

fn f_strjoin(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "strjoin")?;
    let Value::Cell(c) = &a[0] else {
        return err("Abaco:type", "`strjoin` needs a cell array of text");
    };
    let d = match a.get(1) {
        Some(v) => unescape(v.as_str("the delimiter")?),
        None => " ".into(),
    };
    let parts: Vec<&str> =
        c.d.iter()
            .map(|v| v.as_str("a cell element"))
            .collect::<Result<_>>()?;
    one(Value::text(parts.join(&d)))
}

fn f_strtok(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "strtok")?;
    let s = str_arg(a, 0, "strtok")?;
    let delims: Vec<char> = match a.get(1) {
        Some(v) => v.as_str("the delimiters")?.chars().collect(),
        None => vec![' ', '\t', '\n', '\r', '\0', '\u{b}', '\u{c}'],
    };
    let rest = s.trim_start_matches(|c| delims.contains(&c));
    let end = rest.find(|c| delims.contains(&c)).unwrap_or(rest.len());
    Ok(vec![Value::text(&rest[..end]), Value::text(&rest[end..])])
}

fn compare(a: &[Value], name: &str, f: impl Fn(&str, &str) -> bool) -> Out {
    let n = if name.starts_with("strn") { 3 } else { 2 };
    arity(a, n, n, name)?;
    match (&a[0], &a[1]) {
        (Value::Str(x), Value::Str(y)) => boolean(f(x, y)),
        (Value::Cell(c), Value::Str(y)) | (Value::Str(y), Value::Cell(c)) => {
            bool_arr(c.map(|v| matches!(v, Value::Str(x) if f(x, y))))
        }
        (Value::Cell(p), Value::Cell(q)) => {
            if p.d.len() == q.d.len() {
                bool_arr(Arr {
                    r: p.r,
                    c: p.c,
                    d: p.d
                        .iter()
                        .zip(&q.d)
                        .map(|(x, y)| matches!((x, y), (Value::Str(x), Value::Str(y)) if f(x, y)))
                        .collect(),
                })
            } else if q.d.len() == 1 {
                bool_arr(
                    p.map(|x| matches!((x, &q.d[0]), (Value::Str(x), Value::Str(y)) if f(x, y))),
                )
            } else {
                err(
                    "Abaco:size",
                    format!("`{name}` needs cells of the same size"),
                )
            }
        }
        _ => boolean(false),
    }
}

fn f_strcmp(_: &mut Interp, a: &[Value], _: usize) -> Out {
    compare(a, "strcmp", |x, y| x == y)
}

fn f_strcmpi(_: &mut Interp, a: &[Value], _: usize) -> Out {
    compare(a, "strcmpi", |x, y| x.to_lowercase() == y.to_lowercase())
}

fn prefix_n(a: &[Value], name: &str, ci: bool) -> Out {
    arity(a, 3, 3, name)?;
    let n = super::usize_arg(a, 2, name)?;
    let rest = &a[..2];
    compare_n(rest, name, n, ci)
}

fn compare_n(a: &[Value], name: &str, n: usize, ci: bool) -> Out {
    let norm = |s: &str| -> Option<String> {
        if s.chars().count() < n {
            None
        } else if ci {
            Some(s.chars().take(n).collect::<String>().to_lowercase())
        } else {
            Some(s.chars().take(n).collect())
        }
    };
    let eq = |x: &str, y: &str| match (norm(x), norm(y)) {
        (Some(p), Some(q)) => p == q,
        _ => false,
    };
    let args = [a[0].clone(), a[1].clone()];
    let _ = name;
    match (&args[0], &args[1]) {
        (Value::Str(x), Value::Str(y)) => boolean(eq(x, y)),
        (Value::Cell(c), Value::Str(y)) | (Value::Str(y), Value::Cell(c)) => {
            bool_arr(c.map(|v| matches!(v, Value::Str(x) if eq(x, y))))
        }
        _ => boolean(false),
    }
}

fn f_strncmp(_: &mut Interp, a: &[Value], _: usize) -> Out {
    prefix_n(a, "strncmp", false)
}

fn f_strncmpi(_: &mut Interp, a: &[Value], _: usize) -> Out {
    prefix_n(a, "strncmpi", true)
}

fn text_pred(a: &[Value], name: &str, f: impl Fn(&str, &str) -> bool) -> Out {
    arity(a, 2, 3, name)?;
    let pats: Vec<String> = match &a[1] {
        Value::Str(p) => vec![p.clone()],
        Value::Cell(c) => {
            c.d.iter()
                .map(|v| v.as_str("a pattern").map(String::from))
                .collect::<Result<_>>()?
        }
        other => {
            return err(
                "Abaco:type",
                format!("`{name}` needs text patterns, got {}", other.describe()),
            )
        }
    };
    let ci = a.len() == 3
        && matches!((&a[2], a.get(3)), (Value::Str(k), _) if k.eq_ignore_ascii_case("IgnoreCase"));
    let test = |s: &str| {
        pats.iter().any(|p| {
            if ci {
                f(&s.to_lowercase(), &p.to_lowercase())
            } else {
                f(s, p)
            }
        })
    };
    match &a[0] {
        Value::Str(s) => boolean(test(s)),
        Value::Cell(c) => bool_arr(c.map(|v| matches!(v, Value::Str(s) if test(s)))),
        other => err(
            "Abaco:type",
            format!("`{name}` needs text, got {}", other.describe()),
        ),
    }
}

fn f_contains(_: &mut Interp, a: &[Value], _: usize) -> Out {
    text_pred(a, "contains", |s, p| s.contains(p))
}

fn f_startswith(_: &mut Interp, a: &[Value], _: usize) -> Out {
    text_pred(a, "startsWith", |s, p| s.starts_with(p))
}

fn f_endswith(_: &mut Interp, a: &[Value], _: usize) -> Out {
    text_pred(a, "endsWith", |s, p| s.ends_with(p))
}

fn f_strfind(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "strfind")?;
    let pat: Vec<char> = str_arg(a, 1, "strfind")?.chars().collect();
    let find = |s: &str| -> Value {
        let h: Vec<char> = s.chars().collect();
        let mut hits = Vec::new();
        if !pat.is_empty() && h.len() >= pat.len() {
            for i in 0..=h.len() - pat.len() {
                if h[i..i + pat.len()] == pat[..] {
                    hits.push(i as f64 + 1.0);
                }
            }
        }
        if hits.is_empty() {
            Value::Num(Arr::new(1, 0, Vec::new()))
        } else {
            Value::row(hits)
        }
    };
    match &a[0] {
        Value::Str(s) => one(find(s)),
        Value::Cell(c) => one(Value::Cell(c.map(|v| {
            if let Value::Str(s) = v {
                find(s)
            } else {
                Value::empty()
            }
        }))),
        other => err(
            "Abaco:type",
            format!("`strfind` needs text, got {}", other.describe()),
        ),
    }
}

fn f_char(_: &mut Interp, a: &[Value], _: usize) -> Out {
    if a.len() != 1 {
        return err(
            "Abaco:arity",
            "`char` takes one argument; multi-row character arrays are not supported",
        );
    }
    match &a[0] {
        Value::Str(s) => one(Value::Str(s.clone())),
        Value::Cell(_) => err(
            "Abaco:type",
            "`char` of a cell is not supported; use strjoin",
        ),
        v => {
            let x = v.to_real()?;
            if x.r > 1 && x.c > 1 {
                return err(
                    "Abaco:size",
                    "`char` of a matrix is not supported; give a vector of codes",
                );
            }
            let mut s = String::new();
            for &c in &x.d {
                match char::from_u32(c as u32) {
                    Some(ch) if c >= 0.0 && c.fract() == 0.0 => s.push(ch),
                    _ => return err("Abaco:domain", format!("{c} is not a valid character code")),
                }
            }
            one(Value::Str(s))
        }
    }
}

fn char_class(a: &[Value], name: &str, f: fn(char) -> bool) -> Out {
    arity(a, 1, 1, name)?;
    match &a[0] {
        Value::Str(s) => {
            let d: Vec<bool> = s.chars().map(f).collect();
            bool_arr(Arr::new(1, d.len(), d))
        }
        v => {
            let (r, c) = v.dims();
            bool_arr(Arr::filled(r, c, false))
        }
    }
}

fn f_isspace(_: &mut Interp, a: &[Value], _: usize) -> Out {
    char_class(a, "isspace", char::is_whitespace)
}

fn f_isletter(_: &mut Interp, a: &[Value], _: usize) -> Out {
    char_class(a, "isletter", char::is_alphabetic)
}

fn f_isdigit(_: &mut Interp, a: &[Value], _: usize) -> Out {
    char_class(a, "isdigit", |c| c.is_ascii_digit())
}

fn f_isupper(_: &mut Interp, a: &[Value], _: usize) -> Out {
    char_class(a, "isupper", char::is_uppercase)
}

fn f_islower(_: &mut Interp, a: &[Value], _: usize) -> Out {
    char_class(a, "islower", char::is_lowercase)
}

fn f_dec2bin(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "dec2bin")?;
    let n = super::scalar_arg(a, 0, "dec2bin")?;
    if n < 0.0 || n.fract() != 0.0 || n >= 1.8e19 {
        return err(
            "Abaco:domain",
            "`dec2bin` needs a non-negative whole number",
        );
    }
    let width = if a.len() > 1 {
        super::usize_arg(a, 1, "dec2bin")?.min(64)
    } else {
        0
    };
    one(Value::text(format!("{:0width$b}", n as u64)))
}

fn f_dec2hex(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "dec2hex")?;
    let n = super::scalar_arg(a, 0, "dec2hex")?;
    if n < 0.0 || n.fract() != 0.0 || n >= 1.8e19 {
        return err(
            "Abaco:domain",
            "`dec2hex` needs a non-negative whole number",
        );
    }
    let width = if a.len() > 1 {
        super::usize_arg(a, 1, "dec2hex")?.min(16)
    } else {
        0
    };
    one(Value::text(format!("{:0width$X}", n as u64)))
}

fn f_bin2dec(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "bin2dec")?;
    let s = str_arg(a, 0, "bin2dec")?;
    num(u64::from_str_radix(s.trim(), 2).map_or(f64::NAN, |v| v as f64))
}

fn f_hex2dec(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "hex2dec")?;
    let s = str_arg(a, 0, "hex2dec")?;
    num(u64::from_str_radix(s.trim(), 16).map_or(f64::NAN, |v| v as f64))
}

// ---------- regular expressions ----------

fn build_regex(pat: &str, ci: bool) -> Result<regex::Regex> {
    RegexBuilder::new(pat)
        .case_insensitive(ci)
        .size_limit(5_000_000)
        .build()
        .map_err(|e| {
            crate::error::Error::new("Abaco:regexp", format!("invalid regular expression: {e}"))
        })
}

fn char_index(s: &str, byte: usize) -> usize {
    s.get(..byte).map_or(0, |p| p.chars().count())
}

fn regexp_impl(a: &[Value], nargout: usize, ci: bool) -> Out {
    if a.len() < 2 {
        return err("Abaco:arity", "`regexp` needs a string and a pattern");
    }
    let s = str_arg(a, 0, "regexp")?;
    let re = build_regex(
        str_arg(a, 1, "regexp")?,
        ci || a[2..]
            .iter()
            .any(|v| matches!(v, Value::Str(o) if o == "ignorecase")),
    )?;
    let once = a[2..]
        .iter()
        .any(|v| matches!(v, Value::Str(o) if o == "once"));
    let order: Vec<String> = a[2..]
        .iter()
        .filter_map(|v| match v {
            Value::Str(o)
                if matches!(
                    o.as_str(),
                    "match" | "tokens" | "names" | "start" | "end" | "split" | "tokenExtents"
                ) =>
            {
                Some(o.clone())
            }
            _ => None,
        })
        .collect();
    let order: Vec<String> = if order.is_empty() {
        [
            "start",
            "end",
            "tokenExtents",
            "match",
            "tokens",
            "names",
            "split",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
    } else {
        order
    };
    let caps: Vec<regex::Captures<'_>> = if once {
        re.captures(s).into_iter().collect()
    } else {
        re.captures_iter(s).collect()
    };
    let tokens_of = |c: &regex::Captures<'_>| -> Value {
        let groups: Vec<Value> = if c.len() > 1 {
            (1..c.len())
                .map(|k| Value::text(c.get(k).map_or("", |m| m.as_str())))
                .collect()
        } else {
            vec![Value::text(c.get(0).map_or("", |m| m.as_str()))]
        };
        Value::Cell(Arr::row(groups))
    };
    let mut outs = Vec::new();
    for key in order.iter().take(nargout.max(1)) {
        let v = match key.as_str() {
            "match" => {
                if once {
                    caps.first().map_or(Value::text(""), |c| {
                        Value::text(c.get(0).map_or("", |m| m.as_str()))
                    })
                } else {
                    Value::Cell(Arr::row(
                        caps.iter()
                            .map(|c| Value::text(c.get(0).map_or("", |m| m.as_str())))
                            .collect(),
                    ))
                }
            }
            "tokens" => {
                if once {
                    caps.first().map_or(Value::Cell(Arr::empty()), tokens_of)
                } else {
                    Value::Cell(Arr::row(caps.iter().map(tokens_of).collect()))
                }
            }
            "start" | "end" => {
                let f = |c: &regex::Captures<'_>| {
                    c.get(0).map_or(0.0, |m| {
                        if key == "start" {
                            char_index(s, m.start()) as f64 + 1.0
                        } else {
                            char_index(s, m.end()) as f64
                        }
                    })
                };
                if once {
                    caps.first().map_or(Value::empty(), |c| Value::num(f(c)))
                } else if caps.is_empty() {
                    Value::Num(Arr::new(1, 0, Vec::new()))
                } else {
                    Value::row(caps.iter().map(f).collect())
                }
            }
            "tokenExtents" => Value::Cell(Arr::row(
                caps.iter()
                    .map(|c| {
                        let ext: Vec<f64> = (1..c.len().max(2))
                            .flat_map(|k| {
                                c.get(k).map_or(vec![0.0, 0.0], |m| {
                                    vec![
                                        char_index(s, m.start()) as f64 + 1.0,
                                        char_index(s, m.end()) as f64,
                                    ]
                                })
                            })
                            .collect();
                        Value::mat(ext.len() / 2, 2, {
                            let n = ext.len() / 2;
                            let mut d = vec![0.0; ext.len()];
                            for k in 0..n {
                                d[k] = ext[2 * k];
                                d[n + k] = ext[2 * k + 1];
                            }
                            d
                        })
                    })
                    .collect(),
            )),
            "names" => {
                let mut st = crate::value::StructV::default();
                for name in re.capture_names().flatten() {
                    let val = caps
                        .first()
                        .and_then(|c| c.name(name))
                        .map_or("", |m| m.as_str());
                    st.set(name, Value::text(val));
                }
                Value::Struct(Box::new(st))
            }
            "split" => {
                let mut parts = Vec::new();
                let mut last = 0;
                for c in &caps {
                    if let Some(m) = c.get(0) {
                        parts.push(Value::text(&s[last..m.start()]));
                        last = m.end();
                    }
                }
                parts.push(Value::text(&s[last..]));
                Value::Cell(Arr::row(parts))
            }
            _ => Value::empty(),
        };
        outs.push(v);
    }
    Ok(outs)
}

fn f_regexp(_: &mut Interp, a: &[Value], n: usize) -> Out {
    regexp_impl(a, n, false)
}

fn f_regexpi(_: &mut Interp, a: &[Value], n: usize) -> Out {
    regexp_impl(a, n, true)
}

fn f_regexprep(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 5, "regexprep")?;
    let pat = str_arg(a, 1, "regexprep")?;
    let rep = str_arg(a, 2, "regexprep")?;
    let ci = a[3..]
        .iter()
        .any(|v| matches!(v, Value::Str(o) if o == "ignorecase"));
    let once = a[3..]
        .iter()
        .any(|v| matches!(v, Value::Str(o) if o == "once"));
    let re = build_regex(pat, ci)?;
    // MATLAB writes $1; the regex crate wants ${1} so that "$1a" is not read as a group named "1a"
    let mut r = String::new();
    let cs: Vec<char> = unescape(rep).chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '$' && cs.get(i + 1).is_some_and(char::is_ascii_digit) {
            let mut j = i + 1;
            while cs.get(j).is_some_and(char::is_ascii_digit) {
                j += 1;
            }
            r.push_str("${");
            r.extend(&cs[i + 1..j]);
            r.push('}');
            i = j;
        } else if cs[i] == '$' {
            r.push_str("$$");
            i += 1;
        } else {
            r.push(cs[i]);
            i += 1;
        }
    }
    one(per_text(&a[0], "regexprep", |s| {
        if once {
            re.replace(s, r.as_str()).into_owned()
        } else {
            re.replace_all(s, r.as_str()).into_owned()
        }
    })?)
}

pub const TABLE: &[(&str, Bf)] = &[
    ("sprintf", f_sprintf),
    ("fprintf", f_fprintf),
    ("printf", f_printf),
    ("disp", f_disp),
    ("display", f_display),
    ("num2str", f_num2str),
    ("int2str", f_int2str),
    ("mat2str", f_mat2str),
    ("str2double", f_str2double),
    ("str2num", f_str2num),
    ("upper", f_upper),
    ("toupper", f_upper),
    ("lower", f_lower),
    ("tolower", f_lower),
    ("strtrim", f_strtrim),
    ("deblank", f_deblank),
    ("strrep", f_strrep),
    ("blanks", f_blanks),
    ("strcat", f_strcat),
    ("strsplit", f_strsplit),
    ("strjoin", f_strjoin),
    ("strtok", f_strtok),
    ("strcmp", f_strcmp),
    ("strcmpi", f_strcmpi),
    ("strncmp", f_strncmp),
    ("strncmpi", f_strncmpi),
    ("contains", f_contains),
    ("startsWith", f_startswith),
    ("endsWith", f_endswith),
    ("strfind", f_strfind),
    ("char", f_char),
    ("isspace", f_isspace),
    ("isletter", f_isletter),
    ("isalpha", f_isletter),
    ("isdigit", f_isdigit),
    ("isupper", f_isupper),
    ("islower", f_islower),
    ("dec2bin", f_dec2bin),
    ("dec2hex", f_dec2hex),
    ("bin2dec", f_bin2dec),
    ("hex2dec", f_hex2dec),
    ("regexp", f_regexp),
    ("regexpi", f_regexpi),
    ("regexprep", f_regexprep),
];
