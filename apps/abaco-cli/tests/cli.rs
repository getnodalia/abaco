// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Runs the real `abaco` binary.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn abaco(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_abaco"))
        .args(args)
        .output()
        .expect("the abaco binary runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

fn temp_file(name: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("abaco-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

fn doc_file(name: &str, cells: &[(&str, &str)]) -> PathBuf {
    let cells: Vec<Value> = cells
        .iter()
        .enumerate()
        .map(|(i, (kind, text))| match *kind {
            "math" => serde_json::json!({"type": "math", "id": i + 1, "source": text}),
            _ => serde_json::json!({"type": "text", "id": i + 1, "markdown": text}),
        })
        .collect();
    let v = serde_json::json!({"format": "abaco", "format_version": 1, "cells": cells});
    temp_file(name, &v.to_string())
}

#[test]
fn eval_prints_the_value() {
    let o = abaco(&["eval", "10 kg * 9.81 m/s^2 -> N"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o), "98.1 N\n");
    assert_eq!(stderr(&o), "");
    assert_eq!(stdout(&abaco(&["eval", "sin(30 deg)"])), "0.5\n");
    assert_eq!(stdout(&abaco(&["eval", "det([1,2;3,4])"])), "-2\n");
    assert_eq!(stdout(&abaco(&["eval", "(3+4i)*(1-2i)"])), "11 - 2i\n");
    assert_eq!(stdout(&abaco(&["eval", "x = 3 m; 2x"])), "6 m\n");
    assert_eq!(
        stdout(&abaco(&["eval", "f(x) = x + 1"])),
        "",
        "a definition has no value"
    );
    assert_eq!(stdout(&abaco(&["eval", "--", "-3 + 2"])), "-1\n");
    assert_eq!(stdout(&abaco(&["eval", "-3 + 2"])), "-1\n");
}

#[test]
fn eval_digits() {
    assert_eq!(stdout(&abaco(&["eval", "1/3"])), "0.333333\n");
    assert_eq!(
        stdout(&abaco(&["eval", "1/3", "--digits", "10"])),
        "0.3333333333\n"
    );
    assert_eq!(stdout(&abaco(&["--digits=3", "eval", "pi"])), "3.14\n");
    assert_eq!(code(&abaco(&["eval", "1", "--digits", "0"])), 2);
    assert_eq!(code(&abaco(&["eval", "1", "--digits", "x"])), 2);
}

#[test]
fn eval_errors_show_position_and_exit_1() {
    let o = abaco(&["eval", "1 + 2 +* 3"]);
    assert_eq!(code(&o), 1);
    assert_eq!(stdout(&o), "");
    let e = stderr(&o);
    assert!(e.contains("1:8"), "{e}");
    assert!(e.contains("1 + 2 +* 3") && e.contains('^'), "{e}");
    let o = abaco(&["eval", "5 m + 3 s"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("length") && stderr(&o).contains("time"));
    let o = abaco(&["eval", "sinn(1)"]);
    assert!(stderr(&o).contains("did you mean"), "{}", stderr(&o));
}

#[test]
fn eval_json() {
    let o = abaco(&["eval", "9.81 m/s^2", "--json"]);
    assert_eq!(code(&o), 0);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["text"], "9.81 m/s^2");
    assert_eq!(v["value"]["type"], "quantity");
    assert_eq!(v["value"]["unit"], "m/s^2");
    assert_eq!(v["value"]["value"], 9.81);

    let o = abaco(&["eval", "1 +\n2 +* 3", "--json"]);
    assert_eq!(code(&o), 1);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "syntax");
    assert_eq!(v["error"]["line"], 2);
    assert_eq!(v["error"]["column"], 4);
    assert_eq!(stderr(&o), "");

    let v: Value = serde_json::from_str(&stdout(&abaco(&["eval", "[1,2;3,4]", "--json"]))).unwrap();
    assert_eq!(v["value"]["type"], "matrix");
    assert_eq!(v["value"]["items"][1][0]["value"], 3.0);
}

#[test]
fn eval_warns_on_ill_conditioned_systems() {
    let o = abaco(&["eval", "solve([1,1;1,1.00000000001],[1;2])"]);
    assert_eq!(code(&o), 0);
    assert!(
        stderr(&o).contains("warning") && stderr(&o).contains("ill-conditioned"),
        "{}",
        stderr(&o)
    );
    let v: Value = serde_json::from_str(&stdout(&abaco(&[
        "eval",
        "solve([1,1;1,1.00000000001],[1;2])",
        "--json",
    ])))
    .unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn hostile_expressions_fail_cleanly() {
    let huge = "1+".repeat(60_000) + "1";
    let o = abaco(&["eval", &huge]);
    assert_eq!(code(&o), 1);
    assert!(
        stderr(&o).contains("limit"),
        "{}",
        &stderr(&o)[..200.min(stderr(&o).len())]
    );
    let o = abaco(&["eval", &"(".repeat(10_000)]);
    assert_eq!(code(&o), 1);
    let o = abaco(&["eval", "zeros(1e9)"]);
    assert_eq!(code(&o), 1);
    let o = abaco(&["eval", "10^10^10"]);
    assert_eq!(code(&o), 1);
    let o = abaco(&["eval", "日本語 + 1"]);
    assert_eq!(code(&o), 1);
    // no panic message ever reaches the user
    for src in [
        "1/0",
        "ln(0)",
        "[1,2]*[3,4]",
        "factorial(1e6)",
        "\"a\" - 1",
        "(((",
    ] {
        let o = abaco(&["eval", src]);
        assert!(!stderr(&o).contains("panicked"), "{src}: {}", stderr(&o));
        assert_eq!(code(&o), 1, "{src}");
    }
}

#[test]
fn run_prints_every_cell() {
    let f = doc_file(
        "ok.abaco",
        &[
            ("text", "# Beam"),
            ("math", "q = 3 kN/m"),
            ("math", "M = q * L^2 / 8 -> kN*m"),
            ("math", "L = 2.5 m"),
        ],
    );
    let o = abaco(&["run", f.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("[2] q = 3 kN/m = 3 kN/m"), "{out}");
    assert!(
        out.contains("[3] M = q * L^2 / 8 -> kN*m = 2.34375 kN*m"),
        "{out}"
    );
    assert!(out.contains("[4] L = 2.5 m = 2.5 m"), "{out}");
    assert!(!out.contains("Beam"), "text cells are not listed");
    // deterministic
    assert_eq!(stdout(&abaco(&["run", f.to_str().unwrap()])), out);
    // digits
    let o = abaco(&["run", f.to_str().unwrap(), "--digits", "3"]);
    assert!(stdout(&o).contains("2.34 kN*m"), "{}", stdout(&o));
}

#[test]
fn run_reports_cell_errors_with_locations_and_exits_1() {
    let f = doc_file(
        "bad.abaco",
        &[
            ("math", "a = 1"),
            ("math", "b = a +\n  * 2"),
            ("math", "c = 5 m + 3 s"),
            ("math", "d = c * 2"),
            ("math", "e1 = e2"),
            ("math", "e2 = e1"),
            ("math", "ok = a + 1"),
        ],
    );
    let o = abaco(&["run", f.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    let out = stdout(&o);
    assert!(out.contains("[7] ok = a + 1 = 2"), "{out}");
    assert!(
        out.contains("error (syntax)") && out.contains("at cell 2, line 2, column 3"),
        "{out}"
    );
    assert!(
        out.contains("error (dimension)") && out.contains("at cell 3, line 1, column 5"),
        "{out}"
    );
    assert!(out.contains("error (dependency)"), "{out}");
    assert!(
        out.contains("circular reference: `e1` -> `e2` -> `e1`"),
        "{out}"
    );
    assert!(
        stderr(&o).contains("5 cells had an error"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn run_json_report() {
    let f = doc_file(
        "json.abaco",
        &[
            ("text", "hi"),
            ("math", "x = 2 m"),
            ("math", "y = x * 3 s"),
            ("math", "z = 1/0"),
        ],
    );
    let o = abaco(&["run", f.to_str().unwrap(), "--json"]);
    assert_eq!(code(&o), 1);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["errors"], 1);
    let cells = v["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 4);
    assert_eq!(cells[0]["status"], "text");
    assert_eq!(cells[1]["status"], "ok");
    assert_eq!(cells[1]["value"]["text"], "2 m");
    assert_eq!(cells[2]["value"]["text"], "6 m*s");
    assert_eq!(cells[3]["status"], "error");
    assert_eq!(cells[3]["error"]["code"], "domain");
    assert_eq!(cells[3]["error"]["line"], 1);
    assert_eq!(cells[3]["error"]["column"], 5);
}

#[test]
fn run_succeeds_with_exit_0_and_json_when_clean() {
    let f = doc_file("clean.abaco", &[("math", "1 + 1")]);
    let o = abaco(&["run", f.to_str().unwrap(), "--json"]);
    assert_eq!(code(&o), 0);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["errors"], 0);
}

#[test]
fn run_rejects_unusable_files_with_exit_2() {
    let o = abaco(&["run", "/definitely/not/here.abaco"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("not/here.abaco"), "{}", stderr(&o));

    let f = temp_file("garbage.abaco", "this is not json");
    let o = abaco(&["run", f.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("line 1"), "{}", stderr(&o));

    let f = temp_file(
        "newer.abaco",
        r#"{"format":"abaco","format_version":7,"cells":[]}"#,
    );
    let o = abaco(&["run", f.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("version 7"), "{}", stderr(&o));

    let f = temp_file(
        "other.abaco",
        r#"{"format":"something","format_version":1,"cells":[]}"#,
    );
    assert_eq!(code(&abaco(&["run", f.to_str().unwrap()])), 2);

    let f = temp_file("binary.abaco", "\u{0}");
    std::fs::write(&f, [0xffu8, 0xfe, 0x00]).unwrap();
    assert_eq!(code(&abaco(&["run", f.to_str().unwrap()])), 2);

    let dir = std::env::temp_dir();
    assert_eq!(code(&abaco(&["run", dir.to_str().unwrap()])), 2);
}

#[test]
fn units_convert() {
    let o = abaco(&["units", "convert", "60 mph", "m/s"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o), "26.8224 m/s\n");
    assert_eq!(
        stdout(&abaco(&["units", "convert", "1 atm", "kPa"])),
        "101.325 kPa\n"
    );
    assert_eq!(
        stdout(&abaco(&["units", "convert", "100 degC", "degF"])),
        "212 degF\n"
    );
    assert_eq!(
        stdout(&abaco(&["units", "convert", "1 kWh", "J", "--digits", "3"])),
        "3600000 J\n"
    );
    let o = abaco(&["units", "convert", "5 m", "s"]);
    assert_eq!(code(&o), 1);
    assert!(
        stderr(&o).contains("length") && stderr(&o).contains("time"),
        "{}",
        stderr(&o)
    );
    let o = abaco(&["units", "convert", "5 furlongs", "m"]);
    assert_eq!(code(&o), 1);
    let o = abaco(&["units", "convert", "5 m", "kg", "--json"]);
    assert_eq!(code(&o), 1);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["ok"], false);
    let v: Value =
        serde_json::from_str(&stdout(&abaco(&["units", "convert", "2 h", "s", "--json"]))).unwrap();
    assert_eq!(v["value"], 7200.0);
    assert_eq!(v["text"], "7200 s");
}

#[test]
fn units_list() {
    let o = abaco(&["units", "list"]);
    assert_eq!(code(&o), 0);
    let out = stdout(&o);
    assert!(out.lines().count() > 50, "{} units", out.lines().count());
    assert!(
        out.lines()
            .any(|l| l.starts_with("N ") && l.contains("newton") && l.contains("force")),
        "{out}"
    );
    let v: Value = serde_json::from_str(&stdout(&abaco(&["units", "list", "--json"]))).unwrap();
    let units = v["units"].as_array().unwrap();
    assert!(units
        .iter()
        .any(|u| u["symbol"] == "kg" || u["symbol"] == "g"));
    assert!(units
        .iter()
        .all(|u| u["si_factor"].as_f64().is_some_and(f64::is_finite)));
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        vec!["bogus"],
        vec!["eval"],
        vec!["eval", "1", "2"],
        vec!["run"],
        vec!["units"],
        vec!["units", "bogus"],
        vec!["units", "convert", "1 m"],
        vec!["eval", "1", "--bogus"],
    ] {
        let o = abaco(&args);
        assert_eq!(code(&o), 2, "{args:?}");
        assert!(stderr(&o).starts_with("error:"), "{args:?}: {}", stderr(&o));
        assert_eq!(stdout(&o), "", "{args:?}");
    }
}

#[test]
fn help_and_version() {
    for args in [
        vec![],
        vec!["--help"],
        vec!["-h"],
        vec!["help"],
        vec!["eval", "--help"],
        vec!["help", "run"],
        vec!["units", "-h"],
    ] {
        let o = abaco(&args);
        assert_eq!(code(&o), 0, "{args:?}");
        assert!(stdout(&o).contains("abaco"), "{args:?}");
    }
    let o = abaco(&["--version"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o), format!("abaco {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_closed_pipe_is_not_a_crash() {
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_abaco"))
        .args(["units", "list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
}
