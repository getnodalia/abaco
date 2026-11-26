// SPDX-License-Identifier: MIT OR Apache-2.0
//! What each command does.

use crate::args::{Command, Options};
use crate::{err, out};
use abaco_doc::{error_json, CellError, CellKind, Document, DEFAULT_SIGNIFICANT_DIGITS};
use abaco_expr::{eval_str, Env};
use nodalia_units::{registry, Quantity, Unit};
use serde_json::{json, Value as Json};
use std::process::ExitCode;

pub fn run(cmd: &Command) -> ExitCode {
    match cmd {
        Command::Eval(expr, o) => eval(expr, *o),
        Command::Run(file, o) => run_file(file, *o),
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
