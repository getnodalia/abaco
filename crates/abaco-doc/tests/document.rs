// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Evaluation order, dependencies, incremental updates and errors of a document.

use abaco_doc::{CellError, CellId, CellKind, DocError, Document, Settings};

fn text_of(doc: &Document, id: CellId) -> String {
    let r = doc.result(id).expect("evaluated");
    match &r.outcome {
        Ok(out) => out
            .value
            .as_ref()
            .map_or_else(|| "(no value)".to_string(), |v| doc.display(v)),
        Err(e) => format!("error: {}", e.message()),
    }
}

fn err_of(doc: &Document, id: CellId) -> CellError {
    doc.result(id)
        .expect("evaluated")
        .outcome
        .clone()
        .expect_err("an error")
}

#[test]
fn evaluates_in_dependency_order_not_page_order() {
    let mut doc = Document::new();
    let area = doc.push_math("area = width * height").unwrap();
    let w = doc.push_math("width = 3 m").unwrap();
    let h = doc.push_math("height = 4 m").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, area), "12 m^2");
    assert_eq!(text_of(&doc, w), "3 m");
    assert_eq!(text_of(&doc, h), "4 m");
    // no warning unless strict order is on
    assert!(doc.result(area).unwrap().warnings.is_empty());
}

#[test]
fn strict_order_warns_about_use_before_definition() {
    let mut doc = Document::new();
    let area = doc.push_math("area = width * 2").unwrap();
    let w = doc.push_math("width = 3 m").unwrap();
    doc.set_settings(Settings {
        strict_order: true,
        ..Settings::default()
    })
    .unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, area), "6 m");
    let warnings = &doc.result(area).unwrap().warnings;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("`width`"), "{warnings:?}");
    assert!(doc.result(w).unwrap().warnings.is_empty());
    // moving the definition up clears it
    doc.move_cell(w, 0).unwrap();
    doc.evaluate();
    assert!(doc.result(area).unwrap().warnings.is_empty());
    // and turning the setting off clears it too
    doc.move_cell(w, 1).unwrap();
    doc.evaluate();
    assert_eq!(doc.result(area).unwrap().warnings.len(), 1);
    doc.set_settings(Settings::default()).unwrap();
    doc.evaluate();
    assert!(doc.result(area).unwrap().warnings.is_empty());
}

#[test]
fn one_bad_cell_does_not_block_unrelated_ones() {
    let mut doc = Document::new();
    let good1 = doc.push_math("a = 1 + 1").unwrap();
    let bad = doc.push_math("b = 1 / 0").unwrap();
    let uses_bad = doc.push_math("c = b + 1").unwrap();
    let good2 = doc.push_math("d = a * 10").unwrap();
    let syntax = doc.push_math("q = (1 +").unwrap();
    let uses_unknown = doc.push_math("f = q + 1").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, good1), "2");
    assert_eq!(text_of(&doc, good2), "20");
    assert!(text_of(&doc, bad).contains("division by zero"));
    match err_of(&doc, uses_bad) {
        CellError::Dependency { cell, name, .. } => {
            assert_eq!(cell, bad);
            assert_eq!(name, "b");
        }
        other => panic!("{other:?}"),
    }
    match err_of(&doc, syntax) {
        CellError::Expr(e) => assert_eq!(e.kind, abaco_expr::ErrorKind::Syntax),
        other => panic!("{other:?}"),
    }
    // `q` never got defined, so `f` has an ordinary unknown-name error with a position
    match err_of(&doc, uses_unknown) {
        CellError::Expr(e) => {
            assert_eq!(e.kind, abaco_expr::ErrorKind::Name);
            assert_eq!(e.line_col("f = e + 1"), Some((1, 5)));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(doc.error_count(), 4);
}

#[test]
fn cycles_report_the_path() {
    let mut doc = Document::new();
    let a = doc.push_math("a = b + 1").unwrap();
    let b = doc.push_math("b = c + 1").unwrap();
    let c = doc.push_math("c = a + 1").unwrap();
    let ok = doc.push_math("z = 5").unwrap();
    let down = doc.push_math("y = a + z").unwrap();
    doc.evaluate();
    for id in [a, b, c] {
        match err_of(&doc, id) {
            CellError::Cycle { path, message } => {
                assert_eq!(path, vec![a, b, c], "{message}");
                assert_eq!(message, "circular reference: `a` -> `b` -> `c` -> `a`");
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(text_of(&doc, ok), "5");
    assert!(matches!(err_of(&doc, down), CellError::Dependency { .. }));

    // breaking the cycle heals everything downstream
    doc.set_source(c, "c = 1").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, a), "3");
    assert_eq!(text_of(&doc, down), "8");
}

#[test]
fn self_reference_is_a_cycle() {
    let mut doc = Document::new();
    let a = doc.push_math("x = x + 1").unwrap();
    let b = doc.push_math("y = 2; w = y + 1").unwrap();
    doc.evaluate();
    match err_of(&doc, a) {
        CellError::Cycle { path, message } => {
            assert_eq!(path, vec![a]);
            assert_eq!(message, "circular reference: `x` -> `x`");
        }
        other => panic!("{other:?}"),
    }
    // a name defined and then used inside one cell is not a cycle
    assert_eq!(text_of(&doc, b), "3");
}

#[test]
fn two_cycles_are_reported_separately() {
    let mut doc = Document::new();
    let a = doc.push_math("a = b").unwrap();
    let b = doc.push_math("b = a").unwrap();
    let c = doc.push_math("c = d").unwrap();
    let d = doc.push_math("d = c").unwrap();
    doc.evaluate();
    let path = |id| match err_of(&doc, id) {
        CellError::Cycle { path, .. } => path,
        other => panic!("{other:?}"),
    };
    assert_eq!(path(a), vec![a, b]);
    assert_eq!(path(c), vec![c, d]);
}

#[test]
fn duplicate_definitions_keep_the_first() {
    let mut doc = Document::new();
    let first = doc.push_math("x = 1").unwrap();
    let second = doc.push_math("x = 2").unwrap();
    let user = doc.push_math("y = x + 10").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, first), "1");
    assert_eq!(text_of(&doc, user), "11");
    match err_of(&doc, second) {
        CellError::Duplicate { name, first: f, .. } => {
            assert_eq!(name, "x");
            assert_eq!(f, first);
        }
        other => panic!("{other:?}"),
    }
    // removing the first hands the name to the second
    doc.remove(first).unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, second), "2");
    assert_eq!(text_of(&doc, user), "12");
}

#[test]
fn functions_cross_cells() {
    let mut doc = Document::new();
    let g = doc.push_math("g = 9.81 m/s^2").unwrap();
    let f = doc.push_math("weight(m) = m * g").unwrap();
    let use_it = doc.push_math("weight(80 kg) -> N").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, f), "(no value)");
    assert_eq!(text_of(&doc, use_it), "784.8 N");
    // changing g changes the function's behaviour in its users
    doc.set_source(g, "g = 1.62 m/s^2").unwrap();
    let ran = doc.evaluate();
    assert_eq!(ran.len(), 3);
    assert_eq!(text_of(&doc, use_it), "129.6 N");
}

#[test]
fn text_cells_are_never_evaluated() {
    let mut doc = Document::new();
    let t = doc.push_text("# A title\nx = 1 / 0").unwrap();
    let m = doc.push_math("1 + 1").unwrap();
    assert_eq!(doc.evaluate(), vec![m]);
    assert!(doc.result(t).is_none());
    assert_eq!(doc.set_source(t, "1"), Err(DocError::WrongKind(t, "math")));
    assert_eq!(doc.set_text(m, "x"), Err(DocError::WrongKind(m, "text")));
    doc.set_text(t, "new").unwrap();
    assert!(!doc.needs_evaluation());
}

#[test]
fn empty_and_comment_cells_are_fine() {
    let mut doc = Document::new();
    let a = doc.push_math("").unwrap();
    let b = doc.push_math("# just a note").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, a), "(no value)");
    assert_eq!(text_of(&doc, b), "(no value)");
    assert_eq!(doc.error_count(), 0);
}

// ---------------------------------------------------------------- incremental evaluation

#[test]
fn only_the_edited_cell_and_its_dependents_run_again() {
    let mut doc = Document::new();
    let a = doc.push_math("a = 1").unwrap();
    let b = doc.push_math("b = a + 1").unwrap();
    let c = doc.push_math("c = b + 1").unwrap();
    let lone = doc.push_math("lone = 100").unwrap();
    let lone_user = doc.push_math("lone2 = lone * 2").unwrap();
    assert_eq!(doc.evaluate().len(), 5);
    assert_eq!(doc.evaluations(), 5);

    // nothing changed: nothing runs
    assert!(doc.evaluate().is_empty());
    assert_eq!(doc.evaluations(), 5);

    // editing a runs a, b, c in dependency order, and nothing else
    doc.set_source(a, "a = 10").unwrap();
    assert_eq!(doc.evaluate(), vec![a, b, c]);
    assert_eq!(doc.evaluations(), 8);
    assert_eq!(text_of(&doc, c), "12");

    // editing the leaf runs only the leaf
    doc.set_source(c, "c = b * 2").unwrap();
    assert_eq!(doc.evaluate(), vec![c]);
    assert_eq!(text_of(&doc, c), "22");

    // an edit that produces the same outcome does not wake the dependents
    doc.set_source(a, "a = 5 + 5").unwrap();
    assert_eq!(doc.evaluate(), vec![a]);
    assert_eq!(doc.evaluations(), 10);

    // an unrelated chain is untouched by all of this
    assert_eq!(text_of(&doc, lone_user), "200");
    doc.set_source(lone, "lone = 1").unwrap();
    assert_eq!(doc.evaluate(), vec![lone, lone_user]);
    assert_eq!(text_of(&doc, lone_user), "2");
    assert_eq!(doc.last_evaluated(), &[lone, lone_user]);
}

#[test]
fn adding_a_definition_wakes_the_cells_that_were_missing_it() {
    let mut doc = Document::new();
    let user = doc.push_math("y = x * 2").unwrap();
    doc.evaluate();
    assert!(text_of(&doc, user).contains("unknown name `x`"));
    let x = doc.push_math("x = 21").unwrap();
    assert_eq!(doc.evaluate(), vec![x, user]);
    assert_eq!(text_of(&doc, user), "42");
    // removing it breaks the user again
    doc.remove(x).unwrap();
    assert_eq!(doc.evaluate(), vec![user]);
    assert!(text_of(&doc, user).contains("unknown name"));
}

#[test]
fn renaming_a_definition_moves_the_dependency() {
    let mut doc = Document::new();
    let d = doc.push_math("old = 5").unwrap();
    let u = doc.push_math("r = old + new").unwrap();
    let n = doc.push_math("new = 1").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, u), "6");
    doc.set_source(d, "old = 7").unwrap();
    let ran = doc.evaluate();
    assert_eq!(ran, vec![d, u]);
    assert_eq!(text_of(&doc, u), "8");
    doc.set_source(n, "renamed = 1").unwrap();
    doc.evaluate();
    assert!(text_of(&doc, u).contains("unknown name `new`"));
}

#[test]
fn ids_are_stable_and_never_reused() {
    let mut doc = Document::new();
    let a = doc.push_math("a = 1").unwrap();
    let b = doc.push_math("b = 2").unwrap();
    let c = doc.push_math("c = 3").unwrap();
    doc.move_cell(c, 0).unwrap();
    assert_eq!(
        doc.cells().iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![c, a, b]
    );
    doc.remove(a).unwrap();
    let d = doc
        .insert(
            1,
            CellKind::Math {
                source: "d = 4".into(),
            },
        )
        .unwrap();
    assert!(d.0 > c.0 && d.0 > b.0, "ids only grow: {d:?}");
    assert_ne!(d, a);
    assert_eq!(doc.position(d), Some(1));
    assert_eq!(doc.cell(b).map(|x| x.id), Some(b));
    assert_eq!(doc.remove(a), Err(DocError::UnknownCell(a)));
    assert_eq!(
        doc.move_cell(b, 9),
        Err(DocError::BadIndex { index: 9, len: 3 })
    );
    assert!(matches!(
        doc.insert(
            9,
            CellKind::Text {
                markdown: String::new()
            }
        ),
        Err(DocError::BadIndex { .. })
    ));
}

#[test]
fn display_precision_is_a_setting() {
    let mut doc = Document::new();
    let a = doc.push_math("1/3").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, a), "0.333333");
    doc.set_settings(Settings {
        significant_digits: 10,
        ..Settings::default()
    })
    .unwrap();
    assert!(!doc.needs_evaluation(), "display digits do not re-evaluate");
    assert_eq!(text_of(&doc, a), "0.3333333333");
    for bad in [0, 18, 100] {
        assert!(matches!(
            doc.set_settings(Settings {
                significant_digits: bad,
                ..Settings::default()
            }),
            Err(DocError::InvalidSetting(_))
        ));
    }
}

#[test]
fn per_cell_limits_apply() {
    let mut doc = Document::new();
    let a = doc.push_math("sum(1:100000)").unwrap();
    doc.evaluate();
    assert_eq!(text_of(&doc, a), "5.00005e9");
    doc.set_limits(abaco_expr::Limits {
        max_steps: 1000,
        ..abaco_expr::Limits::default()
    });
    doc.evaluate();
    assert!(text_of(&doc, a).contains("limit"), "{}", text_of(&doc, a));
}

#[test]
fn size_limits_on_editing() {
    let mut doc = Document::new();
    let big = "1".repeat(abaco_doc::MAX_SOURCE_BYTES + 1);
    assert!(matches!(
        doc.push_math(big.clone()),
        Err(DocError::TooLarge(_))
    ));
    let id = doc.push_math("1").unwrap();
    assert!(matches!(
        doc.set_source(id, big),
        Err(DocError::TooLarge(_))
    ));
    let huge = "x".repeat(abaco_doc::MAX_TEXT_BYTES + 1);
    assert!(matches!(doc.push_text(huge), Err(DocError::TooLarge(_))));
    for _ in 1..abaco_doc::MAX_CELLS {
        doc.push_text("").unwrap();
    }
    assert!(matches!(doc.push_text(""), Err(DocError::TooLarge(_))));
}

// ---------------------------------------------------------------- scale

#[test]
fn five_thousand_cells_evaluate_quickly() {
    let mut doc = Document::new();
    // a chain, so evaluation order matters, interleaved with independent cells and prose
    doc.push_math("x0 = 0").unwrap();
    for k in 1..2_500 {
        doc.push_math(format!("x{k} = x{} + 1", k - 1)).unwrap();
        doc.push_math(format!("y{k} = {k} m * 2 -> cm")).unwrap();
    }
    assert_eq!(doc.cells().len(), 4_999);
    doc.push_text("end").unwrap();
    let start = std::time::Instant::now();
    let ran = doc.evaluate();
    let took = start.elapsed();
    assert_eq!(ran.len(), 4_999);
    assert_eq!(doc.error_count(), 0);
    let last = doc
        .cells()
        .iter()
        .rev()
        .find_map(|c| doc.result(c.id).map(|_| c.id));
    assert!(last.is_some());
    eprintln!("5000 cells evaluated in {took:?}");
    // Generous for an unoptimised build; the target is well under a second with optimisation.
    let budget = if cfg!(debug_assertions) { 15 } else { 1 };
    assert!(took.as_secs() < budget, "took {took:?}");

    // editing the head of the chain re-evaluates the 2,500 cells downstream, not all 5,000
    let first = doc.cells()[0].id;
    doc.set_source(first, "x0 = 100").unwrap();
    let start = std::time::Instant::now();
    let ran = doc.evaluate();
    eprintln!(
        "chain edit re-evaluated {} cells in {:?}",
        ran.len(),
        start.elapsed()
    );
    assert_eq!(ran.len(), 2_500);
    assert!(start.elapsed().as_secs() < budget);
}

#[test]
fn a_very_long_cycle_does_not_overflow_the_stack() {
    let mut doc = Document::new();
    let n = 3_000;
    for k in 0..n {
        doc.push_math(format!("c{k} = c{} + 1", (k + 1) % n))
            .unwrap();
    }
    let h = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            doc.evaluate();
            doc.error_count()
        })
        .unwrap();
    assert_eq!(h.join().unwrap(), n);
}
