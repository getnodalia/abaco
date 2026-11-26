// SPDX-License-Identifier: MIT OR Apache-2.0
//! Hostile input never panics: random source text, and every built-in with junk arguments.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::{builtins, Arr, Interp, Limits, Value};
use proptest::prelude::*;

fn small() -> Interp {
    Interp::with_limits(Limits {
        max_steps: 20_000,
        max_call_depth: 32,
        max_output: 10_000,
        ..Limits::default()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn arbitrary_text_never_panics(src in "\\PC{0,80}") {
        let _ = small().run(&src);
    }

    #[test]
    fn token_soup_never_panics(parts in proptest::collection::vec(
        prop_oneof![
            Just("("), Just(")"), Just("["), Just("]"), Just("{"), Just("}"), Just(","), Just(";"),
            Just("'"), Just("\""), Just("+"), Just("-"), Just("*"), Just(".*"), Just("^"), Just("'"),
            Just(":"), Just("="), Just("=="), Just("@"), Just("end"), Just("if"), Just("for"),
            Just("while"), Just("function"), Just("x"), Just("y"), Just("1"), Just("2.5e3"), Just("3i"),
            Just("\n"), Just(" "), Just("%c"), Just("..."), Just("zeros"), Just("sin"), Just("cellfun"),
        ], 0..40)) {
        let src = parts.concat();
        let _ = small().run(&src);
    }
}

fn junk_args() -> Vec<Vec<Value>> {
    let m = Value::mat(2, 2, vec![1.0, 2.0, 3.0, 4.0]);
    let big = Value::num(1e300);
    let nan = Value::num(f64::NAN);
    let neg = Value::num(-3.0);
    let s = Value::text("abc");
    let cell = Value::cell_row(vec![Value::num(1.0), Value::text("x")]);
    let empty = Value::empty();
    let fnv = Value::text("sin");
    vec![
        vec![],
        vec![Value::num(3.0)],
        vec![m.clone()],
        vec![m.clone(), m.clone()],
        vec![big.clone()],
        vec![nan.clone(), nan.clone()],
        vec![neg.clone(), neg.clone()],
        vec![s.clone()],
        vec![s.clone(), s.clone()],
        vec![cell.clone()],
        vec![cell, m.clone()],
        vec![empty.clone()],
        vec![empty.clone(), empty.clone(), empty],
        vec![big.clone(), big],
        vec![fnv, Value::num(2.0)],
        vec![Value::num(2.0), s, m],
        vec![Value::Num(Arr::new(
            1,
            3,
            vec![f64::INFINITY, f64::NEG_INFINITY, f64::NAN],
        ))],
        vec![Value::num(7.0), Value::num(0.0), Value::num(0.0)],
    ]
}

#[test]
fn every_builtin_survives_junk_arguments() {
    // Names that legitimately take time or touch the host are exercised elsewhere.
    let skip = ["pause", "input", "keyboard"];
    for name in builtins::names() {
        if skip.contains(&name) {
            continue;
        }
        for args in junk_args() {
            for nargout in [0usize, 1, 2] {
                let mut i = small();
                let _ = i.call_function(name, args.clone(), nargout);
            }
        }
    }
}

#[test]
fn huge_requests_are_refused_not_attempted() {
    for src in [
        "zeros(1e5, 1e5)",
        "ones(2e4, 2e4)",
        "rand(1e9)",
        "repmat(1:1000, 1e5, 1e5)",
        "eye(1e6)",
        "linspace(0, 1, 1e12)",
        "1:1e12",
        "kron(ones(5000), ones(5000))",
    ] {
        let mut i = small();
        assert!(
            i.run(&format!("x = {src};")).is_err(),
            "{src} should be refused"
        );
    }
}

/// Runs `src` on a thread with a deliberately small stack (the test harness default) and
/// reports whether it finished with a result or an error; an overflow aborts the process.
fn on_small_stack(src: String) -> Result<(), String> {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let mut i = small();
            i.run(&src).map_err(|e| e.id)
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn deep_trees_end_in_an_error_not_a_crash() {
    let shapes = [
        format!("x = 1{};", "+1".repeat(290)),
        format!("x = {}1{};", "(-".repeat(100), ")".repeat(100)),
        format!("x = {}1{};", "abs(".repeat(100), ")".repeat(100)),
        format!("a = 1; x = a{};", "'".repeat(290)),
        format!("x = 1{};", "^1".repeat(290)),
        format!("x = [{}1{}];", "[".repeat(100), "]".repeat(100)),
        format!("x = 1{};", " && 1".repeat(290)),
        format!("x = {}1;", "@() ".repeat(100)),
        format!("x = 1{};", "+1".repeat(5000)),
        format!("x = 1{};", "^1".repeat(5000)),
    ];
    for src in shapes {
        match on_small_stack(src.clone()) {
            Ok(()) => {}
            Err(id) => assert_eq!(id, "Abaco:limit", "{}", &src[..src.len().min(40)]),
        }
    }
}

#[test]
fn deep_nesting_is_bounded() {
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(small().run(&deep).is_err());
    let calls = format!("{}x{}", "f(".repeat(3000), ")".repeat(3000));
    assert!(small().run(&calls).is_err());
    let rec = "function r = f(n)\n r = f(n + 1);\nend\nf(0)";
    assert!(small().run(rec).is_err());
}
