// SPDX-License-Identifier: MIT OR Apache-2.0
//! What each command does.

use crate::args::{Command, ConvertArgs, Options, ScriptSource, SimArgs};
use crate::{err, out};
use abaco_blocks::{Model, SimResult};
use abaco_doc::{error_json, CellError, CellKind, Document, DEFAULT_SIGNIFICANT_DIGITS};
use abaco_expr::{eval_str, Env};
use abaco_script::{Event, Interp, Limits};
use nodalia_units::{registry, Quantity, Unit};
use serde_json::{json, Value as Json};
use std::process::ExitCode;

pub fn run(cmd: &Command) -> ExitCode {
    match cmd {
        Command::Eval(expr, o) => eval(expr, *o),
        Command::Run(file, o) => run_file(file, *o),
        Command::Script(src, o) => script(src, *o),
        Command::Sim(a, o) => sim(a, *o),
        Command::Convert(a, o) => convert(a, *o),
        Command::Repl => repl(),
        Command::Functions(o) => functions(*o),
        Command::UnitsList(o) => units_list(*o),
        Command::UnitsConvert(q, u, o) => units_convert(q, u, *o),
    }
}

fn pretty(v: &Json) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".to_string())
}

fn eval(expr: &str, o: Options) -> ExitCode {
    let digits = o.digits.unwrap_or(DEFAULT_SIGNIFICANT_DIGITS);
    let mut env = Env::new();
    match eval_str(expr, &mut env) {
        Ok(ev) => {
            if o.json {
                out(&pretty(&json!({
                    "ok": true,
                    "input": expr,
                    "value": ev.value.as_ref().map_or(Json::Null, |v| v.to_json(digits)),
                    "text": ev.value.as_ref().map(|v| v.display(digits)),
                    "defined": ev.defined,
                    "warnings": ev.warnings,
                })));
            } else {
                if let Some(v) = &ev.value {
                    out(&v.display(digits));
                }
                for w in &ev.warnings {
                    err(&format!("warning: {w}"));
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if o.json {
                out(&pretty(&json!({
                    "ok": false,
                    "input": expr,
                    "error": error_json(&CellError::Expr(e), expr),
                })));
            } else {
                err(&e.render(expr));
            }
            ExitCode::from(1)
        }
    }
}

fn first_line(s: &str) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut shown: String = line.chars().take(60).collect();
    if line.chars().count() > 60 || s.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
        shown.push_str(" ...");
    }
    shown
}

fn run_file(file: &str, o: Options) -> ExitCode {
    let mut doc = match Document::open(file) {
        Ok(d) => d,
        Err(e) => {
            err(&format!("error: {e}"));
            return ExitCode::from(2);
        }
    };
    if let Some(d) = o.digits {
        let s = doc.settings();
        if let Err(e) = doc.set_settings(abaco_doc::Settings {
            significant_digits: d,
            ..s
        }) {
            err(&format!("error: {e}"));
            return ExitCode::from(2);
        }
    }
    doc.evaluate();
    let errors = doc.error_count();
    if o.json {
        out(&pretty(&doc.report()));
    } else {
        for (i, cell) in doc.cells().iter().enumerate() {
            let n = i + 1;
            let CellKind::Math { source } = &cell.kind else {
                continue;
            };
            let Some(result) = doc.result(cell.id) else {
                continue;
            };
            match &result.outcome {
                Ok(o) => match &o.value {
                    Some(v) => out(&format!(
                        "[{n}] {} = {}",
                        first_line(source),
                        doc.display(v)
                    )),
                    None if o.defined.is_empty() => {}
                    None => out(&format!(
                        "[{n}] {} (defined {})",
                        first_line(source),
                        o.defined.join(", ")
                    )),
                },
                Err(e) => {
                    out(&format!("[{n}] {}", first_line(source)));
                    out(&format!("    error ({}): {}", e.code(), e.message()));
                    if let CellError::Expr(x) = e {
                        if let Some((l, c)) = x.line_col(source) {
                            out(&format!("    at cell {n}, line {l}, column {c}"));
                        }
                    }
                }
            }
            for w in &result.warnings {
                out(&format!("    warning: {w}"));
            }
        }
        if errors > 0 {
            err(&format!(
                "{errors} cell{} had an error",
                if errors == 1 { "" } else { "s" }
            ));
        }
    }
    if errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn units_list(o: Options) -> ExitCode {
    if o.json {
        let items: Vec<Json> = registry::UNITS
            .iter()
            .map(|u| {
                json!({
                    "symbol": u.symbol,
                    "name": u.name,
                    "aliases": u.aliases,
                    "dimension": u.dim.describe(),
                    "si_factor": u.factor,
                    "offset": u.zero,
                    "prefixes": format!("{:?}", u.prefix).to_lowercase(),
                })
            })
            .collect();
        out(&pretty(&json!({ "units": items })));
    } else {
        let width = registry::UNITS
            .iter()
            .map(|u| u.symbol.chars().count())
            .max()
            .unwrap_or(4);
        let name_width = registry::UNITS
            .iter()
            .map(|u| u.name.chars().count())
            .max()
            .unwrap_or(4);
        for u in registry::UNITS {
            out(&format!(
                "{:<w$}  {:<nw$}  {}",
                u.symbol,
                u.name,
                u.dim.describe(),
                w = width,
                nw = name_width
            ));
        }
    }
    ExitCode::SUCCESS
}

fn units_convert(qty: &str, unit: &str, o: Options) -> ExitCode {
    let digits = o.digits.unwrap_or(DEFAULT_SIGNIFICANT_DIGITS);
    let result = Quantity::parse(qty).and_then(|q| Unit::parse(unit).and_then(|u| q.to(&u)));
    match result {
        Ok(q) => {
            if o.json {
                out(&pretty(&json!({
                    "ok": true,
                    "input": qty,
                    "unit": unit,
                    "value": q.display_value(),
                    "si": q.si_value(),
                    "text": q.format(digits),
                })));
            } else {
                out(&q.format(digits));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if o.json {
                out(&pretty(&json!({
                    "ok": false,
                    "input": qty,
                    "unit": unit,
                    "error": { "message": e.to_string() },
                })));
            } else {
                err(&format!("error: {e}"));
            }
            ExitCode::from(1)
        }
    }
}

/// Scripts run on a thread with a large stack so deep recursion ends in an error, never a
/// crash.
const SCRIPT_STACK: usize = 256 * 1024 * 1024;

fn script_limits() -> Limits {
    Limits {
        max_stack: SCRIPT_STACK / 4 * 3,
        files: true,
        ..Limits::default()
    }
}

/// Prints what a run produced: text to standard output, warnings to standard error.
fn flush_events(interp: &mut Interp) {
    for e in interp.take_events() {
        match e {
            Event::Text(t) => {
                let mut stdout = std::io::stdout().lock();
                let _ = std::io::Write::write_all(&mut stdout, t.as_bytes());
            }
            Event::Warning(w) => err(&format!("warning: {w}")),
            Event::Figure(_) | Event::ClearScreen => {}
        }
    }
}

fn read_script(src: &ScriptSource) -> Result<String, String> {
    match src {
        ScriptSource::Code(code) => Ok(code.clone()),
        ScriptSource::File(name) if name == "-" => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| format!("cannot read standard input: {e}"))?;
            Ok(text)
        }
        ScriptSource::File(name) => {
            std::fs::read_to_string(name).map_err(|e| format!("cannot read `{name}`: {e}"))
        }
    }
}

fn script(src: &ScriptSource, _o: Options) -> ExitCode {
    let text = match read_script(src) {
        Ok(t) => t,
        Err(e) => {
            err(&format!("error: {e}"));
            return ExitCode::from(2);
        }
    };
    let worker = std::thread::Builder::new()
        .stack_size(SCRIPT_STACK)
        .spawn(move || {
            let mut interp = Interp::with_limits(script_limits());
            let result = interp.run(&text);
            flush_events(&mut interp);
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    err(&e.render(&text));
                    ExitCode::from(1)
                }
            }
        });
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        _ => {
            err("error: the script could not be run");
            ExitCode::from(2)
        }
    }
}

/// `true` when the parse error only says the input stopped too early, so more lines may
/// complete the statement.
fn incomplete(message: &str) -> bool {
    message.contains("end of the input") || message.contains("never closed")
}

fn repl() -> ExitCode {
    let worker = std::thread::Builder::new()
        .stack_size(SCRIPT_STACK)
        .spawn(|| {
            let mut interp = Interp::with_limits(script_limits());
            let mut pending = String::new();
            for line in std::io::stdin().lines() {
                let Ok(line) = line else { break };
                pending.push_str(&line);
                pending.push('\n');
                match abaco_script::parser::parse(&pending) {
                    Err(e) if incomplete(&e.message) => continue,
                    Err(e) => {
                        err(&e.render(&pending));
                        pending.clear();
                        continue;
                    }
                    Ok(_) => {}
                }
                if let Err(e) = interp.run(&pending) {
                    flush_events(&mut interp);
                    err(&e.render(&pending));
                } else {
                    flush_events(&mut interp);
                }
                pending.clear();
            }
            ExitCode::SUCCESS
        });
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        _ => ExitCode::from(2),
    }
}

fn functions(o: Options) -> ExitCode {
    let names = abaco_script::builtins::names();
    if o.json {
        out(&pretty(&json!(names)));
    } else {
        out(&names.join("\n"));
    }
    ExitCode::SUCCESS
}

/// Formats a number with the requested significant digits.
fn sig(v: f64, digits: usize) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v == 0.0 {
        return "0".to_string();
    }
    let decimals = digits.saturating_sub(1);
    let text = format!("{v:.decimals$e}");
    // Plain notation when the exponent is small, scientific otherwise.
    match text
        .split_once('e')
        .and_then(|(m, e)| e.parse::<i32>().ok().map(|e| (m, e)))
    {
        Some((_, e)) if (-4..=6).contains(&e) => {
            let places = (digits as i32 - 1 - e).max(0) as usize;
            let plain = format!("{v:.places$}");
            if plain.contains('.') {
                plain
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            } else {
                plain
            }
        }
        _ => text,
    }
}

fn csv_text(r: &SimResult) -> String {
    let mut head = vec!["time".to_string()];
    for s in &r.signals {
        if s.width <= 1 {
            head.push(s.name.clone());
        } else {
            head.extend((1..=s.width).map(|k| format!("{}[{k}]", s.name)));
        }
    }
    let quote = |h: &str| {
        if h.contains([',', '"', '\n']) {
            format!("\"{}\"", h.replace('"', "\"\""))
        } else {
            h.to_string()
        }
    };
    let mut text = head.iter().map(|h| quote(h)).collect::<Vec<_>>().join(",");
    text.push('\n');
    for (i, t) in r.time.iter().enumerate() {
        let mut row = vec![format!("{t:e}")];
        for s in &r.signals {
            row.extend(s.row(i).iter().map(|v| format!("{v:e}")));
        }
        text.push_str(&row.join(","));
        text.push('\n');
    }
    text
}

fn extension(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read `{path}`: {e}"))
}

/// A block diagram from a JSON, `.mdl` or `.slx` file, with what had to be approximated.
fn load_model(
    path: &str,
    vars: Option<&str>,
    lenient: bool,
) -> Result<(Model, Vec<String>), String> {
    let bytes = read_file(path)?;
    let is_json =
        extension(path) == "json" || bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
    if is_json {
        let text = String::from_utf8(bytes).map_err(|_| format!("`{path}` is not UTF-8 text"))?;
        return Model::from_json(&text)
            .map(|m| (m, Vec::new()))
            .map_err(|e| format!("{path}: {e}"));
    }
    let ws = match vars {
        Some(v) => {
            let file = abaco_formats::mat::read(&read_file(v)?)
                .map_err(|e| format!("cannot read `{v}`: {e}"))?;
            abaco_blocks::simulink::workspace_from_mat(&file.vars)
        }
        None => abaco_blocks::expr::Workspace::new(),
    };
    abaco_blocks::simulink::read(&bytes, &ws, !lenient)
        .map(|i| (i.model, i.warnings))
        .map_err(|e| format!("{path}: {e}"))
}

fn convert(a: &ConvertArgs, o: Options) -> ExitCode {
    let fail = |msg: String| {
        if o.json {
            out(&pretty(&json!({ "ok": false, "error": msg })));
        } else {
            err(&format!("error: {msg}"));
        }
        ExitCode::from(2)
    };
    let (from, to) = (extension(&a.input), extension(&a.output));
    let (bytes, warnings, what): (Vec<u8>, Vec<String>, String) = match (from.as_str(), to.as_str())
    {
        ("mat", "mat") => {
            let file = match read_file(&a.input)
                .and_then(|b| abaco_formats::mat::read(&b).map_err(|e| format!("{}: {e}", a.input)))
            {
                Ok(f) => f,
                Err(e) => return fail(e),
            };
            let version = match a.mat_version.as_deref() {
                Some("4") => abaco_formats::mat::MatVersion::V4,
                Some("6") => abaco_formats::mat::MatVersion::V6,
                Some("7") => abaco_formats::mat::MatVersion::V7,
                Some("7.3") => abaco_formats::mat::MatVersion::V73,
                _ => file.version,
            };
            let n = file.vars.len();
            let out_file = abaco_formats::mat::MatFile { version, ..file };
            match abaco_formats::mat::write(&out_file) {
                Ok(b) => (b, Vec::new(), format!("{n} variables")),
                Err(e) => return fail(format!("{}: {e}", a.output)),
            }
        }
        ("json" | "mdl" | "slx", "json" | "mdl" | "slx") => {
            let (model, warnings) = match load_model(&a.input, a.vars.as_deref(), a.lenient) {
                Ok(r) => r,
                Err(e) => return fail(e),
            };
            let n = model.blocks.len();
            let written: Result<Vec<u8>, String> = match to.as_str() {
                "json" => model.to_json().map(String::into_bytes).map_err(|e| e.to_string()),
                "mdl" => abaco_blocks::simulink::write_mdl(&model)
                    .map(String::into_bytes)
                    .map_err(|e| e.to_string()),
                _ => abaco_blocks::simulink::write_slx(&model).map_err(|e| e.to_string()),
            };
            match written {
                Ok(b) => (b, warnings, format!("{n} blocks at the top level")),
                Err(e) => return fail(format!("{}: {e}", a.output)),
            }
        }
        (f, t) => {
            return fail(format!(
                "cannot convert `.{f}` to `.{t}`; models (.json, .mdl, .slx) convert among themselves and .mat to .mat"
            ))
        }
    };
    if let Err(e) = std::fs::write(&a.output, &bytes) {
        return fail(format!("cannot write `{}`: {e}", a.output));
    }
    if o.json {
        out(&pretty(&json!({
            "ok": true,
            "output": a.output,
            "bytes": bytes.len(),
            "warnings": warnings,
        })));
    } else {
        for w in &warnings {
            err(&format!("warning: {w}"));
        }
        out(&format!(
            "wrote {} ({what}, {} bytes)",
            a.output,
            bytes.len()
        ));
    }
    ExitCode::SUCCESS
}

fn sim(a: &SimArgs, o: Options) -> ExitCode {
    let (mut model, warnings) = match load_model(&a.file, a.vars.as_deref(), a.lenient) {
        Ok(r) => r,
        Err(e) => {
            err(&format!("error: {e}"));
            return ExitCode::from(2);
        }
    };
    if !o.json {
        for w in &warnings {
            err(&format!("warning: {w}"));
        }
    }
    if let Some(stop) = a.stop {
        model.solver.stop_time = stop;
    }
    let r = match abaco_blocks::simulate(&model) {
        Ok(r) => r,
        Err(e) => {
            if o.json {
                out(&pretty(&json!({ "ok": false, "error": e.to_string() })));
            } else {
                err(&format!("error: {e}"));
            }
            return ExitCode::from(1);
        }
    };
    if let Some(path) = &a.csv {
        if let Err(e) = std::fs::write(path, csv_text(&r)) {
            err(&format!("error: cannot write `{path}`: {e}"));
            return ExitCode::from(2);
        }
    }
    if o.json {
        let signals: Vec<Json> = r
            .signals
            .iter()
            .map(|s| json!({ "name": s.name, "width": s.width, "data": s.data }))
            .collect();
        out(&pretty(&json!({
            "ok": true,
            "warnings": warnings,
            "model": (if model.name.is_empty() { "model" } else { model.name.as_str() }),
            "time": r.time,
            "signals": signals,
            "stopped_by": r.stopped_by,
            "stats": {
                "steps": r.stats.steps,
                "rejected": r.stats.rejected,
                "zero_crossings": r.stats.zero_crossings,
                "evaluations": r.stats.evaluations,
            },
            "method": format!("{:?}", model.solver.method),
        })));
        return ExitCode::SUCCESS;
    }
    let digits = o.digits.unwrap_or(DEFAULT_SIGNIFICANT_DIGITS);
    let t_end = r.time.last().copied().unwrap_or(0.0);
    out(&format!(
        "{}: {:?}, t = 0 … {} ({} samples)",
        (if model.name.is_empty() {
            "model"
        } else {
            model.name.as_str()
        }),
        model.solver.method,
        sig(t_end, digits),
        r.time.len()
    ));
    if let Some(b) = &r.stopped_by {
        out(&format!("stopped by {b}"));
    }
    for s in &r.signals {
        let last = r.time.len().checked_sub(1).map(|i| s.row(i)).unwrap_or(&[]);
        let shown: Vec<String> = last.iter().take(8).map(|v| sig(*v, digits)).collect();
        let more = if last.len() > 8 { " …" } else { "" };
        out(&format!("  {} = [{}]{more}", s.name, shown.join(", ")));
    }
    out(&format!(
        "steps {}, rejected {}, zero crossings {}, evaluations {}",
        r.stats.steps, r.stats.rejected, r.stats.zero_crossings, r.stats.evaluations
    ));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod sim_tests {
    use super::*;
    use abaco_blocks::{BlockKind, Model};

    #[test]
    fn significant_digits_switch_to_scientific_outside_a_readable_range() {
        assert_eq!(sig(0.0, 6), "0");
        assert_eq!(sig(98.1, 6), "98.1");
        assert_eq!(sig(1.0 / 3.0, 4), "0.3333");
        assert_eq!(sig(-2.5, 6), "-2.5");
        assert_eq!(sig(1.0e8, 3), "1.00e8");
        assert_eq!(sig(1.5e-7, 3), "1.50e-7");
        assert_eq!(sig(f64::INFINITY, 6), "inf");
    }

    #[test]
    fn csv_has_a_header_with_vector_elements_and_one_row_per_sample() {
        let mut m = Model::new("csv");
        m.solver.stop_time = 1.0;
        m.add(
            "v",
            BlockKind::Constant {
                value: vec![1.0, 2.0],
            },
        )
        .unwrap();
        m.add("Out, 1", BlockKind::Scope { inputs: 1 }).unwrap();
        m.connect(("v", 0), ("Out, 1", 0));
        let r = abaco_blocks::simulate(&m).unwrap();
        let text = csv_text(&r);
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("time,\"Out, 1[1]\",\"Out, 1[2]\""));
        assert_eq!(lines.count(), r.time.len());
    }
}
