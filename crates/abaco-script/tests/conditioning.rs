// SPDX-License-Identifier: MIT OR Apache-2.0
//! A system whose answer is mostly rounding noise is reported, never returned silently.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::{Event, Interp};

fn warnings(src: &str) -> Vec<String> {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Warning(w) => Some(w),
            _ => None,
        })
        .collect()
}

#[test]
fn a_singular_matrix_is_solved_with_a_warning_naming_rcond() {
    // magic(4) has rank 3: rcond is about 1e-17 (MATLAB reports 4.6e-18).
    let w = warnings("A = magic(4); x = A \\ [1;2;3;4];");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("close to singular") && w[0].contains("RCOND"),
        "{w:?}"
    );
}

#[test]
fn inv_of_a_nearly_singular_matrix_warns_too() {
    assert_eq!(warnings("B = inv([1 2 3; 4 5 6; 7 8 9]);").len(), 1);
}

#[test]
fn a_well_conditioned_system_says_nothing() {
    assert!(warnings("x = [4 1; 2 3] \\ [1; 2]; y = inv(eye(3));").is_empty());
}

#[test]
fn rcond_matches_the_reference_values() {
    let mut i = Interp::new();
    i.run("a = rcond(eye(4)); b = rcond([1 2; 2 4]); c = rcond(hilb(6));")
        .unwrap();
    let get = |n: &str| i.get(n).unwrap().to_real().unwrap().d[0];
    assert!(
        (get("a") - 1.0).abs() < 1e-12,
        "the identity is perfectly conditioned"
    );
    assert_eq!(get("b"), 0.0, "an exactly singular matrix");
    // 1-norm condition number of the 6x6 Hilbert matrix is 2.9070e7 (published value).
    let expected = 1.0 / 2.9070e7;
    assert!(
        (get("c") / expected - 1.0).abs() < 0.2,
        "rcond(hilb(6)) = {} against {expected}",
        get("c")
    );
}
