// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The `.abaco` file format: round trips, refusals and hostile files.

use abaco_doc::{CellKind, Document, FormatError, Settings, FORMAT_VERSION};
use serde_json::json;

fn sample() -> Document {
    let mut d = Document::new();
    d.push_text("# Beam\nA *simple* check.").unwrap();
    d.push_math("L = 2.5 m").unwrap();
    d.push_math("q = 3 kN/m").unwrap();
    d.push_math("M = q * L^2 / 8 -> kN*m").unwrap();
    d.push_math("s = \"unicode ñ 日本\"").unwrap();
    d.set_settings(Settings {
        significant_digits: 8,
        strict_order: true,
    })
    .unwrap();
    d
}

#[test]
fn round_trip_is_byte_identical() {
    let d = sample();
    let a = d.to_json().unwrap();
    let back = Document::from_json(&a).unwrap();
    let b = back.to_json().unwrap();
    assert_eq!(a, b);
    assert_eq!(back.cells(), d.cells());
    assert_eq!(back.settings(), d.settings());
    assert_eq!(back.next_id(), d.next_id());
}

#[test]
fn saved_text_is_deterministic_and_shaped_as_documented() {
    let d = sample();
    let text = d.to_json().unwrap();
    assert_eq!(text, d.to_json().unwrap());
    assert!(text.ends_with("}\n"));
    // key order is the documented one
    let order: Vec<usize> = [
        "\"format\"",
        "\"format_version\"",
        "\"settings\"",
        "\"next_cell_id\"",
        "\"cells\"",
        "\"engine\"",
    ]
    .iter()
    .map(|k| text.find(k).unwrap_or_else(|| panic!("missing {k}")))
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["format"], "abaco");
    assert_eq!(v["format_version"], 1);
    assert_eq!(v["cells"][0]["type"], "text");
    assert_eq!(v["cells"][1]["source"], "L = 2.5 m");
    assert_eq!(v["engine"]["abaco-expr"], abaco_expr::VERSION);
    assert_eq!(v["engine"]["nodalia-units"], nodalia_units::VERSION);
}

#[test]
fn results_are_recomputed_after_loading() {
    let mut d = sample();
    d.evaluate();
    let before = d.report();
    let mut back = Document::from_json(&d.to_json().unwrap()).unwrap();
    assert!(
        back.result(back.cells()[1].id).is_none(),
        "no results are stored"
    );
    back.evaluate();
    assert_eq!(back.report(), before);
}

fn minimal(extra: serde_json::Value) -> String {
    let mut v = json!({
        "format": "abaco",
        "format_version": 1,
        "cells": [{"type": "math", "id": 1, "source": "1 + 1"}],
    });
    if let (Some(base), Some(add)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in add {
            if val.is_null() {
                base.remove(k);
            } else {
                base.insert(k.clone(), val.clone());
            }
        }
    }
    v.to_string()
}

#[test]
fn minimal_files_get_defaults() {
    let d = Document::from_json(&minimal(json!({}))).unwrap();
    assert_eq!(d.settings(), Settings::default());
    assert_eq!(d.next_id(), 2);
    let d = Document::from_json(&minimal(json!({"cells": []}))).unwrap();
    assert!(d.cells().is_empty());
}

#[test]
fn newer_versions_are_refused_not_read_partially() {
    for v in [2u64, 3, 99, 1_000_000] {
        // even when the new version adds cell types this build does not know
        let text = json!({
            "format": "abaco", "format_version": v,
            "cells": [{"type": "plot", "id": 1, "series": []}],
        })
        .to_string();
        match Document::from_json(&text) {
            Err(FormatError::NewerVersion { found, supported }) => {
                assert_eq!(found, v);
                assert_eq!(supported, FORMAT_VERSION);
            }
            other => panic!("{other:?}"),
        }
    }
    let msg = Document::from_json(&minimal(json!({"format_version": 2})))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("update"), "{msg}");
}

#[test]
fn not_abaco_files() {
    for (extra, needle) in [
        (json!({"format": "pcraft"}), "pcraft"),
        (json!({"format": null}), "format"),
        (json!({"format_version": null}), "format_version"),
    ] {
        let e = Document::from_json(&minimal(extra)).unwrap_err();
        assert!(matches!(e, FormatError::NotAbaco(_)), "{e:?}");
        assert!(e.to_string().contains(needle), "{e}");
    }
    for bad in [
        json!("1"),
        json!(1.5),
        json!(-1),
        json!(0),
        json!(null),
        json!([1]),
        json!(1e30),
    ] {
        let text = format!(
            "{{\"format\":\"abaco\",\"format_version\":{},\"cells\":[]}}",
            bad
        );
        let r = Document::from_json(&text);
        assert!(r.is_err() || bad == json!(null), "{bad} gave {r:?}");
    }
    assert!(Document::from_json("[]").is_err());
    assert!(Document::from_json("42").is_err());
    assert!(Document::from_json("null").is_err());
    assert!(Document::from_json("").is_err());
}

#[test]
fn malformed_json_has_a_position() {
    let e = Document::from_json(
        "{\n  \"format\": \"abaco\",\n  \"format_version\": 1,\n  \"cells\": [ {\"type\": }\n}",
    )
    .unwrap_err();
    match e {
        FormatError::Malformed { line, column, .. } => {
            assert_eq!(line, 4);
            assert!(column > 10, "{column}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn wrong_types_are_errors() {
    let cases = [
        json!({"cells": "nope"}),
        json!({"cells": [1]}),
        json!({"cells": [{"type": "math", "id": "one", "source": "1"}]}),
        json!({"cells": [{"type": "math", "id": 1, "source": 5}]}),
        json!({"cells": [{"type": "math", "id": 1}]}),
        json!({"cells": [{"id": 1, "source": "1"}]}),
        json!({"cells": [{"type": "text", "id": 1, "source": "1"}]}),
        json!({"cells": [{"type": "plot", "id": 1}]}),
        json!({"cells": [{"type": "math", "id": 1.5, "source": "1"}]}),
        json!({"cells": [{"type": "math", "id": -1, "source": "1"}]}),
        json!({"settings": {"significant_digits": "six"}}),
        json!({"settings": {"significant_digits": -3}}),
        json!({"settings": {"strict_order": "yes"}}),
        json!({"settings": 7}),
        json!({"next_cell_id": "x"}),
    ];
    for extra in cases {
        let text = minimal(extra.clone());
        assert!(Document::from_json(&text).is_err(), "{extra} was accepted");
    }
}

#[test]
fn huge_and_odd_numbers_are_refused() {
    let big_ids = [
        "1e300",
        "99999999999999999999999999999",
        "18446744073709551616",
        "9007199254740993",
        "0",
    ];
    for id in big_ids {
        let text = format!(
            "{{\"format\":\"abaco\",\"format_version\":1,\"cells\":[{{\"type\":\"math\",\"id\":{id},\"source\":\"1\"}}]}}"
        );
        assert!(Document::from_json(&text).is_err(), "id {id} was accepted");
    }
    for digits in ["0", "18", "1000000", "1e300", "99999999999999999999"] {
        let text = format!(
            "{{\"format\":\"abaco\",\"format_version\":1,\"settings\":{{\"significant_digits\":{digits}}},\"cells\":[]}}"
        );
        assert!(
            Document::from_json(&text).is_err(),
            "digits {digits} accepted"
        );
    }
    let text = "{\"format\":\"abaco\",\"format_version\":1,\"next_cell_id\":9007199254740993,\"cells\":[]}";
    assert!(Document::from_json(text).is_err());
}

#[test]
fn duplicate_ids_and_a_low_counter() {
    let text = json!({
        "format": "abaco", "format_version": 1,
        "cells": [
            {"type": "math", "id": 5, "source": "1"},
            {"type": "text", "id": 5, "markdown": "x"},
        ],
    })
    .to_string();
    assert!(Document::from_json(&text)
        .unwrap_err()
        .to_string()
        .contains("more than one"));
    // a counter below the largest id would reuse ids: it is raised
    let text = json!({
        "format": "abaco", "format_version": 1, "next_cell_id": 2,
        "cells": [{"type": "math", "id": 40, "source": "1"}],
    })
    .to_string();
    let mut d = Document::from_json(&text).unwrap();
    assert_eq!(d.next_id(), 41);
    assert_eq!(d.push_math("2").unwrap().0, 41);
}

#[test]
fn unknown_fields_are_ignored_and_not_kept() {
    let text = json!({
        "format": "abaco", "format_version": 1, "future_thing": {"a": [1, 2]},
        "settings": {"significant_digits": 4, "mystery": true},
        "cells": [{"type": "math", "id": 1, "source": "1", "color": "red"}],
    })
    .to_string();
    let d = Document::from_json(&text).unwrap();
    assert_eq!(d.settings().significant_digits, 4);
    let saved = d.to_json().unwrap();
    assert!(
        !saved.contains("future_thing") && !saved.contains("mystery") && !saved.contains("color")
    );
}

#[test]
fn size_limits_on_read() {
    let big = "1".repeat(abaco_doc::MAX_SOURCE_BYTES + 1);
    let text =
        json!({"format":"abaco","format_version":1,"cells":[{"type":"math","id":1,"source":big}]})
            .to_string();
    assert!(matches!(
        Document::from_json(&text),
        Err(FormatError::TooLarge(_))
    ));
    let cells: Vec<_> = (1..=abaco_doc::MAX_CELLS as u64 + 1)
        .map(|i| json!({"type":"text","id":i,"markdown":""}))
        .collect();
    let text = json!({"format":"abaco","format_version":1,"cells":cells}).to_string();
    assert!(matches!(
        Document::from_json(&text),
        Err(FormatError::TooLarge(_))
    ));
    let filler = " ".repeat(abaco_doc::MAX_FILE_BYTES as usize + 1);
    assert!(matches!(
        Document::from_json(&filler),
        Err(FormatError::TooLarge(_))
    ));
}

#[test]
fn deeply_nested_json_cannot_overflow_the_stack() {
    let h = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            // nested junk in an unknown field is skipped without recursion
            let text = format!(
                "{{\"format\":\"abaco\",\"format_version\":1,\"x\":{}{},\"cells\":[]}}",
                "[".repeat(100_000),
                "]".repeat(100_000)
            );
            let _ = Document::from_json(&text);
            // nesting where the reader needs the structure is refused by serde_json's depth limit
            let text = format!(
                "{{\"format\":\"abaco\",\"format_version\":1,\"cells\":{}{}}}",
                "[".repeat(100_000),
                "]".repeat(100_000)
            );
            assert!(Document::from_json(&text).is_err());
            let text = "[".repeat(200_000);
            assert!(Document::from_json(&text).is_err());
        })
        .unwrap();
    h.join().unwrap();
}

#[test]
fn every_truncation_of_a_valid_file_is_handled() {
    let text = sample().to_json().unwrap();
    let mut accepted = 0;
    for end in 0..text.trim_end().len() {
        if !text.is_char_boundary(end) {
            continue;
        }
        if Document::from_json(&text[..end]).is_ok() {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 0, "a truncated file must not load");
    assert!(Document::from_json(&text).is_ok());
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("abaco-doc-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn save_and_open_round_trip_atomically() {
    let path = temp_path("round.abaco");
    let d = sample();
    d.save(&path).unwrap();
    let back = Document::open(&path).unwrap();
    assert_eq!(back.cells(), d.cells());
    // overwriting replaces the old content and leaves no temporary file
    let mut d2 = Document::new();
    d2.push_math("2 + 2").unwrap();
    d2.save(&path).unwrap();
    assert_eq!(Document::open(&path).unwrap().cells().len(), 1);
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn open_reports_files_that_cannot_be_read() {
    let missing = temp_path("missing.abaco");
    let e = Document::open(&missing).unwrap_err();
    assert!(matches!(e, FormatError::Io { .. }), "{e:?}");
    assert!(e.to_string().contains("missing.abaco"));
    let bad = temp_path("binary.abaco");
    std::fs::write(&bad, [0xff, 0xfe, 0x00, 0x80]).unwrap();
    assert!(matches!(Document::open(&bad), Err(FormatError::Invalid(_))));
    std::fs::remove_file(&bad).unwrap();
    // a directory is not a document
    assert!(Document::open(std::env::temp_dir()).is_err());
    // saving into a folder that does not exist is an error, not a panic
    let nowhere = temp_path("no/such/dir/x.abaco");
    assert!(matches!(
        sample().save(&nowhere),
        Err(FormatError::Io { .. })
    ));
}

#[test]
fn unicode_survives() {
    let mut d = Document::new();
    d.push_text("日本語 ñ é 🙂 \u{0} \"quotes\" \\ back\nslash")
        .unwrap();
    d.push_math("s = \"ñ 🙂\"").unwrap();
    let back = Document::from_json(&d.to_json().unwrap()).unwrap();
    assert_eq!(back.cells(), d.cells());
    match &back.cells()[0].kind {
        CellKind::Text { markdown } => {
            assert!(markdown.contains('🙂') && markdown.contains('\u{0}'))
        }
        other => panic!("{other:?}"),
    }
}

mod mutation {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        /// Corrupting a valid file in a few places must give a document or an error, never a
        /// panic, and whatever loads must save and load again identically.
        #[test]
        fn corrupted_files_never_panic(
            edits in proptest::collection::vec((0usize..2000, any::<char>(), 0usize..3), 1..6)
        ) {
            let mut chars: Vec<char> = sample().to_json().unwrap().chars().collect();
            for (at, ch, kind) in edits {
                if chars.is_empty() { break; }
                let i = at % chars.len();
                match kind {
                    0 => chars[i] = ch,
                    1 => { chars.remove(i); }
                    _ => chars.insert(i, ch),
                }
            }
            let text: String = chars.into_iter().collect();
            if let Ok(doc) = Document::from_json(&text) {
                let again = Document::from_json(&doc.to_json().unwrap()).unwrap();
                prop_assert_eq!(again.cells(), doc.cells());
                let mut doc = doc;
                doc.evaluate();
            }
        }
    }
}
