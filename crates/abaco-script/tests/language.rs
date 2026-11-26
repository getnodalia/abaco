// SPDX-License-Identifier: MIT OR Apache-2.0
//! The language: syntax, semantics, indexing, control flow, functions, cells and structs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::{Interp, Value};

fn run(src: &str) -> String {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i.take_output()
}

/// Runs `src` and returns the value of its last expression (`ans`).
fn val(src: &str) -> Value {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i.get("ans")
        .cloned()
        .unwrap_or_else(|| panic!("`{src}` produced no value"))
}

fn real(src: &str) -> Vec<f64> {
    val(src).to_real().unwrap().d
}

fn scalar(src: &str) -> f64 {
    let v = real(src);
    assert_eq!(v.len(), 1, "{src} gave {v:?}");
    v[0]
}

fn fails(src: &str) -> String {
    let mut i = Interp::new();
    match i.run(src) {
        Ok(()) => panic!("expected an error from: {src}"),
        Err(e) => e.message,
    }
}

fn close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol * b.abs().max(1.0), "{a} vs {b}");
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(scalar("2 + 3 * 4"), 14.0);
    assert_eq!(scalar("-2^2"), -4.0);
    assert_eq!(scalar("2^3^2"), 64.0); // MATLAB: left associative
    assert_eq!(scalar("(1+2)*3"), 9.0);
    assert_eq!(scalar("7 / 2"), 3.5);
    assert_eq!(scalar("mod(-1, 3)"), 2.0);
    assert_eq!(scalar("rem(-1, 3)"), -1.0);
    assert_eq!(scalar("1e3 + 0x10"), 1016.0);
}

#[test]
fn ranges_and_indexing() {
    assert_eq!(real("1:5"), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    assert_eq!(real("10:-3:0"), vec![10.0, 7.0, 4.0, 1.0]);
    assert_eq!(real("0:0.1:0.3").len(), 4);
    assert_eq!(real("(1:5)(end)"), vec![5.0]);
    let out = run("A = magic(4); x = A(2:3, [1 end]); disp(x)");
    assert!(out.contains("5") && out.contains("8"), "{out}");
    assert_eq!(scalar("A = reshape(1:6, 2, 3); A(2, 3)"), 6.0);
    assert_eq!(scalar("A = reshape(1:6, 2, 3); A(5)"), 5.0);
    assert_eq!(real("A = reshape(1:6, 2, 3); A(:)'").len(), 6);
    assert_eq!(real("v = 1:5; v(v > 2)"), vec![3.0, 4.0, 5.0]);
}

#[test]
fn assignment_grows_and_deletes() {
    let mut i = Interp::new();
    i.run("v = []; for k = 1:4, v(end+1) = k^2; end").unwrap();
    assert_eq!(
        i.get("v").unwrap().to_real().unwrap().d,
        vec![1.0, 4.0, 9.0, 16.0]
    );
    i.run("v(2) = []").unwrap();
    assert_eq!(
        i.get("v").unwrap().to_real().unwrap().d,
        vec![1.0, 9.0, 16.0]
    );
    i.run("M = zeros(2); M(3, 3) = 7;").unwrap();
    assert_eq!(i.get("M").unwrap().dims(), (3, 3));
    i.run("M(:, 2) = []").unwrap();
    assert_eq!(i.get("M").unwrap().dims(), (3, 2));
}

#[test]
fn matrix_literals_and_concatenation() {
    assert_eq!(real("[1, 2; 3, 4]"), vec![1.0, 3.0, 2.0, 4.0]);
    assert_eq!(val("[1 2 3]'").dims(), (3, 1));
    assert_eq!(val("[[1 2]; [3 4]]").dims(), (2, 2));
    assert_eq!(val("[1:3, 7]").dims(), (1, 4));
    assert!(fails("x = [1 2; 3]").contains("columns"));
}

#[test]
fn control_flow() {
    let out = run("s = 0; for k = 1:10, if mod(k, 2) == 0, continue, end, s = s + k; end, disp(s)");
    assert_eq!(out.trim(), "25");
    let out = run("n = 0; while true, n = n + 1; if n >= 5, break; end, end, disp(n)");
    assert_eq!(out.trim(), "5");
    let out = run("switch 'abc', case {'x','abc'}, disp('hit'); otherwise, disp('miss'); end");
    assert_eq!(out.trim(), "hit");
}

#[test]
fn functions_and_recursion() {
    let src = "function r = fact(n)\n if n <= 1, r = 1; else, r = n * fact(n - 1); end\nend\ndisp(fact(10))";
    assert_eq!(run(src).trim(), "3628800");
    let src = "function [s, p] = sp(a, b)\n s = a + b; p = a * b;\nend\n[x, y] = sp(3, 4); printf('%d %d', x, y)";
    assert_eq!(run(src), "7 12");
    let src = "function varargout = many(varargin)\n for k = 1:nargout, varargout{k} = k * numel(varargin); end\nend\n[a, b, c] = many(1, 2); printf('%d %d %d', a, b, c)";
    assert_eq!(run(src), "2 4 6");
}

#[test]
fn anonymous_functions_capture_values() {
    assert_eq!(
        run("a = 3; f = @(x) a * x; a = 100; disp(f(2))").trim(),
        "6"
    );
    assert_eq!(scalar("g = @sin; g(0)"), 0.0);
    assert_eq!(
        scalar("compose = @(f, g) @(x) f(g(x)); h = compose(@(x) x + 1, @(x) 2 * x); h(5)"),
        11.0
    );
    assert_eq!(scalar("cellfun(@numel, {'ab', 'cde'})(2)"), 3.0);
}

#[test]
fn try_catch_and_errors() {
    let out = run("try, error('Pkg:bad', 'value %d is bad', 7); catch err, disp(err.identifier); disp(err.message); end");
    assert_eq!(out, "Pkg:bad\nvalue 7 is bad\n");
    let out = run("try, x = [1 2 3]; x(5); catch e, disp(e.message); end");
    assert!(out.contains("5"), "{out}");
    assert!(fails("undefined_thing + 1").contains("undefined_thing"));
}

#[test]
fn cells_and_structs() {
    let out = run("c = {1, 'two', [3 4]}; disp(numel(c)); disp(class(c{2})); disp(c{3}(2))");
    assert_eq!(out, "3\nchar\n4\n");
    let out = run("s.a = 1; s.b.c = 'deep'; s.list = {1, 2}; disp(s.b.c); disp(isfield(s, 'a')); disp(numel(fieldnames(s)))");
    assert_eq!(out, "deep\n1\n3\n");
    let out = run("s = struct('x', 3, 'y', 'hi'); disp(s.x)");
    assert_eq!(out.trim(), "3");
}

#[test]
fn strings_and_printf() {
    assert_eq!(
        run("printf('%5.2f|%-4d|%s\\n', pi, 42, 'ok')"),
        " 3.14|42  |ok\n"
    );
    assert_eq!(run("printf('%d,', [1 2 3])"), "1,2,3,");
    assert_eq!(run("disp(upper('abc'))").trim(), "ABC");
    assert_eq!(run("disp(strrep('a-b-c', '-', '+'))").trim(), "a+b+c");
    assert_eq!(run("disp(num2str(3.14159, 4))").trim(), "3.142");
    assert_eq!(run("disp(strjoin({'a','b','c'}, ', '))").trim(), "a, b, c");
    assert_eq!(
        run("x = regexprep('abc123', '(\\d+)', '<$1>'); disp(x)").trim(),
        "abc<123>"
    );
    assert_eq!(scalar("str2double('2.5e2')"), 250.0);
    assert_eq!(run("disp(sprintf('%x', 255))").trim(), "ff");
}

#[test]
fn display_of_assignments() {
    assert_eq!(run("x = 5").trim(), "x = 5");
    let out = run("A = [1 2; 3 4]");
    assert!(
        out.contains("A =") && out.contains("1") && out.contains("4"),
        "{out}"
    );
    assert_eq!(run("x = 5;"), "");
    assert_eq!(run("1 + 1").trim(), "ans = 2");
}

#[test]
fn complex_numbers() {
    close(scalar("abs(3 + 4i)"), 5.0, 1e-14);
    close(scalar("real(exp(1i * pi)) + 1"), 0.0, 1e-14);
    close(scalar("imag(sqrt(-4))"), 2.0, 1e-14);
    close(scalar("angle(1i)"), std::f64::consts::FRAC_PI_2, 1e-14);
    close(scalar("abs(roots([1 0 1])(1))"), 1.0, 1e-12);
}

#[test]
fn logical_operations() {
    assert_eq!(real("[1 0 1] & [1 1 0]"), vec![1.0, 0.0, 0.0]);
    assert_eq!(scalar("any([0 0 1])"), 1.0);
    assert_eq!(scalar("xor(true, false)"), 1.0);
    assert_eq!(run("if [] , disp('t'); else, disp('f'); end").trim(), "f");
    assert_eq!(scalar("1 || (1/0 > 5)"), 1.0);
}

#[test]
fn limits_stop_runaway_scripts() {
    let mut i = Interp::with_limits(abaco_script::Limits {
        max_steps: 10_000,
        ..Default::default()
    });
    let e = i.run("while true, end").unwrap_err();
    assert_eq!(e.id, "Abaco:limit");
    let e = Interp::new().run("zeros(1e6, 1e6)").unwrap_err();
    assert_eq!(e.id, "Abaco:limit");
}

#[test]
fn command_syntax_and_comments() {
    let out = run("format long\npi % a comment\nformat short\ndisp(pi)");
    assert!(out.contains("3.141592653589793"), "{out}");
    assert!(out.contains("3.1416"), "{out}");
    let out = run("x = 1; # hash comment\n%{\nblock comment\n%}\ndisp(x)");
    assert_eq!(out.trim(), "1");
    assert_eq!(run("x = [1, 2, ...\n 3]; disp(numel(x))").trim(), "3");
}

#[test]
fn plotting_records_figures() {
    let mut i = Interp::new();
    i.run("x = 0:0.1:1; plot(x, x.^2, 'r--'); hold on; plot(x, x); title('t'); xlabel('x'); legend('a', 'b'); grid on").unwrap();
    let fig = i.figures.cur();
    let ax = fig.ax();
    assert_eq!(ax.series.len(), 2);
    assert_eq!(ax.title, "t");
    assert!(ax.grid && ax.legend);
    assert_eq!(ax.series[0].label, "a");
    assert_eq!(ax.series[0].spec, "r--");
    i.run("figure(2); surf(magic(4));").unwrap();
    assert!(i.figures.list.iter().any(|(k, _)| *k == 2));
}
