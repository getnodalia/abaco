// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Random diagrams, some of them nonsense: compiling and running must give a result or an
//! error, never a panic, a hang or an unbounded allocation.

use abaco_blocks::*;
use proptest::prelude::*;

fn finite() -> impl Strategy<Value = f64> {
    prop_oneof![
        30 => -10.0f64..10.0,
        3 => Just(0.0),
        3 => Just(1.0),
        3 => Just(-1.0),
        1 => Just(1e300),
        1 => Just(f64::NAN),
        1 => Just(f64::INFINITY),
    ]
}

fn kind() -> impl Strategy<Value = BlockKind> {
    prop_oneof![
        finite().prop_map(|v| BlockKind::Constant { value: vec![v] }),
        (finite(), finite(), finite()).prop_map(|(time, before, after)| BlockKind::Step {
            time,
            before,
            after
        }),
        (finite(), finite(), finite(), finite()).prop_map(|(amplitude, frequency, phase, bias)| {
            BlockKind::Sine {
                amplitude,
                frequency,
                phase,
                bias,
            }
        }),
        finite().prop_map(|gain| BlockKind::Gain { gain }),
        prop_oneof![Just("++"), Just("+-"), Just("-"), Just("+++"), Just("+x")]
            .prop_map(|s| BlockKind::Sum { signs: s.into() }),
        prop_oneof![Just("**"), Just("*/"), Just("/")]
            .prop_map(|s| BlockKind::Product { ops: s.into() }),
        (finite(), finite()).prop_map(|(lower, upper)| BlockKind::Saturation { lower, upper }),
        finite().prop_map(|i| BlockKind::Integrator {
            initial: vec![i],
            lower: None,
            upper: None
        }),
        (finite(), finite()).prop_map(|(i, ts)| BlockKind::UnitDelay {
            initial: vec![i],
            sample_time: ts
        }),
        Just(BlockKind::Math {
            func: MathFunc::Abs
        }),
        Just(BlockKind::Math {
            func: MathFunc::Sign
        }),
        Just(BlockKind::Scope { inputs: 1 }),
        Just(BlockKind::Scope { inputs: 2 }),
    ]
}

fn model() -> impl Strategy<Value = Model> {
    (
        prop::collection::vec(kind(), 1..9),
        prop::collection::vec((0usize..9, 0usize..3, 0usize..9, 0usize..3), 0..16),
        prop_oneof![
            Just(Method::Euler),
            Just(Method::Rk4),
            Just(Method::Dopri5),
            Just(Method::Rosenbrock23)
        ],
        finite(),
    )
        .prop_map(|(kinds, lines, method, stop)| {
            let mut m = Model::new("fuzz");
            m.solver = SolverConfig {
                method,
                stop_time: if stop.is_finite() && stop > 0.0 {
                    stop.min(5.0)
                } else {
                    1.0
                },
                ..SolverConfig::default()
            };
            let n = kinds.len();
            for (i, k) in kinds.into_iter().enumerate() {
                m.add(format!("b{i}"), k).unwrap();
            }
            for (a, pa, b, pb) in lines {
                m.connect((&format!("b{}", a % n), pa), (&format!("b{}", b % n), pb));
            }
            m
        })
}

fn inputs_of(k: &BlockKind) -> usize {
    match k {
        BlockKind::Constant { .. } | BlockKind::Step { .. } | BlockKind::Sine { .. } => 0,
        BlockKind::Sum { signs } => signs.len(),
        BlockKind::Product { ops } => ops.len(),
        BlockKind::Scope { inputs } => *inputs,
        _ => 1,
    }
}

/// Every input driven by a random block's first output, so most of these compile and run
/// (loops included: the algebraic ones must be reported, the others must simulate).
fn wired() -> impl Strategy<Value = Model> {
    (
        prop::collection::vec(kind(), 1..9),
        prop::collection::vec(0usize..9, 64),
        prop_oneof![
            Just(Method::Euler),
            Just(Method::Rk4),
            Just(Method::Dopri5),
            Just(Method::Rosenbrock23)
        ],
    )
        .prop_map(|(kinds, picks, method)| {
            let mut m = Model::new("wired");
            m.solver = SolverConfig {
                method,
                stop_time: 2.0,
                ..SolverConfig::default()
            };
            // A constant first, so there is always something to drive from; scopes have
            // no output, so they are never a source.
            let mut kinds = kinds;
            kinds.insert(0, BlockKind::Constant { value: vec![1.0] });
            let sources: Vec<usize> = kinds
                .iter()
                .enumerate()
                .filter(|(_, k)| !matches!(k, BlockKind::Scope { .. }))
                .map(|(i, _)| i)
                .collect();
            let counts: Vec<usize> = kinds.iter().map(inputs_of).collect();
            for (i, k) in kinds.into_iter().enumerate() {
                m.add(format!("b{i}"), k).unwrap();
            }
            let mut pick = picks.into_iter().cycle();
            for (i, c) in counts.iter().enumerate() {
                for port in 0..*c {
                    let src = sources[pick.next().unwrap_or(0) % sources.len()];
                    m.connect((&format!("b{src}"), 0), (&format!("b{i}"), port));
                }
            }
            m
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn random_diagrams_never_panic(m in model()) {
        // Either outcome is fine; the point is that it returns.
        let _ = simulate(&m);
        // And the file format round-trips whatever was built.
        let text = m.to_json();
        if let Ok(text) = text {
            let _ = Model::from_json(&text);
        }
    }

    #[test]
    fn fully_wired_random_diagrams_never_panic(m in wired()) {
        match simulate(&m) {
            Ok(r) => {
                // A run that succeeds has finite, increasing times.
                prop_assert!(r.time.windows(2).all(|w| w[1] >= w[0]));
                prop_assert!(r.time.iter().all(|t| t.is_finite()));
            }
            Err(e) => prop_assert!(!e.to_string().is_empty()),
        }
    }

    #[test]
    fn random_text_is_not_a_model(s in ".{0,200}") {
        let _ = Model::from_json(&s);
    }
}
