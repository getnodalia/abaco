// SPDX-License-Identifier: MIT OR Apache-2.0
//! File input and output from scripts, and `sub2ind`/`ind2sub`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::{Event, Interp, Limits};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("abaco-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open() -> Interp {
    Interp::with_limits(Limits {
        files: true,
        ..Limits::default()
    })
}

fn run(i: &mut Interp, src: &str) {
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
}

fn real(i: &Interp, name: &str) -> Vec<f64> {
    i.get(name).unwrap().to_real().unwrap().d
}

fn path(d: &std::path::Path, name: &str) -> String {
    d.join(name).to_string_lossy().replace('\\', "/")
}

#[test]
fn files_are_off_unless_the_host_allows_them() {
    let mut i = Interp::new();
    for src in [
        "readmatrix('x.csv')",
        "writematrix([1 2], 'x.csv')",
        "save('x.abd')",
        "load('x.abd')",
        "fileread('x.txt')",
    ] {
        let e = i.run(src).unwrap_err();
        assert_eq!(e.id, "Abaco:files", "{src}");
    }
}

#[test]
fn matrices_round_trip_through_csv_exactly() {
    let d = dir("csv");
    let f = path(&d, "m.csv");
    let mut i = open();
    run(
        &mut i,
        &format!(
            "A = [1 0.1 -2.5; 1/3 1e-300 NaN; Inf -Inf 12345];\nwritematrix(A, '{f}');\nB = readmatrix('{f}');\nok = isequaln(A, B);"
        ),
    );
    assert_eq!(real(&i, "ok"), vec![1.0]);
    let text = std::fs::read_to_string(&f).unwrap();
    assert_eq!(text.lines().next(), Some("1,0.1,-2.5"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn readmatrix_skips_headers_and_fills_gaps_with_nan() {
    let d = dir("hdr");
    let f = path(&d, "t.txt");
    std::fs::write(&f, "time;speed\n0;1.5\n1;\n2;3.5\n").unwrap();
    let mut i = open();
    run(&mut i, &format!("T = readmatrix('{f}');"));
    let t = i.get("T").unwrap().to_real().unwrap();
    assert_eq!((t.r, t.c), (3, 2));
    assert_eq!(t.d[..3], [0.0, 1.0, 2.0]);
    assert!(t.d[4].is_nan());
    run(
        &mut i,
        &format!("U = readmatrix('{f}', 'Delimiter', ';', 'NumHeaderLines', 2);"),
    );
    let u = i.get("U").unwrap().to_real().unwrap();
    assert_eq!((u.r, u.c), (2, 2));
    assert!(i
        .run("readmatrix('x', 'Bogus', 1)")
        .unwrap_err()
        .message
        .contains("Bogus"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn dlm_and_csv_functions_agree() {
    let d = dir("dlm");
    let f = path(&d, "a.dat");
    let mut i = open();
    run(
        &mut i,
        &format!(
            "dlmwrite('{f}', [1 2; 3 4], ';');\nA = dlmread('{f}', ';');\ncsvwrite('{f}', A * 2);\nB = csvread('{f}');"
        ),
    );
    assert_eq!(real(&i, "A"), vec![1.0, 3.0, 2.0, 4.0]);
    assert_eq!(real(&i, "B"), vec![2.0, 6.0, 4.0, 8.0]);
    run(&mut i, &format!("s = fileread('{f}'); e = isfile('{f}');"));
    assert_eq!(i.get("s").unwrap().as_str("s").unwrap(), "2,4\n6,8\n");
    assert_eq!(real(&i, "e"), vec![1.0]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_workspace_round_trips() {
    let d = dir("ws");
    let f = path(&d, "w");
    let mut i = open();
    run(
        &mut i,
        &format!(
            r#"
x = [1 2; 3 4.5];
z = [1+2i, 3-4i];
b = [true false true];
s = "line one\nline two\ttab \\ back";
c = {{1, 'two'; [3 4 5], {{6}}}};
st.a = 1; st.name = 'motor'; st.inner.k = [9 8];
h = @sin;
e = [];
save('{f}');
clear all
load('{f}');
"#
        ),
    );
    assert!(d.join("w.abd").is_file(), "the extension is added");
    assert_eq!(real(&i, "x"), vec![1.0, 3.0, 2.0, 4.5]);
    run(
        &mut i,
        "ok = isequal(z, [1+2i, 3-4i]) && isequal(b, [true false true]);",
    );
    assert_eq!(real(&i, "ok"), vec![1.0]);
    assert_eq!(
        i.get("s").unwrap().as_str("s").unwrap(),
        "line one\nline two\ttab \\ back"
    );
    run(
        &mut i,
        "k = st.inner.k; n = st.name; v = c{2,1}; w = c{2,2}{1}; t = c{1,2}; r = h(0); q = size(e);",
    );
    assert_eq!(real(&i, "k"), vec![9.0, 8.0]);
    assert_eq!(real(&i, "v"), vec![3.0, 4.0, 5.0]);
    assert_eq!(real(&i, "w"), vec![6.0]);
    assert_eq!(i.get("t").unwrap().as_str("t").unwrap(), "two");
    assert_eq!(real(&i, "r"), vec![0.0]);
    assert_eq!(real(&i, "q"), vec![0.0, 0.0]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn save_and_load_pick_variables_and_return_structs() {
    let d = dir("pick");
    let f = path(&d, "p.abd");
    let mut i = open();
    run(
        &mut i,
        &format!(
            "a = 1; b = 2; c = 3;\nsave('{f}', 'a', 'c');\nS = load('{f}');\nclear a c\nload('{f}', 'c');"
        ),
    );
    run(
        &mut i,
        "fa = isfield(S, 'a'); fb = isfield(S, 'b'); fc = isfield(S, 'c'); ea = exist('a');",
    );
    assert_eq!(real(&i, "fa"), vec![1.0]);
    assert_eq!(real(&i, "fb"), vec![0.0]);
    assert_eq!(real(&i, "fc"), vec![1.0]);
    assert_eq!(real(&i, "ea"), vec![0.0]);
    assert_eq!(real(&i, "c"), vec![3.0]);
    let e = i.run(&format!("save('{f}', 'nope')")).unwrap_err();
    assert!(e.message.contains("nope"), "{}", e.message);
    let e = i.run(&format!("load('{f}', 'b')")).unwrap_err();
    assert!(e.message.contains('b'), "{}", e.message);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_anonymous_function_is_skipped_with_a_warning() {
    let d = dir("anon");
    let f = path(&d, "f.abd");
    let mut i = open();
    run(
        &mut i,
        &format!("g = @(t) t.^2; k = 5;\nsave('{f}');\nclear all\nload('{f}');"),
    );
    assert_eq!(real(&i, "k"), vec![5.0]);
    assert!(i.get("g").is_none());
    let warned = i
        .take_events()
        .iter()
        .any(|e| matches!(e, Event::Warning(w) if w.contains('g')));
    assert!(warned);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn load_reads_a_text_file_of_numbers_into_a_named_variable() {
    let d = dir("txt");
    let f = path(&d, "my data.txt");
    std::fs::write(&f, "% a comment\n1 2 3\n4 5 6\n").unwrap();
    let mut i = open();
    run(&mut i, &format!("load('{f}'); M = load('{f}');"));
    let x = i.get("my_data").unwrap().to_real().unwrap();
    assert_eq!((x.r, x.c), (2, 3));
    assert_eq!(real(&i, "M"), x.d);
}

#[test]
fn bad_files_are_errors_that_name_the_file() {
    let d = dir("bad");
    let mut i = open();
    let missing = path(&d, "missing.csv");
    let e = i.run(&format!("readmatrix('{missing}')")).unwrap_err();
    assert!(e.message.contains("missing.csv"), "{}", e.message);
    let f = path(&d, "broken.abd");
    std::fs::write(&f, "ABACO-DATA 1\nvar x\nnum 2 2\n1 2\n").unwrap();
    let e = i.run(&format!("load('{f}')")).unwrap_err();
    assert_eq!(e.id, "Abaco:load");
    assert!(e.message.contains("line"), "{}", e.message);
    let e = i
        .run(&format!("readmatrix('{}')", path(&d, "")))
        .unwrap_err();
    assert!(e.message.contains("not a file"), "{}", e.message);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn sub2ind_and_ind2sub_are_inverses() {
    let mut i = Interp::new();
    run(
        &mut i,
        "k = sub2ind([3 4], [1 3 2], [1 4 2]); [r, c] = ind2sub([3 4], k); one = ind2sub([3 4], 5);",
    );
    assert_eq!(real(&i, "k"), vec![1.0, 12.0, 5.0]);
    assert_eq!(real(&i, "r"), vec![1.0, 3.0, 2.0]);
    assert_eq!(real(&i, "c"), vec![1.0, 4.0, 2.0]);
    assert_eq!(real(&i, "one"), vec![5.0]);
    run(
        &mut i,
        "A = magic(4); ok = A(sub2ind(size(A), 2, 3)) == A(2, 3);",
    );
    assert_eq!(real(&i, "ok"), vec![1.0]);
    for src in [
        "sub2ind([3 4], 4, 1)",
        "sub2ind([3 4], 1, 5)",
        "sub2ind([3 4], 0, 1)",
        "sub2ind([3 4], [1 2], 1)",
        "sub2ind([3 4 2], 1, 1)",
        "ind2sub([3 4], 13)",
        "ind2sub([3 4], 0)",
        "ind2sub([3 4], 1.5)",
    ] {
        assert!(i.run(src).is_err(), "{src}");
    }
}

#[test]
fn isequaln_treats_nan_as_equal() {
    let mut i = Interp::new();
    run(
        &mut i,
        "a = isequaln([1 NaN], [1 NaN]); b = isequal([1 NaN], [1 NaN]); c = isequaln([1 NaN], [1 2]); d = isequaln(struct('x', NaN), struct('x', NaN));",
    );
    assert_eq!(real(&i, "a"), vec![1.0]);
    assert_eq!(real(&i, "b"), vec![0.0]);
    assert_eq!(real(&i, "c"), vec![0.0]);
    assert_eq!(real(&i, "d"), vec![1.0]);
}
