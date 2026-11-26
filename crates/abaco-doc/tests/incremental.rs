// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Random edit sequences: an incrementally maintained document must always agree with the
//! same document evaluated from scratch, and nothing may panic.

use abaco_doc::{CellId, Document, Settings};
use proptest::prelude::*;

const SOURCES: &[&str] = &[
    "a = 1",
    "a = b + 1",
    "b = c * 2",
    "b = 3 m",
    "c = a + b",
    "c = 4",
    "d = a + c",
    "e = d / 0",
    "f(x) = x * a",
    "g = f(2)",
    "h = [a, b, c]",
    "a = 2; b = a + 1",
    "x = (1 +",
    "unknown + 1",
    "m = 2 kg",
    "w = m * 9.81 m/s^2 -> N",
    "",
    "# note",
    "a",
    "q = q + 1",
    "r = s",
    "s = r",
];

#[derive(Clone, Debug)]
enum Op {
    Push(usize),
    PushText,
    Insert(usize, usize),
    Edit(usize, usize),
    Remove(usize),
    Move(usize, usize),
    Strict(bool),
    Digits(usize),
    Evaluate,
}

fn op() -> impl Strategy<Value = Op> {
    let n = SOURCES.len();
    prop_oneof![
        3 => (0..n).prop_map(Op::Push),
        1 => Just(Op::PushText),
        2 => (0..100usize, 0..n).prop_map(|(i, s)| Op::Insert(i, s)),
        4 => (0..100usize, 0..n).prop_map(|(i, s)| Op::Edit(i, s)),
        2 => (0..100usize).prop_map(Op::Remove),
        2 => (0..100usize, 0..100usize).prop_map(|(a, b)| Op::Move(a, b)),
        1 => any::<bool>().prop_map(Op::Strict),
        1 => (1..18usize).prop_map(Op::Digits),
        3 => Just(Op::Evaluate),
    ]
}

fn pick(doc: &Document, i: usize) -> Option<CellId> {
    let n = doc.cells().len();
    if n == 0 {
        None
    } else {
        doc.cells().get(i % n).map(|c| c.id)
    }
}

fn apply(doc: &mut Document, op: &Op) {
    match op {
        Op::Push(s) => {
            doc.push_math(SOURCES[*s]).unwrap();
        }
        Op::PushText => {
            doc.push_text("prose").unwrap();
        }
        Op::Insert(i, s) => {
            let at = i % (doc.cells().len() + 1);
            doc.insert(
                at,
                abaco_doc::CellKind::Math {
                    source: SOURCES[*s].to_string(),
                },
            )
            .unwrap();
        }
        Op::Edit(i, s) => {
            if let Some(id) = pick(doc, *i) {
                let _ = doc.set_source(id, SOURCES[*s]);
            }
        }
        Op::Remove(i) => {
            if let Some(id) = pick(doc, *i) {
                doc.remove(id).unwrap();
            }
        }
        Op::Move(i, to) => {
            if let Some(id) = pick(doc, *i) {
                let n = doc.cells().len();
                doc.move_cell(id, to % n).unwrap();
            }
        }
        Op::Strict(b) => {
            let s = doc.settings();
            doc.set_settings(Settings {
                strict_order: *b,
                ..s
            })
            .unwrap();
        }
        Op::Digits(d) => {
            let s = doc.settings();
            doc.set_settings(Settings {
                significant_digits: *d,
                ..s
            })
            .unwrap();
        }
        Op::Evaluate => {
            doc.evaluate();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn incremental_equals_from_scratch(ops in proptest::collection::vec(op(), 1..40)) {
        let mut doc = Document::new();
        for o in &ops {
            apply(&mut doc, o);
        }
        doc.evaluate();
        let mut fresh = Document::from_json(&doc.to_json().unwrap()).unwrap();
        fresh.evaluate();
        prop_assert_eq!(doc.report(), fresh.report());
        // evaluating again changes nothing and runs nothing
        prop_assert!(doc.evaluate().is_empty());
    }

    #[test]
    fn every_intermediate_state_agrees(ops in proptest::collection::vec(op(), 1..25)) {
        let mut doc = Document::new();
        for o in &ops {
            apply(&mut doc, o);
            doc.evaluate();
            let mut fresh = Document::from_json(&doc.to_json().unwrap()).unwrap();
            fresh.evaluate();
            prop_assert_eq!(doc.report(), fresh.report(), "after {:?}", o);
        }
    }

    #[test]
    fn saved_files_round_trip(ops in proptest::collection::vec(op(), 0..30)) {
        let mut doc = Document::new();
        for o in &ops {
            apply(&mut doc, o);
        }
        let a = doc.to_json().unwrap();
        let b = Document::from_json(&a).unwrap().to_json().unwrap();
        prop_assert_eq!(a, b);
    }
}
