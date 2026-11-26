// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Hostile input: nothing here may panic, hang or overflow the stack.

use abaco_expr::{eval_str, eval_str_with, parse, Env, ErrorKind, Limits};
use proptest::prelude::*;
use std::time::{Duration, Instant};

const TOKENS: &[&str] = &[
    "1",
    "2",
    "0",
    "3.5",
    ".5",
    "1e3",
    "1e-3",
    "1_000",
    "1e999",
    "x",
    "y",
    "m",
    "kg",
    "s",
    "N",
    "degC",
    "sin",
    "cos",
    "sqrt",
    "ln",
    "sum",
    "min",
    "max",
    "if",
    "det",
    "inv",
    "solve",
    "zeros",
    "eye",
    "range",
    "linspace",
    "factorial",
    "gamma",
    "pi",
    "e",
    "i",
    "c",
    "h",
    "true",
    "false",
    "and",
    "or",
    "not",
    "to",
    "->",
    "+",
    "-",
    "*",
    "/",
    "^",
    ".*",
    "./",
    ".^",
    "==",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "=",
    "(",
    ")",
    "[",
    "]",
    ",",
    ";",
    ":",
    "\n",
    "#",
    "\"",
    "\"txt\"",
    "f",
    "g(x)",
    "x =",
    "f(x) =",
    "2i",
    "3 m",
    "9.81 m/s^2",
    "µ",
    "°",
    "²",
    "é",
    "日本",
    "\u{0}",
    "'",
    "$",
    "@",
    "%",
];

fn run_guarded(src: &str) -> Duration {
    let start = Instant::now();
    let mut env = Env::new();
    let _ = eval_str("f(x) = x^2 + 1", &mut env);
    let _ = eval_str("x = 2; y = [1,2;3,4]", &mut env);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = eval_str(src, &mut env);
        let _ = parse(src);
    }));
    assert!(r.is_ok(), "panicked on {src:?}");
    start.elapsed()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_text_never_panics(s in "\\PC{0,200}") {
        let took = run_guarded(&s);
        prop_assert!(took < Duration::from_secs(10), "slow input {:?}: {:?}", s, took);
    }

    #[test]
    fn random_bytes_as_text_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let s = String::from_utf8_lossy(&bytes).into_owned();
        run_guarded(&s);
    }

    #[test]
    fn token_soup_never_panics(toks in proptest::collection::vec(proptest::sample::select(TOKENS), 0..60)) {
        let src = toks.join(" ");
        let took = run_guarded(&src);
        prop_assert!(took < Duration::from_secs(10), "slow input {:?}: {:?}", src, took);
        let src = toks.concat();
        run_guarded(&src);
    }

    #[test]
    fn evaluation_is_deterministic(toks in proptest::collection::vec(proptest::sample::select(TOKENS), 0..30)) {
        let src = toks.join(" ");
        let a = eval_str(&src, &mut Env::new());
        let b = eval_str(&src, &mut Env::new());
        prop_assert_eq!(a, b);
    }

    #[test]
    fn errors_carry_in_range_spans(toks in proptest::collection::vec(proptest::sample::select(TOKENS), 0..40)) {
        let src = toks.join(" ");
        if let Err(e) = eval_str(&src, &mut Env::new()) {
            if let Some(span) = e.span {
                prop_assert!(span.start <= span.end, "{:?} {:?}", src, span);
                prop_assert!(span.end <= src.len(), "{:?} {:?}", src, span);
                prop_assert!(src.is_char_boundary(span.start) && src.is_char_boundary(span.end), "{:?} {:?}", src, span);
                let (line, col) = e.line_col(&src).unwrap();
                prop_assert!(line >= 1 && col >= 1);
                let _ = e.render(&src);
            }
        }
    }
}

/// Valid-looking expressions built from a grammar, so deeper evaluator paths are reached.
fn expr_strategy() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("1".to_string()),
        Just("2.5".to_string()),
        Just("x".to_string()),
        Just("3 m".to_string()),
        Just("2 s".to_string()),
        Just("pi".to_string()),
        Just("i".to_string()),
        Just("[1,2,3]".to_string()),
        Just("[1,2;3,4]".to_string()),
        Just("y".to_string()),
        Just("\"s\"".to_string()),
        Just("true".to_string()),
    ];
    leaf.prop_recursive(6, 64, 4, |inner| {
        prop_oneof![
            (
                inner.clone(),
                prop_oneof![
                    Just("+"),
                    Just("-"),
                    Just("*"),
                    Just("/"),
                    Just("^"),
                    Just(".*"),
                    Just("<"),
                    Just("=="),
                    Just("and")
                ],
                inner.clone()
            )
                .prop_map(|(a, op, b)| format!("({a} {op} {b})")),
            inner.clone().prop_map(|a| format!("-{a}")),
            (
                prop_oneof![
                    Just("sin"),
                    Just("sqrt"),
                    Just("ln"),
                    Just("abs"),
                    Just("sum"),
                    Just("det"),
                    Just("inv"),
                    Just("transpose"),
                    Just("norm"),
                    Just("f")
                ],
                inner.clone()
            )
                .prop_map(|(f, a)| format!("{f}({a})")),
            (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(a, b, c)| format!("if({a}, {b}, {c})")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a} {b}")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("{a}[{b}]")),
            (
                inner.clone(),
                prop_oneof![Just("m"), Just("s"), Just("kg"), Just("N"), Just("degC")]
            )
                .prop_map(|(a, u)| format!("{a} -> {u}")),
            (inner.clone(), inner).prop_map(|(a, b)| format!("{a}:{b}")),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn grammar_expressions_never_panic(src in expr_strategy()) {
        let took = run_guarded(&src);
        prop_assert!(took < Duration::from_secs(10), "slow input {:?}: {:?}", src, took);
    }
}

fn time_limited(src: &str, kind: ErrorKind, secs: u64) {
    let start = Instant::now();
    let e = eval_str(src, &mut Env::new()).expect_err(src);
    assert_eq!(e.kind, kind, "{src}: {}", e.message);
    assert!(
        start.elapsed() < Duration::from_secs(secs),
        "{src} took {:?}",
        start.elapsed()
    );
}

#[test]
fn runaway_work_is_stopped() {
    time_limited("zeros(300, 300) * zeros(300, 300)", ErrorKind::Limit, 20);
    time_limited(
        "sum(1:99999) * zeros(300, 300) * zeros(300,300)",
        ErrorKind::Limit,
        20,
    );
    time_limited("inv(zeros(300, 300))", ErrorKind::Limit, 20);
    time_limited("det(ones(500, 500))", ErrorKind::Limit, 20);
    time_limited(&vec!["sum(1:100000)"; 60].join(" + "), ErrorKind::Limit, 20);
    time_limited("ones(200,200) ^ 1000000", ErrorKind::Limit, 20);
}

#[test]
fn overflow_is_an_error_not_infinity() {
    time_limited("10^10^10", ErrorKind::Numeric, 5);
    time_limited("factorial(170) * factorial(170)", ErrorKind::Numeric, 5);
    time_limited("exp(1000)", ErrorKind::Numeric, 5);
    time_limited("1e308 + 1e308", ErrorKind::Numeric, 5);
    let e = eval_str("0/0", &mut Env::new()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Domain);
}

#[test]
fn custom_limits_are_honoured() {
    let tight = Limits {
        max_steps: 50,
        ..Limits::default()
    };
    let e = eval_str_with("sum(1:1000)", &mut Env::new(), &tight).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    let tight = Limits {
        max_source: 10,
        ..Limits::default()
    };
    let e = eval_str_with("1 + 1 + 1 + 1 + 1 + 1", &mut Env::new(), &tight).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    let tight = Limits {
        max_elements: 10,
        ..Limits::default()
    };
    let e = eval_str_with("1:11", &mut Env::new(), &tight).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    assert!(eval_str_with("1:10", &mut Env::new(), &tight).is_ok());
}

/// The evaluator and parser must fit in a small thread stack (1 MB, even in a debug build)
/// for the deepest input the limits allow.
#[test]
fn worst_case_nesting_fits_in_a_small_stack() {
    let cases = [
        format!("{}1{}", "(".repeat(31), ")".repeat(31)),
        format!("{}1{}", "abs(".repeat(31), ")".repeat(31)),
        format!("{}1", "-".repeat(31)),
        format!("{}1", "1+".repeat(199)),
        format!("{}1", "2^".repeat(30)),
        "fact(n) = if(n <= 1, 1, n * fact(n - 1)); fact(60)".to_string(),
        "f(n) = f(n + 1); f(0)".to_string(),
    ];
    for src in cases {
        let h = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn({
                let src = src.clone();
                move || {
                    let _ = eval_str(&src, &mut Env::new());
                }
            })
            .unwrap();
        assert!(h.join().is_ok(), "stack overflow or panic on {src:.60}");
    }
}
