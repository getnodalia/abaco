// SPDX-License-Identifier: MIT OR Apache-2.0
//! The function library against analytic results and published reference values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::{Interp, Value};

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

fn out(src: &str) -> String {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i.take_output()
}

fn near(a: f64, b: f64, tol: f64) {
    assert!(
        (a - b).abs() <= tol * b.abs().max(1.0),
        "{a} vs {b} (tol {tol})"
    );
}

fn all_near(a: &[f64], b: &[f64], tol: f64) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        near(*x, *y, tol);
    }
}

#[test]
fn linear_systems() {
    // A x = b with a known solution.
    all_near(
        &real("A = [4 -2 1; -2 4 -2; 1 -2 4]; b = [11; -16; 17]; A \\ b"),
        &[1.0, -2.0, 3.0],
        1e-12,
    );
    near(scalar("det([1 2; 3 4])"), -2.0, 1e-13);
    all_near(&real("inv([2 0; 0 4])"), &[0.5, 0.0, 0.0, 0.25], 1e-14);
    // Overdetermined: least squares fit of y = 2x + 1.
    all_near(
        &real("x = (0:4)'; A = [x ones(5,1)]; A \\ (2*x + 1)"),
        &[2.0, 1.0],
        1e-12,
    );
    near(scalar("trace(magic(4))"), 34.0, 0.0);
    near(scalar("rank(magic(4))"), 3.0, 0.0);
}

#[test]
fn decompositions() {
    // QR: Q*R reproduces A and Q is orthogonal.
    near(
        scalar("A = [1 2; 3 4; 5 6]; [Q, R] = qr(A); norm(Q*R - A)"),
        0.0,
        1e-13,
    );
    near(
        scalar("A = [1 2; 3 4; 5 6]; [Q, R] = qr(A); norm(Q'*Q - eye(3))"),
        0.0,
        1e-13,
    );
    // LU: P*A = L*U.
    near(
        scalar("A = magic(4); [L, U, P] = lu(A); norm(P*A - L*U)"),
        0.0,
        1e-12,
    );
    // Cholesky of a symmetric positive definite matrix.
    near(
        scalar("A = [4 2; 2 3]; R = chol(A); norm(R'*R - A)"),
        0.0,
        1e-14,
    );
    // SVD: singular values of a diagonal matrix, and reconstruction.
    all_near(&real("svd([3 0; 0 -4])"), &[4.0, 3.0], 1e-13);
    near(
        scalar("A = [1 2; 3 4; 5 6]; [U, S, V] = svd(A); norm(U*S*V' - A)"),
        0.0,
        1e-12,
    );
    near(scalar("cond([1 0; 0 0.01])"), 100.0, 1e-12);
    near(scalar("norm([3 4])"), 5.0, 1e-14);
    near(scalar("norm([1 2; 3 4], 'fro')"), 30f64.sqrt(), 1e-14);
}

#[test]
fn eigenvalues() {
    all_near(&real("eig([2 0; 0 3])"), &[2.0, 3.0], 1e-13);
    // Symmetric: eigenvalues of [2 1; 1 2] are 1 and 3.
    all_near(&real("eig([2 1; 1 2])"), &[1.0, 3.0], 1e-13);
    // A V = V D for a symmetric matrix.
    near(
        scalar("A = [4 1 0; 1 3 1; 0 1 2]; [V, D] = eig(A); norm(A*V - V*D)"),
        0.0,
        1e-12,
    );
    // Companion matrix of (x-1)(x-2)(x-3): roots 1, 2, 3.
    let mut r = real("sort(roots(poly([1 2 3])))");
    r.sort_by(|a, b| a.partial_cmp(b).unwrap());
    all_near(&r, &[1.0, 2.0, 3.0], 1e-10);
}

#[test]
fn matrix_exponential() {
    // expm of a rotation generator is a rotation.
    all_near(
        &real("expm([0 -pi/2; pi/2 0])"),
        &[0.0, 1.0, -1.0, 0.0],
        1e-12,
    );
    near(scalar("expm(0)"), 1.0, 1e-15);
    near(
        scalar("norm(expm([1 2; 3 4]) - [51.9690 74.7366; 112.1048 164.0738])"),
        0.0,
        1e-3,
    );
}

#[test]
fn pseudo_inverse_and_null_space() {
    near(
        scalar("A = [1 2; 3 4; 5 6]; norm(pinv(A)*A - eye(2))"),
        0.0,
        1e-12,
    );
    near(
        scalar("N = null([1 1; 1 1]); norm([1 1; 1 1]*N)"),
        0.0,
        1e-13,
    );
    near(scalar("size(null([1 2 3; 4 5 6; 7 8 9]), 2)"), 1.0, 0.0);
}

#[test]
fn reductions_and_statistics() {
    near(scalar("sum(1:100)"), 5050.0, 0.0);
    near(scalar("mean([1 2 3 4])"), 2.5, 0.0);
    near(scalar("median([5 1 3])"), 3.0, 0.0);
    near(
        scalar("std([2 4 4 4 5 5 7 9])"),
        (32.0f64 / 7.0).sqrt(),
        1e-14,
    );
    near(scalar("var([1 2 3 4])"), 5.0 / 3.0, 1e-14);
    near(scalar("mode([1 2 2 3 3 3])"), 3.0, 0.0);
    all_near(&real("cumsum([1 2 3 4])"), &[1.0, 3.0, 6.0, 10.0], 0.0);
    all_near(&real("sum([1 2; 3 4])"), &[4.0, 6.0], 0.0);
    all_near(&real("sum([1 2; 3 4], 2)"), &[3.0, 7.0], 0.0);
    all_near(&real("prod([1 2 3 4])"), &[24.0], 0.0);
    all_near(&real("diff([1 4 9 16])"), &[3.0, 5.0, 7.0], 0.0);
    near(scalar("corrcoef([1 2 3 4], [2 4 6 8])(1, 2)"), 1.0, 1e-14);
}

#[test]
fn sorting_searching_and_sets() {
    all_near(&real("sort([3 1 2], 'descend')"), &[3.0, 2.0, 1.0], 0.0);
    all_near(
        &real("[s, k] = sort([30 10 20]); ans = k"),
        &[2.0, 3.0, 1.0],
        0.0,
    );
    all_near(&real("unique([3 1 1 2 3])"), &[1.0, 2.0, 3.0], 0.0);
    all_near(&real("find([0 5 0 7])"), &[2.0, 4.0], 0.0);
    all_near(&real("max([1 9 3])"), &[9.0], 0.0);
    all_near(&real("[m, i] = max([1 9 3]); ans = i"), &[2.0], 0.0);
    all_near(&real("intersect([1 2 3 4], [3 4 5])"), &[3.0, 4.0], 0.0);
    all_near(&real("union([1 2], [2 3])"), &[1.0, 2.0, 3.0], 0.0);
    all_near(&real("setdiff([1 2 3 4], [2 4])"), &[1.0, 3.0], 0.0);
    all_near(&real("ismember([1 5 3], [1 2 3])"), &[1.0, 0.0, 1.0], 0.0);
}

#[test]
fn polynomials() {
    all_near(&real("conv([1 1], [1 -1])"), &[1.0, 0.0, -1.0], 0.0);
    all_near(&real("polyval([1 0 -1], 3)"), &[8.0], 0.0);
    all_near(&real("polyder([1 0 0 0])"), &[3.0, 0.0, 0.0], 0.0);
    all_near(&real("polyint([3 0 0])"), &[1.0, 0.0, 0.0, 0.0], 0.0);
    // Exact data from y = 2x^2 - 3x + 1 recovers the coefficients.
    all_near(
        &real("x = 0:5; polyfit(x, 2*x.^2 - 3*x + 1, 2)"),
        &[2.0, -3.0, 1.0],
        1e-10,
    );
    let q = real("[q, r] = deconv([1 0 -1], [1 1]); ans = q");
    all_near(&q, &[1.0, -1.0], 1e-14);
}

#[test]
fn calculus() {
    near(scalar("integral(@(x) x.^2, 0, 3)"), 9.0, 1e-10);
    near(scalar("integral(@sin, 0, pi)"), 2.0, 1e-10);
    near(
        scalar("integral(@(x) exp(-x.^2), -Inf, Inf)"),
        std::f64::consts::PI.sqrt(),
        1e-8,
    );
    near(scalar("integral2(@(x, y) x .* y, 0, 2, 0, 3)"), 9.0, 1e-8);
    near(scalar("trapz(0:0.001:1, (0:0.001:1).^2)"), 1.0 / 3.0, 1e-6);
    near(
        scalar("fzero(@(x) cos(x) - x, [0 1])"),
        0.7390851332151607,
        1e-10,
    );
    near(scalar("fzero(@(x) x.^3 - 2, 1)"), 2f64.cbrt(), 1e-10);
}

#[test]
fn special_functions() {
    near(scalar("gamma(5)"), 24.0, 1e-12);
    near(scalar("gamma(0.5)"), std::f64::consts::PI.sqrt(), 1e-12);
    near(scalar("erf(0.5)"), 0.5204998778130465, 1e-12);
    near(scalar("erfc(2)"), 0.004677734981047266, 1e-10);
    near(scalar("factorial(10)"), 3_628_800.0, 0.0);
    near(scalar("nchoosek(10, 3)"), 120.0, 0.0);
    near(scalar("gcd(48, 18)"), 6.0, 0.0);
    near(scalar("lcm(4, 6)"), 12.0, 0.0);
    all_near(
        &real("primes(20)"),
        &[2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0],
        0.0,
    );
    all_near(&real("factor(360)"), &[2.0, 2.0, 2.0, 3.0, 3.0, 5.0], 0.0);
    near(scalar("atan2(1, 1)"), std::f64::consts::FRAC_PI_4, 1e-15);
}

#[test]
fn constructors() {
    // Every magic square has equal row, column and diagonal sums, for odd, doubly-even
    // and singly-even orders alike.
    for n in [3, 4, 5, 6, 8, 10] {
        let s = (n * (n * n + 1) / 2) as f64;
        let src =
            format!("A = magic({n}); [sum(A, 1), sum(A, 2)', sum(diag(A)), sum(diag(fliplr(A)))]");
        let v = real(&src);
        assert!(v.iter().all(|x| (*x - s).abs() < 1e-9), "magic({n}): {v:?}");
    }
    all_near(
        &real("linspace(0, 1, 5)"),
        &[0.0, 0.25, 0.5, 0.75, 1.0],
        1e-15,
    );
    all_near(&real("repmat([1 2], 1, 2)"), &[1.0, 2.0, 1.0, 2.0], 0.0);
    all_near(&real("kron([1 2], [1; 1])"), &[1.0, 1.0, 2.0, 2.0], 0.0);
    all_near(&real("circshift(1:5, 2)"), &[4.0, 5.0, 1.0, 2.0, 3.0], 0.0);
    all_near(
        &real("toeplitz([1 2 3])"),
        &[1.0, 2.0, 3.0, 2.0, 1.0, 2.0, 3.0, 2.0, 1.0],
        0.0,
    );
    all_near(
        &real("[X, Y] = meshgrid(1:2, 1:3); ans = X"),
        &[1.0, 1.0, 1.0, 2.0, 2.0, 2.0],
        0.0,
    );
    // Seeded generators are reproducible.
    assert_eq!(real("rng(7); rand(1, 3)"), real("rng(7); rand(1, 3)"));
    let r = real("rand(1, 1000)");
    assert!(r.iter().all(|x| (0.0..1.0).contains(x)));
    let m = real("mean(randn(1, 20000))")[0];
    assert!(m.abs() < 0.05, "{m}");
}

#[test]
fn text_functions() {
    assert_eq!(out("printf('%s', mat2str([1 2; 3 4]))"), "[1 2;3 4]");
    assert_eq!(out("printf('%s', int2str(3.7))"), "4");
    assert_eq!(
        out("c = strsplit('a,b,,c', ','); printf('%d', numel(c))"),
        "4"
    );
    assert_eq!(
        out("[tok, rest] = strtok('hello world'); printf('%s|%s', tok, rest)"),
        "hello| world"
    );
    assert_eq!(out("printf('%d', strcmp('a', {'a', 'b'}))"), "10");
    assert_eq!(out("printf('%s', dec2bin(10))"), "1010");
    assert_eq!(out("printf('%d', bin2dec('1010'))"), "10");
    assert_eq!(out("printf('%d', hex2dec('FF'))"), "255");
    assert_eq!(out("[tok] = regexp('x=12, y=34', '(\\w)=(\\d+)', 'tokens'); printf('%s%s', tok{2}{1}, tok{2}{2})"), "y34");
    assert_eq!(
        out("printf('%d', numel(regexp('a1b22c333', '\\d+', 'match')))"),
        "3"
    );
    assert_eq!(out("printf('%s', strtrim('  hi  '))"), "hi");
    assert_eq!(out("printf('%d', contains('hello', 'ell'))"), "1");
    assert_eq!(out("printf('%e', 12345.678)"), "1.234568e+04");
    assert_eq!(out("printf('%g %g', 0.0001, 1e10)"), "0.0001 1e+10");
}

#[test]
fn cell_and_struct_functions() {
    assert_eq!(
        out("c = cellfun(@(x) x * 2, {1, 2, 3}); printf('%d,', c)"),
        "2,4,6,"
    );
    assert_eq!(
        out("c = cellfun(@(x) [x x], {1, 2}, 'UniformOutput', false); printf('%d', numel(c{2}))"),
        "2"
    );
    assert_eq!(out("printf('%d,', arrayfun(@(k) k^2, 1:4))"), "1,4,9,16,");
    assert_eq!(
        out("s = struct('a', 1, 'b', 2); f = fieldnames(s); printf('%s', f{2})"),
        "b"
    );
    assert_eq!(out("c = num2cell([1 2 3]); printf('%d', numel(c))"), "3");
    assert_eq!(out("m = cell2mat({1 2; 3 4}); printf('%d', m(2, 1))"), "3");
    assert_eq!(
        out("s = struct('k', 5); s = setfield(s, 'k', 9); printf('%d', getfield(s, 'k'))"),
        "9"
    );
    assert_eq!(out("printf('%d', isequal({1, 'a'}, {1, 'a'}))"), "1");
}

#[test]
fn error_messages_say_what_to_do() {
    let mut i = Interp::new();
    let e = i.run("ones(2, 3) * ones(2, 3)").unwrap_err();
    assert!(
        e.message.contains("inner")
            || e.message.contains("dimension")
            || e.message.contains("columns"),
        "{}",
        e.message
    );
    let e = i.run("inv([1 2; 2 4])").unwrap_err();
    assert!(
        e.message.to_lowercase().contains("singular"),
        "{}",
        e.message
    );
    let e = i.run("chol([1 2; 2 1])").unwrap_err();
    assert!(
        e.message.to_lowercase().contains("positive"),
        "{}",
        e.message
    );
    let e = i.run("x = sin()").unwrap_err();
    assert!(e.id == "Abaco:arity", "{}", e.id);
    let e = i.run("nosuchfn(1)").unwrap_err();
    assert!(e.message.contains("nosuchfn"));
}

#[test]
fn system_functions() {
    assert_eq!(out("tic; t = toc; printf('%d', t >= 0)"), "1");
    assert_eq!(out("printf('%d', bitand(12, 10)); printf(' %d', bitor(12, 10)); printf(' %d', bitxor(12, 10)); printf(' %d', bitshift(1, 4))"), "8 14 6 16");
    assert_eq!(out("eval('x = 3;'); printf('%d', x)"), "3");
    assert_eq!(
        out("eval('error(\"boom\")', 'disp(\"caught\")')").trim(),
        "caught"
    );
    assert_eq!(out("printf('%d', exist('pi'))"), "5");
    assert_eq!(out("y = 1; printf('%d', exist('y'))"), "1");
    assert_eq!(out("printf('%d', intmax('int8'))"), "127");
    assert_eq!(
        out("assert(true); assert(1, 1); assert(1, 1.0000001, 1e-3); printf('ok')"),
        "ok"
    );
    assert_eq!(out("try, assert(false, 'My:id', 'nope %d', 3); catch e, printf('%s %s', e.identifier, e.message); end"), "My:id nope 3");
}

fn eval_real(src: &str) -> Vec<f64> {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i.get("r").unwrap().to_real().unwrap().d
}

#[test]
fn special_matrices_match_their_definitions() {
    // column-major data
    assert_eq!(
        eval_real("r = blkdiag([1 2], 3);"),
        vec![1.0, 0.0, 2.0, 0.0, 0.0, 3.0]
    );
    assert_eq!(
        eval_real("r = hankel([1 2 3]);"),
        vec![1.0, 2.0, 3.0, 2.0, 3.0, 0.0, 3.0, 0.0, 0.0]
    );
    assert_eq!(
        eval_real("r = hankel([1 2], [2 5 6]);"),
        vec![1.0, 2.0, 2.0, 5.0, 5.0, 6.0]
    );
    assert_eq!(
        eval_real("r = pascal(3);"),
        vec![1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 1.0, 3.0, 6.0]
    );
    assert_eq!(
        eval_real("r = wilkinson(3);"),
        vec![1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0]
    );
    // the eigenvalues of the companion matrix are the roots of the polynomial
    assert_eq!(
        eval_real("r = compan([1 -3 2]);"),
        vec![3.0, 1.0, -2.0, 0.0]
    );
    let roots = eval_real("r = sort(eig(compan([1 -6 11 -6])))';");
    for (got, want) in roots.iter().zip([1.0, 2.0, 3.0]) {
        assert!((got - want).abs() < 1e-12, "{roots:?}");
    }
}

#[test]
fn perms_accumarray_gradient_and_conv2() {
    // MATLAB's order: reverse lexicographic by position
    assert_eq!(
        eval_real("r = perms([1 2 3]);"),
        vec![
            3.0, 3.0, 2.0, 2.0, 1.0, 1.0, 2.0, 1.0, 3.0, 1.0, 3.0, 2.0, 1.0, 2.0, 1.0, 3.0, 2.0,
            3.0
        ]
    );
    assert_eq!(
        eval_real("r = accumarray([1;2;1;3], [10;20;30;40]);"),
        vec![40.0, 20.0, 40.0]
    );
    assert_eq!(
        eval_real("r = accumarray([1 1; 2 2; 1 1], 5);"),
        vec![10.0, 0.0, 0.0, 5.0]
    );
    assert_eq!(
        eval_real("r = gradient([1 4 9 16], 1);"),
        vec![3.0, 4.0, 6.0, 7.0]
    );
    assert_eq!(
        eval_real("r = gradient([0 1 4 9], 0.5);"),
        vec![2.0, 4.0, 8.0, 10.0]
    );
    assert_eq!(
        eval_real("[gx, gy] = gradient([1 2 4; 2 4 8]); r = [gx(:); gy(:)]';").len(),
        12
    );
    assert_eq!(
        eval_real("r = conv2([1 2; 3 4], [1 1; 1 1]);"),
        vec![1.0, 4.0, 3.0, 3.0, 10.0, 7.0, 2.0, 6.0, 4.0]
    );
    assert_eq!(
        eval_real("r = conv2([1 2 3; 4 5 6; 7 8 9], [1 1; 1 1], 'valid');"),
        vec![12.0, 24.0, 16.0, 28.0]
    );
    assert_eq!(
        eval_real("r = conv2([1 2 3; 4 5 6; 7 8 9], ones(3), 'same');")[4],
        45.0
    );
}

#[test]
fn the_new_functions_refuse_bad_input() {
    for src in [
        "perms(1:11)",
        "accumarray([0; 1], [1; 2])",
        "accumarray([1; 2], [1; 2; 3])",
        "gradient([1 2 3], 0)",
        "realsqrt(-1)",
        "conv2([1 2], [1], 'cubic')",
        "pascal(1e6)",
        "wilkinson(1e6)",
    ] {
        let mut i = Interp::new();
        assert!(i.run(src).is_err(), "{src}");
    }
}
