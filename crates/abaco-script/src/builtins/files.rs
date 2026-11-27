// SPDX-License-Identifier: MIT OR Apache-2.0
//! Files: delimited matrices (`readmatrix`, `writematrix`, `csvread`, `dlmread` and friends),
//! text (`fileread`) and workspaces (`save`, `load`). Everything here is off unless the host
//! turned `Limits::files` on, every file read is size-capped, and every file written is
//! replaced atomically (written beside it, then renamed).

use super::{arg, arity, is_flag, one, real_arg, str_arg, Bf, Out};
use crate::error::{err, Result};
use crate::interp::Interp;
use crate::ops::MAX_ELEMENTS;
use crate::value::{Arr, FnValue, StructV, Value};
use num_complex::Complex64;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The largest file a script may read.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// The deepest nesting of cells and structs in a saved workspace.
const MAX_DEPTH: usize = 64;
/// The most variables a saved workspace may hold.
const MAX_VARS: usize = 100_000;
/// The first line of a saved workspace.
const MAGIC: &str = "ABACO-DATA 1";
/// The extension `save` adds when the name has none.
const EXT: &str = "abd";

fn need_files(i: &Interp, name: &str) -> Result<()> {
    if i.files_allowed() {
        Ok(())
    } else {
        err(
            "Abaco:files",
            format!("`{name}`: this run is not allowed to read or write files"),
        )
    }
}

fn io_error(what: &str, path: &Path, e: &std::io::Error) -> crate::error::Error {
    crate::error::Error::new(
        "Abaco:file",
        format!("cannot {what} `{}`: {e}", path.display()),
    )
}

fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| io_error("read", path, &e))?;
    if !meta.is_file() {
        return err("Abaco:file", format!("`{}` is not a file", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return err(
            "Abaco:limit",
            format!(
                "`{}` is {} MB; scripts may read files up to {} MB",
                path.display(),
                meta.len() / (1024 * 1024),
                MAX_FILE_BYTES / (1024 * 1024)
            ),
        );
    }
    std::fs::read(path).map_err(|e| io_error("read", path, &e))
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = read_bytes(path)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

/// Writes `bytes` to a temporary file beside `path` and renames it over `path`, so a failed
/// write never leaves half a file behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return err(
            "Abaco:file",
            format!("`{}` is not a file name", path.display()),
        );
    }
    let tmp: PathBuf = path.with_file_name(format!(".{name}.abaco-tmp"));
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_error("write", path, &e));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_error("write", path, &e));
    }
    Ok(())
}

// ---------- delimited text ----------

/// How a delimited file separates its fields.
#[derive(Clone, Copy, PartialEq)]
enum Delim {
    /// Decide from the first data line.
    Auto,
    /// Runs of blanks.
    Space,
    Char(char),
}

fn delim_from(name: &str) -> Result<Delim> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "comma" | "," => Delim::Char(','),
        "semi" | "semicolon" | ";" => Delim::Char(';'),
        "tab" | "\\t" | "\t" => Delim::Char('\t'),
        "space" | " " => Delim::Space,
        "pipe" | "|" => Delim::Char('|'),
        "" | "auto" => Delim::Auto,
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Delim::Char(c),
                _ => {
                    return err(
                        "Abaco:type",
                        format!(
                            "the delimiter `{name}` is not one of comma, semi, tab, space, pipe or a single character"
                        ),
                    )
                }
            }
        }
    })
}

fn detect(line: &str) -> Delim {
    if line.contains(',') {
        Delim::Char(',')
    } else if line.contains(';') {
        Delim::Char(';')
    } else if line.contains('\t') {
        Delim::Char('\t')
    } else {
        Delim::Space
    }
}

fn fields(line: &str, d: Delim) -> Vec<&str> {
    match d {
        Delim::Space | Delim::Auto => line.split_whitespace().collect(),
        Delim::Char(c) => line.split(c).map(str::trim).collect(),
    }
}

fn number(field: &str) -> Option<f64> {
    let t = field.trim().trim_matches('"');
    if t.is_empty() {
        None
    } else {
        t.parse::<f64>().ok()
    }
}

/// A matrix from delimited text. Leading lines without a single number are headers; a
/// field that is not a number (or is empty) becomes NaN; short rows are padded with NaN.
fn parse_matrix(text: &str, delim: Delim, header_lines: Option<usize>) -> Result<Arr<f64>> {
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut delim = delim;
    let mut width = 0usize;
    let mut skipped = 0usize;
    for line in text.lines() {
        if let Some(n) = header_lines {
            if skipped < n {
                skipped += 1;
                continue;
            }
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('%') || t.starts_with('#') {
            continue;
        }
        if delim == Delim::Auto {
            delim = detect(t);
        }
        let parts = fields(t, delim);
        let vals: Vec<Option<f64>> = parts.iter().map(|f| number(f)).collect();
        if header_lines.is_none() && rows.is_empty() && vals.iter().all(Option::is_none) {
            continue;
        }
        width = width.max(vals.len());
        if width.saturating_mul(rows.len() + 1) > MAX_ELEMENTS {
            return err(
                "Abaco:limit",
                format!("the file has more than {MAX_ELEMENTS} values"),
            );
        }
        rows.push(vals.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect());
    }
    let r = rows.len();
    let mut d = vec![f64::NAN; r * width];
    for (i, row) in rows.iter().enumerate() {
        for (j, &x) in row.iter().enumerate() {
            if let Some(slot) = d.get_mut(j * r + i) {
                *slot = x;
            }
        }
    }
    Ok(Arr {
        r,
        c: if r == 0 { 0 } else { width },
        d,
    })
}

/// A number as text that reads back exactly.
fn fmt_num(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "Inf" } else { "-Inf" }.into()
    } else if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x:?}")
    }
}

/// Splits trailing `'Name', value` options off the arguments.
struct Options {
    delim: Delim,
    header_lines: Option<usize>,
}

fn options(args: &[Value], from: usize, name: &str) -> Result<Options> {
    let mut o = Options {
        delim: Delim::Auto,
        header_lines: None,
    };
    let mut k = from;
    while k < args.len() {
        let key = str_arg(args, k, name)?;
        let value = arg(args, k + 1, name)?;
        if key.eq_ignore_ascii_case("Delimiter") {
            o.delim = delim_from(value.as_str("the Delimiter")?)?;
        } else if key.eq_ignore_ascii_case("NumHeaderLines") {
            o.header_lines = Some(value.scalar_usize("NumHeaderLines")?);
        } else {
            return err(
                "Abaco:arity",
                format!("`{name}` does not know the option `{key}` (Delimiter, NumHeaderLines)"),
            );
        }
        k += 2;
    }
    Ok(o)
}

fn f_readmatrix(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "readmatrix")?;
    arity(a, 1, usize::MAX, "readmatrix")?;
    let path = str_arg(a, 0, "readmatrix")?;
    let o = options(a, 1, "readmatrix")?;
    let text = read_text(Path::new(path))?;
    one(Value::Num(parse_matrix(&text, o.delim, o.header_lines)?))
}

/// `csvread(file)` and `dlmread(file, delimiter)`.
fn f_dlmread(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "dlmread")?;
    arity(a, 1, 2, "dlmread")?;
    let path = str_arg(a, 0, "dlmread")?;
    let delim = match a.get(1) {
        Some(d) => delim_from(d.as_str("the delimiter")?)?,
        None => Delim::Auto,
    };
    let text = read_text(Path::new(path))?;
    one(Value::Num(parse_matrix(&text, delim, Some(0))?))
}

fn matrix_text(m: &Arr<f64>, delim: Delim) -> String {
    let sep = match delim {
        Delim::Char(c) => c,
        Delim::Space => ' ',
        Delim::Auto => ',',
    };
    let mut out = String::new();
    for i in 0..m.r {
        for j in 0..m.c {
            if j > 0 {
                out.push(sep);
            }
            out.push_str(&fmt_num(m.at(i, j).copied().unwrap_or(f64::NAN)));
        }
        out.push('\n');
    }
    out
}

fn f_writematrix(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "writematrix")?;
    arity(a, 2, usize::MAX, "writematrix")?;
    let m = real_arg(a, 0, "writematrix")?;
    let path = str_arg(a, 1, "writematrix")?;
    let o = options(a, 2, "writematrix")?;
    write_atomic(Path::new(path), matrix_text(&m, o.delim).as_bytes())?;
    Ok(vec![])
}

/// `csvwrite(file, A)`.
fn f_csvwrite(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "csvwrite")?;
    arity(a, 2, 2, "csvwrite")?;
    let path = str_arg(a, 0, "csvwrite")?;
    let m = real_arg(a, 1, "csvwrite")?;
    write_atomic(
        Path::new(path),
        matrix_text(&m, Delim::Char(',')).as_bytes(),
    )?;
    Ok(vec![])
}

/// `dlmwrite(file, A, delimiter)`.
fn f_dlmwrite(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "dlmwrite")?;
    arity(a, 2, 3, "dlmwrite")?;
    let path = str_arg(a, 0, "dlmwrite")?;
    let m = real_arg(a, 1, "dlmwrite")?;
    let delim = match a.get(2) {
        Some(d) => delim_from(d.as_str("the delimiter")?)?,
        None => Delim::Char(','),
    };
    write_atomic(Path::new(path), matrix_text(&m, delim).as_bytes())?;
    Ok(vec![])
}

fn f_fileread(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "fileread")?;
    arity(a, 1, 1, "fileread")?;
    one(Value::Str(read_text(Path::new(str_arg(
        a, 0, "fileread",
    )?))?))
}

fn f_isfile(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "isfile")?;
    let path = str_arg(a, 0, "isfile")?;
    one(Value::boolean(Path::new(path).is_file()))
}

// ---------- workspaces ----------

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

fn unescape(s: &str) -> Result<String> {
    let mut o = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => o.push('\\'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some('t') => o.push('\t'),
            other => {
                return err(
                    "Abaco:load",
                    format!("the file has a bad escape `\\{}`", other.unwrap_or(' ')),
                )
            }
        }
    }
    Ok(o)
}

fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn put_list<T>(out: &mut String, items: &[T], mut f: impl FnMut(&mut String, &T)) {
    for (k, x) in items.iter().enumerate() {
        if k > 0 {
            out.push(' ');
        }
        f(out, x);
    }
    out.push('\n');
}

fn put(v: &Value, out: &mut String, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return err("Abaco:save", "the value is nested too deeply to save");
    }
    match v {
        Value::Num(a) => {
            let _ = writeln!(out, "num {} {}", a.r, a.c);
            put_list(out, &a.d, |o, x| o.push_str(&format!("{x:?}")));
        }
        Value::Cplx(a) => {
            let _ = writeln!(out, "cplx {} {}", a.r, a.c);
            put_list(out, &a.d, |o, z| {
                let _ = write!(o, "{:?} {:?}", z.re, z.im);
            });
        }
        Value::Bool(a) => {
            let _ = writeln!(out, "bool {} {}", a.r, a.c);
            put_list(out, &a.d, |o, b| o.push(if *b { '1' } else { '0' }));
        }
        Value::Str(s) => {
            let _ = writeln!(out, "str {}", escape(s));
        }
        Value::Cell(a) => {
            let _ = writeln!(out, "cell {} {}", a.r, a.c);
            for item in &a.d {
                put(item, out, depth + 1)?;
            }
        }
        Value::Struct(s) => {
            let _ = writeln!(out, "struct {}", s.fields.len());
            for (name, item) in &s.fields {
                let _ = writeln!(out, "field {name}");
                put(item, out, depth + 1)?;
            }
        }
        Value::Fn(f) => match &**f {
            FnValue::Named(n) => {
                let _ = writeln!(out, "fn {n}");
            }
            FnValue::Anon { .. } => {
                return err(
                    "Abaco:save",
                    "an anonymous function holds the values it captured and cannot be saved",
                )
            }
        },
    }
    Ok(())
}

struct Reader<'a> {
    lines: std::str::Lines<'a>,
    line: usize,
}

impl<'a> Reader<'a> {
    fn next(&mut self) -> Result<&'a str> {
        self.line += 1;
        match self.lines.next() {
            Some(l) => Ok(l),
            None => err(
                "Abaco:load",
                format!("the file ends early (line {})", self.line),
            ),
        }
    }

    fn bad<T>(&self, what: &str) -> Result<T> {
        err("Abaco:load", format!("line {}: {what}", self.line))
    }

    fn dims(&self, rest: &str) -> Result<(usize, usize)> {
        let mut it = rest.split_whitespace();
        let (Some(r), Some(c), None) = (it.next(), it.next(), it.next()) else {
            return self.bad("expected two sizes");
        };
        let (Ok(r), Ok(c)) = (r.parse::<usize>(), c.parse::<usize>()) else {
            return self.bad("the sizes are not whole numbers");
        };
        if r.saturating_mul(c) > MAX_ELEMENTS {
            return self.bad("the array is larger than this program allows");
        }
        Ok((r, c))
    }

    fn list<T>(&mut self, n: usize, parse: impl Fn(&str) -> Option<T>) -> Result<Vec<T>> {
        let line = self.next()?;
        let mut out = Vec::with_capacity(n.min(1 << 20));
        for tok in line.split_whitespace() {
            match parse(tok) {
                Some(x) => out.push(x),
                None => return self.bad(&format!("`{tok}` is not a valid value")),
            }
        }
        if out.len() != n {
            return self.bad(&format!("expected {n} values, found {}", out.len()));
        }
        Ok(out)
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return self.bad("the value is nested too deeply");
        }
        let line = self.next()?;
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "num" => {
                let (r, c) = self.dims(rest)?;
                let d = self.list(r * c, |t| t.parse::<f64>().ok())?;
                Ok(Value::Num(Arr { r, c, d }))
            }
            "cplx" => {
                let (r, c) = self.dims(rest)?;
                let flat = self.list(r * c * 2, |t| t.parse::<f64>().ok())?;
                let d = flat
                    .chunks(2)
                    .map(|p| {
                        Complex64::new(
                            p.first().copied().unwrap_or(0.0),
                            p.get(1).copied().unwrap_or(0.0),
                        )
                    })
                    .collect();
                Ok(Value::Cplx(Arr { r, c, d }))
            }
            "bool" => {
                let (r, c) = self.dims(rest)?;
                let d = self.list(r * c, |t| match t {
                    "1" => Some(true),
                    "0" => Some(false),
                    _ => None,
                })?;
                Ok(Value::Bool(Arr { r, c, d }))
            }
            "str" => Ok(Value::Str(unescape(rest)?)),
            "cell" => {
                let (r, c) = self.dims(rest)?;
                let mut d = Vec::with_capacity((r * c).min(1 << 16));
                for _ in 0..r * c {
                    d.push(self.value(depth + 1)?);
                }
                Ok(Value::Cell(Arr { r, c, d }))
            }
            "struct" => {
                let Ok(n) = rest.trim().parse::<usize>() else {
                    return self.bad("expected a field count");
                };
                if n > MAX_VARS {
                    return self.bad("too many fields");
                }
                let mut s = StructV::default();
                for _ in 0..n {
                    let f = self.next()?;
                    let Some(name) = f.strip_prefix("field ") else {
                        return self.bad("expected `field <name>`");
                    };
                    if !valid_name(name) {
                        return self.bad(&format!("`{name}` is not a field name"));
                    }
                    let v = self.value(depth + 1)?;
                    s.set(name, v);
                }
                Ok(Value::Struct(Box::new(s)))
            }
            "fn" => {
                if !valid_name(rest) {
                    return self.bad(&format!("`{rest}` is not a function name"));
                }
                Ok(Value::Fn(Arc::new(FnValue::Named(rest.to_string()))))
            }
            other => self.bad(&format!("unknown kind `{other}`")),
        }
    }
}

/// The variables of a saved workspace.
fn parse_workspace(text: &str) -> Result<Vec<(String, Value)>> {
    let mut r = Reader {
        lines: text.lines(),
        line: 0,
    };
    if r.next()?.trim_end() != MAGIC {
        return err("Abaco:load", "this is not an Ábaco data file");
    }
    let mut vars = Vec::new();
    loop {
        r.line += 1;
        let Some(line) = r.lines.next() else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(name) = line.strip_prefix("var ") else {
            return r.bad("expected `var <name>`");
        };
        if !valid_name(name) {
            return r.bad(&format!("`{name}` is not a variable name"));
        }
        if vars.len() >= MAX_VARS {
            return r.bad("too many variables");
        }
        let v = r.value(0)?;
        vars.push((name.to_string(), v));
    }
    Ok(vars)
}

fn with_ext(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.extension().is_some() {
        p.to_path_buf()
    } else {
        p.with_extension(EXT)
    }
}

/// `save(file, 'a', 'b')` writes the named variables, or all of them, to a text file.
fn f_save(i: &mut Interp, a: &[Value], _: usize) -> Out {
    need_files(i, "save")?;
    arity(a, 1, usize::MAX, "save")?;
    let path = with_ext(str_arg(a, 0, "save")?);
    let mut chosen: Vec<(String, Value)> = Vec::new();
    if a.len() == 1 {
        chosen = i.variables();
    } else {
        for k in 1..a.len() {
            let name = str_arg(a, k, "save")?;
            match i.get(name) {
                Some(v) => chosen.push((name.to_string(), v.clone())),
                None => {
                    return err(
                        "Abaco:value",
                        format!("`save`: the variable `{name}` does not exist"),
                    )
                }
            }
        }
    }
    let mut text = String::from(MAGIC);
    text.push('\n');
    let mut skipped = Vec::new();
    for (name, v) in &chosen {
        let mut block = String::new();
        let _ = writeln!(block, "var {name}");
        match put(v, &mut block, 0) {
            Ok(()) => text.push_str(&block),
            Err(e) => skipped.push(format!("`{name}` ({})", e.message)),
        }
    }
    write_atomic(&path, text.as_bytes())?;
    for s in skipped {
        i.warn(format!("save: skipped {s}"));
    }
    Ok(vec![])
}

/// A variable name for a data file that is not a saved workspace.
fn name_from_file(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        name.insert(0, 'X');
    }
    name
}

/// `load(file)` restores a saved workspace, or reads a numeric text file into a variable
/// named after the file. With an output it returns a struct (workspace) or the matrix.
fn f_load(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    need_files(i, "load")?;
    arity(a, 1, usize::MAX, "load")?;
    let given = str_arg(a, 0, "load")?;
    let mut path = PathBuf::from(given);
    if !path.exists() && path.extension().is_none() {
        path.set_extension(EXT);
    }
    let text = read_text(&path)?;
    if text.starts_with("ABACO-DATA") {
        let mut vars = parse_workspace(&text)?;
        if a.len() > 1 {
            let mut wanted = Vec::new();
            for k in 1..a.len() {
                wanted.push(str_arg(a, k, "load")?.to_string());
            }
            for w in &wanted {
                if !vars.iter().any(|(n, _)| n == w) {
                    return err(
                        "Abaco:value",
                        format!("`load`: `{given}` has no variable `{w}`"),
                    );
                }
            }
            vars.retain(|(n, _)| wanted.contains(n));
        }
        if nargout >= 1 {
            let mut s = StructV::default();
            for (n, v) in vars {
                s.set(&n, v);
            }
            return one(Value::Struct(Box::new(s)));
        }
        for (n, v) in vars {
            i.set(&n, v);
        }
        return Ok(vec![]);
    }
    if a.len() > 1 && !is_flag(a, 1, "-ascii") {
        return err(
            "Abaco:load",
            format!("`{given}` is a text file of numbers; it has no variables to choose from"),
        );
    }
    let m = parse_matrix(&text, Delim::Auto, None)?;
    if nargout >= 1 {
        return one(Value::Num(m));
    }
    i.set(&name_from_file(&path), Value::Num(m));
    Ok(vec![])
}

pub(super) const TABLE: &[(&str, Bf)] = &[
    ("csvread", f_dlmread),
    ("csvwrite", f_csvwrite),
    ("dlmread", f_dlmread),
    ("dlmwrite", f_dlmwrite),
    ("fileread", f_fileread),
    ("isfile", f_isfile),
    ("load", f_load),
    ("readmatrix", f_readmatrix),
    ("save", f_save),
    ("writematrix", f_writematrix),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_back_exactly() {
        for x in [0.1, -2.5, 1e-300, 1e300, 12345.0, 1.0 / 3.0, 2.0f64.sqrt()] {
            assert_eq!(fmt_num(x).parse::<f64>().ok(), Some(x), "{x}");
        }
        assert_eq!(fmt_num(f64::NAN), "NaN");
        assert_eq!(fmt_num(f64::NEG_INFINITY), "-Inf");
    }

    #[test]
    fn headers_and_ragged_rows() {
        let m = parse_matrix("a,b,c\n1,2,3\n4,,6\n7,8\n", Delim::Auto, None).unwrap();
        assert_eq!((m.r, m.c), (3, 3));
        assert_eq!(m.at(0, 0), Some(&1.0));
        assert!(m.at(1, 1).is_some_and(|x| x.is_nan()));
        assert!(m.at(2, 2).is_some_and(|x| x.is_nan()));
        let m = parse_matrix("1 2\n3 4\n", Delim::Auto, None).unwrap();
        assert_eq!(m.d, vec![1.0, 3.0, 2.0, 4.0]);
        let m = parse_matrix("x\n1\n", Delim::Auto, Some(0)).unwrap();
        assert_eq!((m.r, m.c), (2, 1));
    }

    #[test]
    fn escapes_round_trip() {
        let s = "a\\b\nc\td\re";
        assert_eq!(unescape(&escape(s)).unwrap(), s);
        assert!(unescape("bad\\q").is_err());
    }

    #[test]
    fn hostile_workspaces_are_errors() {
        for bad in [
            "",
            "ABACO-DATA 1\nvar x\n",
            "ABACO-DATA 1\nvar x\nnum 2 2\n1 2 3\n",
            "ABACO-DATA 1\nvar x\nnum 99999999 99999999\n\n",
            "ABACO-DATA 1\nvar 1x\nnum 0 0\n\n",
            "ABACO-DATA 1\nvar x\nweird 1\n",
            "ABACO-DATA 1\nvar x\nstruct 1\nfield a b\nnum 0 0\n\n",
        ] {
            assert!(parse_workspace(bad).is_err(), "{bad:?}");
        }
        let mut deep = String::from("ABACO-DATA 1\nvar x\n");
        for _ in 0..200 {
            deep.push_str("cell 1 1\n");
        }
        deep.push_str("num 0 0\n\n");
        assert!(parse_workspace(&deep).is_err());
    }
}
