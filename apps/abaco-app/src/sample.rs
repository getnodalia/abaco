// SPDX-License-Identifier: MIT OR Apache-2.0
//! The document a first launch opens: a small engineering example that shows units, a matrix
//! solve and a user function. The prose and the comments are interface text and follow the
//! language; the maths is the same in every language.

use crate::engine::CellSpec;
use crate::i18n::tr;

/// The cells of the sample, in page order.
pub fn cells() -> Vec<CellSpec> {
    use CellSpec::{Math, Text};
    let note = |source: &str, comment: &'static str| Math(format!("{source}  # {}", tr(comment)));
    vec![
        Text(tr("# A cantilever beam").to_owned()),
        Text(tr("A steel beam is fixed at one end and loaded at the other. Every number carries its unit, and every cell recomputes when something it depends on changes. Change any value below.").to_owned()),
        note("L = 2.5 m", "length"),
        note("P = 1.2 kN", "load at the free end"),
        note("E = 200 GPa", "Young's modulus of steel"),
        Math(format!(
            "b = 40 mm  # {}\nh = 80 mm  # {}",
            tr("section width"),
            tr("section height")
        )),
        note("I = b * h^3 / 12 -> cm^4", "second moment of area"),
        note("delta = P * L^3 / (3 * E * I) -> mm", "deflection at the free end"),
        note("sigma = P * L * (h / 2) / I -> MPa", "peak bending stress"),
        Text(tr("## Mesh currents").to_owned()),
        Text(tr("Kirchhoff's voltage law for a circuit with three loops is a linear system. `solve` returns the three loop currents.").to_owned()),
        note("R = [10, -4, 0; -4, 14, -6; 0, -6, 11]", "resistances in ohms"),
        note("V = [12, 0, -5]", "loop voltages in volts"),
        note("i = solve(R, V) * 1 A", "loop currents"),
        Text(tr("## A function").to_owned()),
        Text(tr("Functions are cells too. The period of a pendulum depends only on its length.").to_owned()),
        Math("T(len) = 2 * pi * sqrt(len / g0) -> s".to_owned()),
        note("T(1 m)", "a one metre pendulum"),
        note("T(L)", "as long as the beam"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Backend, Body, Outcome, Request};

    #[test]
    fn the_sample_evaluates_without_errors_in_both_languages() {
        for lang in [crate::i18n::Lang::En, crate::i18n::Lang::Es] {
            crate::i18n::set(lang);
            let mut b = Backend::new();
            let r = b.handle(vec![Request::New {
                cells: cells(),
                digits: 6,
            }]);
            assert_eq!(r.snapshot.errors, 0, "{lang:?}");
            assert!(r.notices.is_empty());
            assert!(!r.snapshot.dirty);
        }
        crate::i18n::set(crate::i18n::Lang::Es);
    }

    #[test]
    fn the_sample_gets_physically_sensible_numbers() {
        crate::i18n::set(crate::i18n::Lang::En);
        let mut b = Backend::new();
        let r = b.handle(vec![Request::New {
            cells: cells(),
            digits: 6,
        }]);
        let text = |name: &str| {
            r.snapshot
                .cells
                .iter()
                .find_map(|c| match &c.body {
                    Body::Math {
                        outcome:
                            Outcome::Ok {
                                value: Some(v),
                                defined,
                                ..
                            },
                        ..
                    } if defined.iter().any(|d| d == name) => Some(v.text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        // delta = P L^3 / (3 E I), I = b h^3 / 12 = 1.70667e-6 m^4: 18.75 kN m^3 / 1.024e3 N m^2
        assert!(text("delta").ends_with(" mm"), "{}", text("delta"));
        let delta: f64 = text("delta").trim_end_matches(" mm").parse().unwrap();
        assert!((delta - 18.3105).abs() < 1e-3, "{delta}");
        let sigma: f64 = text("sigma").trim_end_matches(" MPa").parse().unwrap();
        assert!((sigma - 70.3125).abs() < 1e-3, "{sigma}");
        crate::i18n::set(crate::i18n::Lang::Es);
    }
}
