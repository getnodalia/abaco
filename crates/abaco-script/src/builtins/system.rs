// SPDX-License-Identifier: MIT OR Apache-2.0
//! The system: errors and warnings, timing, `eval`, workspace queries and bit operations.

use super::text::sprintf;
use super::{arg, arity, boolean, is_flag, num, one, real_arg, real_out, str_arg, Bf, Out};
use crate::error::{err, Error, Result};
use crate::interp::{error_struct, Event, Interp};
use crate::value::{Arr, Value};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ---------- errors ----------

/// `true` when `s` looks like a message identifier such as `Pkg:name`.
fn looks_like_id(s: &str) -> bool {
    if !s.contains(':') || s.contains(char::is_whitespace) {
        return false;
    }
    s.split(':').all(|part| {
        let mut chars = part.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

/// Splits `(id?, fmt, args...)` into an identifier and a formatted message.
fn message_from(args: &[Value], name: &str) -> Result<(String, String)> {
    let first = str_arg(args, 0, name)?;
    if args.len() >= 2 && looks_like_id(first) {
        let fmt = str_arg(args, 1, name)?;
        let msg = sprintf(fmt, &args[2..])?;
        return Ok((first.to_string(), msg));
    }
    if args.len() == 1 {
        return Ok((String::new(), first.to_string()));
    }
    Ok((String::new(), sprintf(first, &args[1..])?))
}

fn error_from_struct(s: &Value) -> Option<Error> {
    let Value::Struct(st) = s else {
        return None;
    };
    let message = match st.get("message") {
        Some(Value::Str(m)) => m.clone(),
        _ => return None,
    };
    let id = match st.get("identifier") {
        Some(Value::Str(i)) if !i.is_empty() => i.clone(),
        _ => "Abaco:error".to_string(),
    };
    Some(Error::new(&id, message))
}

fn f_error(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, usize::MAX, "error")?;
    if let Some(e) = error_from_struct(&a[0]) {
        return Err(e);
    }
    let (id, msg) = message_from(a, "error")?;
    Err(Error::new(
        if id.is_empty() { "Abaco:error" } else { &id },
        msg,
    ))
}

fn f_rethrow(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "rethrow")?;
    match error_from_struct(&a[0]) {
        Some(e) => Err(e),
        None => err(
            "Abaco:type",
            "`rethrow` needs a struct with `message` and `identifier` fields",
        ),
    }
}

fn f_mexception(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, usize::MAX, "MException")?;
    let (id, msg) = message_from(a, "MException")?;
    let e = Error::new(&id, msg);
    one(error_struct(&e))
}

fn f_lasterr(i: &mut Interp, _: &[Value], _: usize) -> Out {
    one(Value::text(
        i.last_error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_default(),
    ))
}

fn f_warning(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, usize::MAX, "warning")?;
    let first = str_arg(a, 0, "warning")?;
    if matches!(first, "on" | "off" | "error" | "query") {
        return Ok(vec![]);
    }
    let (_, msg) = message_from(a, "warning")?;
    i.warn(msg);
    Ok(vec![])
}

fn f_assert(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, usize::MAX, "assert")?;
    // assert(a, b) and assert(a, b, tol) compare; assert(cond, msg, ...) checks a condition.
    let comparing =
        a.len() >= 2 && !matches!(a[1], Value::Str(_)) && !matches!(a[0], Value::Bool(_));
    if comparing {
        let tol = if a.len() >= 3 {
            a[2].scalar_f64("the tolerance of `assert`")?
        } else {
            0.0
        };
        let ok = if tol == 0.0 {
            super::structs::values_equal(&a[0], &a[1])
        } else {
            let x = a[0].to_real()?;
            let y = a[1].to_real()?;
            x.r == y.r
                && x.c == y.c
                && x.d.iter().zip(&y.d).all(|(p, q)| {
                    let bound = if tol < 0.0 { tol.abs() * q.abs() } else { tol };
                    (p - q).abs() <= bound || (p.is_nan() && q.is_nan())
                })
        };
        if ok {
            return Ok(vec![]);
        }
        return err("Abaco:assert", "assertion failed: the two values differ");
    }
    if a[0].is_true()? {
        return Ok(vec![]);
    }
    if a.len() >= 2 {
        let (id, msg) = message_from(&a[1..], "assert")?;
        return Err(Error::new(
            if id.is_empty() { "Abaco:assert" } else { &id },
            msg,
        ));
    }
    err("Abaco:assert", "assertion failed")
}

fn f_narginchk(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "narginchk")?;
    let lo = a[0].scalar_f64("the minimum")?;
    let hi = a[1].scalar_f64("the maximum")?;
    let n = i.nargin() as f64;
    if n < lo {
        return err("Abaco:arity", "not enough input arguments");
    }
    if n > hi {
        return err("Abaco:arity", "too many input arguments");
    }
    Ok(vec![])
}

fn f_nargoutchk(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "nargoutchk")?;
    let lo = a[0].scalar_f64("the minimum")?;
    let hi = a[1].scalar_f64("the maximum")?;
    let n = i.nargout() as f64;
    if n < lo {
        return err("Abaco:arity", "not enough output arguments");
    }
    if n > hi {
        return err("Abaco:arity", "too many output arguments");
    }
    Ok(vec![])
}

fn f_print_usage(_: &mut Interp, _: &[Value], _: usize) -> Out {
    err(
        "Abaco:usage",
        "invalid call; check the number and kind of arguments",
    )
}

fn f_input(_: &mut Interp, _: &[Value], _: usize) -> Out {
    err("Abaco:io", "`input` and `keyboard` are not available: scripts run without a console to read from; pass values as arguments instead")
}

// ---------- time ----------

fn base_instant() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

fn since_base(t: Instant) -> f64 {
    t.saturating_duration_since(base_instant()).as_secs_f64()
}

fn f_tic(i: &mut Interp, _: &[Value], nargout: usize) -> Out {
    let now = Instant::now();
    if nargout > 0 {
        return num(since_base(now));
    }
    i.tic = Some(now);
    Ok(vec![])
}

fn f_toc(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    let now = Instant::now();
    let start = match a.first() {
        Some(v) => v.scalar_f64("the timer of `toc`")?,
        None => match i.tic {
            Some(t) => since_base(t),
            None => return err("Abaco:toc", "call `tic` before `toc`"),
        },
    };
    let dt = (since_base(now) - start).max(0.0);
    if nargout > 0 {
        return num(dt);
    }
    i.print(&format!("Elapsed time is {dt:.6} seconds.\n"));
    Ok(vec![])
}

fn f_cputime(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(since_base(Instant::now()))
}

fn f_pause(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 0, 1, "pause")?;
    // Scripts never block the interface; a pause only honours a pending stop request.
    i.check_cancel()?;
    Ok(vec![])
}

fn f_noop(_: &mut Interp, _: &[Value], _: usize) -> Out {
    Ok(vec![])
}

/// The civil date for a count of days since 1970-01-01 (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn f_clock(_: &mut Interp, _: &[Value], _: usize) -> Out {
    let t = unix_seconds();
    let days = (t / 86_400.0).floor();
    let secs = t - days * 86_400.0;
    let (y, m, d) = civil_from_days(days as i64);
    let h = (secs / 3600.0).floor();
    let mi = ((secs - h * 3600.0) / 60.0).floor();
    let s = secs - h * 3600.0 - mi * 60.0;
    one(Value::row(vec![y as f64, m as f64, d as f64, h, mi, s]))
}

fn f_now(_: &mut Interp, _: &[Value], _: usize) -> Out {
    // Days since year 0, as MATLAB's datenum counts them (1970-01-01 is 719529).
    num(719_529.0 + unix_seconds() / 86_400.0)
}

// ---------- eval and the workspace ----------

fn f_eval(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "eval")?;
    let code = str_arg(a, 0, "eval")?.to_string();
    match i.eval_code(&code) {
        Ok(()) => Ok(vec![]),
        Err(e) if a.len() == 2 && e.id != "Abaco:cancelled" && e.id != "Abaco:limit" => {
            i.last_error = Some(e);
            let fallback = str_arg(a, 1, "eval")?.to_string();
            i.eval_code(&fallback)?;
            Ok(vec![])
        }
        Err(e) => Err(e),
    }
}

fn f_evalc(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "evalc")?;
    let code = str_arg(a, 0, "evalc")?.to_string();
    let before = i.take_events();
    let result = i.eval_code(&code);
    let produced = i.take_events();
    let mut text = String::new();
    let mut rest = before;
    for e in produced {
        match e {
            Event::Text(t) => text.push_str(&t),
            other => rest.push(other),
        }
    }
    for e in rest {
        i.emit(e);
    }
    result?;
    one(Value::text(text))
}

fn is_varname(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && crate::lexer::keyword(s).is_none()
}

fn f_isvarname(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "isvarname")?;
    boolean(matches!(&a[0], Value::Str(s) if is_varname(s)))
}

fn f_exist(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "exist")?;
    let name = str_arg(a, 0, "exist")?;
    let code = if i.get(name).is_some() && !is_flag(a, 1, "builtin") {
        1.0
    } else if i.find_m_file(name).is_some() {
        2.0
    } else if i.has_function(name) {
        103.0
    } else if super::lookup(name).is_some() {
        5.0
    } else {
        0.0
    };
    num(code)
}

fn f_who(i: &mut Interp, _: &[Value], _: usize) -> Out {
    let mut names: Vec<String> = i.variables().into_iter().map(|(n, _)| n).collect();
    names.sort();
    if names.is_empty() {
        return Ok(vec![]);
    }
    i.print(&format!("Your variables are:\n{}\n", names.join("  ")));
    Ok(vec![])
}

fn f_format(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 0, 1, "format")?;
    i.format_long = is_flag(a, 0, "long");
    Ok(vec![])
}

fn f_clc(i: &mut Interp, _: &[Value], _: usize) -> Out {
    i.emit(Event::ClearScreen);
    Ok(vec![])
}

fn f_clear(i: &mut Interp, a: &[Value], _: usize) -> Out {
    let names: Result<Vec<String>> = a
        .iter()
        .map(|v| v.as_str("a name for `clear`").map(String::from))
        .collect();
    i.run_command("clear", &names?)?;
    Ok(vec![])
}

fn dir_args(a: &[Value], who: &str) -> Result<(Vec<std::path::PathBuf>, bool)> {
    let (mut dirs, mut front) = (Vec::new(), true);
    for v in a {
        let s = v.as_str(&format!("a folder name for `{who}`"))?;
        match s {
            "-end" | "-END" => front = false,
            "-begin" | "-BEGIN" => front = true,
            _ => dirs.extend(
                s.split(if cfg!(windows) { ';' } else { ':' })
                    .filter(|d| !d.is_empty())
                    .map(std::path::PathBuf::from),
            ),
        }
    }
    Ok((dirs, front))
}

fn f_addpath(i: &mut Interp, a: &[Value], _: usize) -> Out {
    if !i.files_allowed() {
        return err(
            "Abaco:files",
            "`addpath`: this run is not allowed to read or write files",
        );
    }
    arity(a, 1, usize::MAX, "addpath")?;
    let (dirs, front) = dir_args(a, "addpath")?;
    for d in &dirs {
        if !d.is_dir() {
            return err(
                "Abaco:file",
                format!("`addpath`: `{}` is not a folder", d.display()),
            );
        }
    }
    for d in dirs.into_iter().rev() {
        i.add_path(d, front);
    }
    Ok(vec![])
}

fn f_rmpath(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, usize::MAX, "rmpath")?;
    let (dirs, _) = dir_args(a, "rmpath")?;
    for d in dirs {
        if !i.remove_path(&d) {
            i.warn(format!("rmpath: `{}` was not on the path", d.display()));
        }
    }
    Ok(vec![])
}

fn f_path(i: &mut Interp, _: &[Value], _: usize) -> Out {
    let list: Vec<String> = i
        .search_path()
        .iter()
        .map(|d| d.display().to_string())
        .collect();
    one(Value::text(list.join(if cfg!(windows) {
        ";"
    } else {
        ":"
    })))
}

fn f_pwd(i: &mut Interp, _: &[Value], _: usize) -> Out {
    if !i.files_allowed() {
        return err(
            "Abaco:files",
            "`pwd`: this run is not allowed to read or write files",
        );
    }
    match std::env::current_dir() {
        Ok(d) => one(Value::text(d.display().to_string())),
        Err(e) => err("Abaco:file", format!("`pwd`: {e}")),
    }
}

/// `run('file')` runs a script file (`.m` added when the name has no extension) in the current
/// workspace.
fn f_run(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "run")?;
    let name = str_arg(a, 0, "run")?;
    let mut path = std::path::PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("m");
    }
    if !path.is_file() {
        let found = i
            .search_path()
            .iter()
            .map(|d| d.join(&path))
            .find(|p| p.is_file());
        match found {
            Some(p) => path = p,
            None => {
                return err(
                    "Abaco:file",
                    format!("`run`: cannot find `{}`", path.display()),
                )
            }
        }
    }
    i.run_script_file(&path)?;
    Ok(vec![])
}

fn f_version(_: &mut Interp, _: &[Value], _: usize) -> Out {
    one(Value::text(env!("CARGO_PKG_VERSION")))
}

fn f_computer(_: &mut Interp, _: &[Value], _: usize) -> Out {
    one(Value::text(match std::env::consts::OS {
        "linux" => "GLNXA64",
        "macos" => "MACI64",
        "windows" => "PCWIN64",
        other => other,
    }))
}

fn f_ispc(_: &mut Interp, _: &[Value], _: usize) -> Out {
    boolean(cfg!(windows))
}

fn f_ismac(_: &mut Interp, _: &[Value], _: usize) -> Out {
    boolean(cfg!(target_os = "macos"))
}

fn f_isunix(_: &mut Interp, _: &[Value], _: usize) -> Out {
    boolean(cfg!(unix))
}

fn f_false(_: &mut Interp, _: &[Value], _: usize) -> Out {
    boolean(false)
}

fn f_mfilename(_: &mut Interp, _: &[Value], _: usize) -> Out {
    one(Value::text(""))
}

fn f_stdout(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(1.0)
}

fn f_stderr(_: &mut Interp, _: &[Value], _: usize) -> Out {
    num(2.0)
}

// ---------- integer limits and bit operations ----------

fn int_bounds(class: &str) -> Option<(f64, f64)> {
    Some(match class {
        "int8" => (-128.0, 127.0),
        "uint8" => (0.0, 255.0),
        "int16" => (-32_768.0, 32_767.0),
        "uint16" => (0.0, 65_535.0),
        "int32" => (-2_147_483_648.0, 2_147_483_647.0),
        "uint32" => (0.0, 4_294_967_295.0),
        "int64" => (-9_223_372_036_854_775_808.0, 9_223_372_036_854_775_807.0),
        "uint64" => (0.0, 18_446_744_073_709_551_615.0),
        _ => return None,
    })
}

fn int_limit(a: &[Value], name: &str, upper: bool) -> Out {
    arity(a, 0, 1, name)?;
    let class = if a.is_empty() {
        "int32"
    } else {
        str_arg(a, 0, name)?
    };
    match int_bounds(class) {
        Some((lo, hi)) => num(if upper { hi } else { lo }),
        None => err(
            "Abaco:type",
            format!(
                "`{name}`: `{class}` is not an integer class; use int8 … int64 or uint8 … uint64"
            ),
        ),
    }
}

fn f_intmax(_: &mut Interp, a: &[Value], _: usize) -> Out {
    int_limit(a, "intmax", true)
}

fn f_intmin(_: &mut Interp, a: &[Value], _: usize) -> Out {
    int_limit(a, "intmin", false)
}

/// Reads a bit-operation operand as a non-negative whole number below 2^53.
fn bits(x: f64, name: &str) -> Result<u64> {
    if x.is_nan() || x < 0.0 || x.fract() != 0.0 || x > 9_007_199_254_740_992.0 {
        return err(
            "Abaco:value",
            format!("`{name}` works on whole numbers from 0 to 2^53, got {x}"),
        );
    }
    Ok(x as u64)
}

fn unwrap_all(a: Arr<Result<f64>>) -> Result<Arr<f64>> {
    let mut d = Vec::with_capacity(a.d.len());
    for x in a.d {
        d.push(x?);
    }
    Ok(Arr { r: a.r, c: a.c, d })
}

fn bitop(a: &[Value], name: &str, f: fn(u64, u64) -> u64) -> Out {
    arity(a, 2, 2, name)?;
    let x = real_arg(a, 0, name)?;
    let y = real_arg(a, 1, name)?;
    let z = crate::ops::zip(&x, &y, |&p, &q| match (bits(p, name), bits(q, name)) {
        (Ok(p), Ok(q)) => Ok(f(p, q) as f64),
        (Err(e), _) | (_, Err(e)) => Err(e),
    })?;
    real_out(unwrap_all(z)?)
}

fn f_bitand(_: &mut Interp, a: &[Value], _: usize) -> Out {
    bitop(a, "bitand", |p, q| p & q)
}

fn f_bitor(_: &mut Interp, a: &[Value], _: usize) -> Out {
    bitop(a, "bitor", |p, q| p | q)
}

fn f_bitxor(_: &mut Interp, a: &[Value], _: usize) -> Out {
    bitop(a, "bitxor", |p, q| p ^ q)
}

fn f_bitshift(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "bitshift")?;
    let x = real_arg(a, 0, "bitshift")?;
    let n = real_arg(a, 1, "bitshift")?;
    let z = crate::ops::zip(&x, &n, |&p, &k| {
        let p = bits(p, "bitshift")?;
        if k.fract() != 0.0 || k.abs() > 53.0 {
            return err(
                "Abaco:value",
                "`bitshift` shifts by a whole number of at most 53 places",
            );
        }
        let k = k as i64;
        let shifted = if k >= 0 {
            p.checked_shl(k as u32).unwrap_or(0)
        } else {
            p >> (-k) as u32
        };
        // Stay inside the exactly-representable range.
        Ok((shifted & ((1u64 << 53) - 1)) as f64)
    })?;
    real_out(unwrap_all(z)?)
}

fn f_numeric_flags(_: &mut Interp, a: &[Value], _: usize) -> Out {
    // `isdeployed`-style probes that are always false here.
    let _ = arg(a, 0, "flag").ok();
    one(Value::Bool(Arr::scalar(false)))
}

pub const TABLE: &[(&str, Bf)] = &[
    ("MException", f_mexception),
    ("assert", f_assert),
    ("beep", f_noop),
    ("bitand", f_bitand),
    ("bitor", f_bitor),
    ("bitshift", f_bitshift),
    ("bitxor", f_bitxor),
    ("clc", f_clc),
    ("clear", f_clear),
    ("clock", f_clock),
    ("computer", f_computer),
    ("cputime", f_cputime),
    ("drawnow", f_noop),
    ("error", f_error),
    ("eval", f_eval),
    ("evalc", f_evalc),
    ("addpath", f_addpath),
    ("exist", f_exist),
    ("path", f_path),
    ("pwd", f_pwd),
    ("rmpath", f_rmpath),
    ("run", f_run),
    ("fflush", f_noop),
    ("format", f_format),
    ("input", f_input),
    ("intmax", f_intmax),
    ("intmin", f_intmin),
    ("isdeployed", f_false),
    ("ismac", f_ismac),
    ("ispc", f_ispc),
    ("isunix", f_isunix),
    ("isvarname", f_isvarname),
    ("keyboard", f_input),
    ("lasterr", f_lasterr),
    ("mfilename", f_mfilename),
    ("narginchk", f_narginchk),
    ("nargoutchk", f_nargoutchk),
    ("now", f_now),
    ("pause", f_pause),
    ("print_usage", f_print_usage),
    ("rethrow", f_rethrow),
    ("stderr", f_stderr),
    ("stdout", f_stdout),
    ("throw", f_rethrow),
    ("tic", f_tic),
    ("toc", f_toc),
    ("usejava", f_numeric_flags),
    ("version", f_version),
    ("warning", f_warning),
    ("who", f_who),
    ("whos", f_who),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_recognised() {
        assert!(looks_like_id("Pkg:thing"));
        assert!(looks_like_id("a:b:c"));
        assert!(!looks_like_id("Value: bad"));
        assert!(!looks_like_id("plain"));
    }

    #[test]
    fn civil_dates_round_trip_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn varnames_reject_keywords_and_digits() {
        assert!(is_varname("x_1"));
        assert!(!is_varname("1x"));
        assert!(!is_varname("end"));
    }
}
