// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Behaviour of the expression language, checked against hand-computed and published values.

use abaco_expr::{eval_str, Env, ErrorKind, ExprError, Value};

fn run(src: &str) -> Result<Value, ExprError> {
    let mut env = Env::new();
    run_in(src, &mut env)
}

fn run_in(src: &str, env: &mut Env) -> Result<Value, ExprError> {
    // a definition has no value; stand in for it with an empty string
    eval_str(src, env).map(|e| e.value.unwrap_or_else(|| Value::Str(String::new())))
}

fn show(src: &str) -> String {
    run(src)
        .unwrap_or_else(|e| panic!("`{src}` failed: {e}"))
        .to_string()
}

fn real(src: &str) -> f64 {
    match run(src).unwrap_or_else(|e| panic!("`{src}` failed: {e}")) {
        Value::Real(x) => x,
        Value::Quantity(q) => q.si_value(),
        other => panic!("`{src}` is {other:?}, not a number"),
    }
}

fn close(src: &str, want: f64, rel: f64) {
    let got = real(src);
    let tol = rel * want.abs().max(1e-300);
    assert!(
        (got - want).abs() <= tol,
        "`{src}` = {got:e}, want {want:e} (tolerance {tol:e})"
    );
}

fn fails(src: &str) -> ExprError {
    match run(src) {
        Ok(v) => panic!("`{src}` should fail but gave {v}"),
        Err(e) => e,
    }
}

/// The value as shown in its own unit (so `0 degC -> degF` is 32, not kelvin).
fn disp(src: &str) -> f64 {
    match run(src).unwrap_or_else(|e| panic!("`{src}` failed: {e}")) {
        Value::Real(x) => x,
        Value::Quantity(q) => q.display_value(),
        other => panic!("`{src}` is {other:?}, not a number"),
    }
}

fn complex(src: &str) -> (f64, f64) {
    match run(src).unwrap() {
        Value::Complex(z) => (z.re, z.im),
        Value::Real(x) => (x, 0.0),
        other => panic!("`{src}` is {other:?}"),
    }
}

// ---------------------------------------------------------------- precedence and literals

#[test]
fn precedence_table() {
    let cases = [
        ("1 + 2 * 3", 7.0),
        ("(1 + 2) * 3", 9.0),
        ("10 - 4 - 3", 3.0),
        ("100 / 10 / 5", 2.0),
        ("-2^2", -4.0),
        ("(-2)^2", 4.0),
        ("2^3^2", 512.0),
        ("2^-3", 0.125),
        ("-3 + 5", 2.0),
        ("2 * -3", -6.0),
        ("--4", 4.0),
        ("2^2*3", 12.0),
        ("12 / 3 * 2", 8.0),
        ("1_000 * 2", 2000.0),
        ("1e3", 1000.0),
        ("1.5e-3", 0.0015),
        (".5 + .25", 0.75),
        ("2 * 3 + 4 * 5", 26.0),
        ("7 - -2", 9.0),
    ];
    for (src, want) in cases {
        assert_eq!(real(src), want, "{src}");
    }
}

#[test]
fn implicit_multiplication() {
    let mut env = Env::new();
    run_in("x = 4", &mut env).unwrap();
    assert_eq!(run_in("2x", &mut env).unwrap().to_string(), "8");
    assert_eq!(run_in("3(4+5)", &mut env).unwrap().to_string(), "27");
    assert_eq!(run_in("(2+3)(4+1)", &mut env).unwrap().to_string(), "25");
    assert_eq!(run_in("2 x + 1", &mut env).unwrap().to_string(), "9");
    assert_eq!(run_in("x(3)", &mut env).unwrap().to_string(), "12");
    assert_eq!(show("2 pi"), "6.28318530717959");
    assert_eq!(show("(2+3) m"), "5 m");
    // juxtaposition binds tighter than `/`: `12/2x` is `12/(2x)`, as in handwritten maths
    assert_eq!(run_in("12/2x", &mut env).unwrap().to_string(), "1.5");
    assert_eq!(run_in("12/2*x", &mut env).unwrap().to_string(), "24");
    assert_eq!(show("5 m / 2 s"), "2.5 m/s");
}

#[test]
fn comparison_and_logic() {
    assert_eq!(show("1 < 2"), "true");
    assert_eq!(show("2 <= 2"), "true");
    assert_eq!(show("3 > 4"), "false");
    assert_eq!(show("2 == 2"), "true");
    assert_eq!(show("2 != 2"), "false");
    assert_eq!(show("1 < 2 and 2 < 3"), "true");
    assert_eq!(show("1 > 2 or 2 < 3"), "true");
    assert_eq!(show("not (1 > 2)"), "true");
    assert_eq!(show("1 m < 1 km"), "true");
    assert_eq!(show("100 cm == 1 m"), "true");
    // short circuit: the right side would be an error
    assert_eq!(show("1 > 2 and 1/0 > 1"), "false");
    assert_eq!(show("1 < 2 or 1/0 > 1"), "true");
    assert_eq!(fails("1 and 2").kind, ErrorKind::Type);
}

#[test]
fn if_is_lazy() {
    assert_eq!(show("if(1 < 2, 10, 20)"), "10");
    assert_eq!(show("if(1 > 2, 1/0, 5)"), "5");
    assert_eq!(fails("if(3, 1, 2)").kind, ErrorKind::Type);
    assert_eq!(fails("if(true, 1)").kind, ErrorKind::Arity);
}

#[test]
fn strings_and_comments() {
    assert_eq!(show("\"hello\""), "\"hello\"");
    assert_eq!(show("1 + 2 # a comment"), "3");
    assert_eq!(show("# only a comment\n7"), "7");
    assert!(run("\"a\" + 1").is_err());
}

// ---------------------------------------------------------------- units

#[test]
fn units_by_juxtaposition() {
    assert_eq!(show("9.81 m/s^2"), "9.81 m/s^2");
    assert_eq!(show("10 kg * 9.81 m/s^2 -> N"), "98.1 N");
    assert_eq!(show("10 km + 5 m"), "10.005 km");
    assert_eq!(show("5 m + 10 km"), "10005 m");
    assert_eq!(show("3 m * 4 m"), "12 m^2");
    assert_eq!(show("12 m^2 / 3 m"), "4 m");
    close("2 h -> s", 7200.0, 1e-14);
    close("1 mi -> m", 1609.344, 1e-14);
    close("60 mph -> m/s", 26.8224, 1e-14);
    close("1 atm -> Pa", 101_325.0, 1e-14);
    close("1 kWh -> J", 3.6e6, 1e-14);
    close("1 hp -> W", 745.699_871_582_270_2, 1e-12);
    assert!((disp("1 in -> mm") - 25.4).abs() < 1e-12);
}

#[test]
fn conversion_operators() {
    assert_eq!(show("5 km -> m"), "5000 m");
    assert_eq!(show("5 km to m"), "5000 m");
    assert_eq!(show("1 N -> kg*m/s^2"), "1 kg*m/s^2");
    let e = fails("5 km -> s");
    assert_eq!(e.kind, ErrorKind::Dimension);
    assert!(
        e.message.contains("km") || e.message.contains("length"),
        "{}",
        e.message
    );
    let e = fails("5 km -> furlongs");
    assert_eq!(e.kind, ErrorKind::Name);
}

#[test]
fn temperature_points_and_differences() {
    close("25 degC -> K", 298.15, 1e-14);
    assert!((disp("0 degC -> degF") - 32.0).abs() < 1e-10);
    assert!((disp("100 degC -> degF") - 212.0).abs() < 1e-10);
    assert!(disp("32 degF -> degC").abs() < 1e-10);
    assert!((disp("(-40) degC -> degF") + 40.0).abs() < 1e-10);
    assert!((disp("300 K -> degC") - 26.85).abs() < 1e-10);
    let e = fails("20 degC * 2");
    assert_eq!(e.kind, ErrorKind::Dimension);
}

#[test]
fn dimension_errors_are_actionable() {
    let e = fails("5 m + 3 s");
    assert_eq!(e.kind, ErrorKind::Dimension);
    assert!(
        e.message.contains("length") && e.message.contains("time"),
        "{}",
        e.message
    );
    assert_eq!(e.line_col("5 m + 3 s"), Some((1, 1)));
    assert_eq!(fails("exp(1 m)").kind, ErrorKind::Dimension);
    assert_eq!(fails("ln(2 kg)").kind, ErrorKind::Dimension);
    assert_eq!(fails("sin(2 m)").kind, ErrorKind::Dimension);
    assert_eq!(fails("2^(1 m)").kind, ErrorKind::Dimension);
}

#[test]
fn trig_with_angles() {
    close("sin(30 deg)", 0.5, 1e-15);
    close("cos(60 deg)", 0.5, 1e-15);
    close("tan(45 deg)", 1.0, 1e-15);
    close("sin(pi/2)", 1.0, 1e-15);
    close("cos(pi)", -1.0, 1e-15);
    close("sin(1)", 0.841_470_984_807_896_5, 1e-15);
    close("cos(1)", 0.540_302_305_868_139_8, 1e-15);
    close("tan(1)", 1.557_407_724_654_902_3, 1e-15);
    assert_eq!(show("asin(0.5) -> deg"), "30 deg");
    close("atan2(1, 1)", std::f64::consts::FRAC_PI_4, 1e-15);
    close("atan2(1 m, 1 m)", std::f64::consts::FRAC_PI_4, 1e-15);
    assert_eq!(fails("atan2(1 m, 1 s)").kind, ErrorKind::Dimension);
}

#[test]
fn sqrt_halves_the_dimension() {
    assert_eq!(show("sqrt(9 m^2)"), "3 m");
    assert_eq!(show("sqrt(16 m^2/s^2)"), "4 m/s");
    assert_eq!(show("sqrt(2)^2 > 1.99999"), "true");
    assert_eq!(show("cbrt(27 m^3)"), "3 m");
    // an odd power has a half-integer dimension: allowed, exact rational exponents
    assert_eq!(show("sqrt(4 m)^2"), "4 m");
}

#[test]
fn physical_constants() {
    close("c", 299_792_458.0, 1e-15);
    close("g0", 9.80665, 1e-15);
    close("h_planck", 6.626_070_15e-34, 1e-15);
    close("k_B", 1.380_649e-23, 1e-15);
    close("N_A", 6.022_140_76e23, 1e-15);
    // E = m c^2 for 1 kg, in joules
    close("1 kg * c^2 -> J", 8.987_551_787_368_176e16, 1e-14);
    // `h` is the hour, Planck's constant is `h_planck`
    close("1 h -> s", 3600.0, 1e-15);
    close("e", std::f64::consts::E, 1e-15);
    close("pi", std::f64::consts::PI, 1e-15);
    close("tau", std::f64::consts::TAU, 1e-15);
}

// ---------------------------------------------------------------- numeric functions

#[test]
fn elementary_functions_against_references() {
    close("exp(1)", std::f64::consts::E, 1e-15);
    close("exp(0)", 1.0, 1e-15);
    close("ln(10)", std::f64::consts::LN_10, 1e-15);
    close("ln(e)", 1.0, 1e-15);
    close("log(1000)", 3.0, 1e-15);
    close("log10(0.01)", -2.0, 1e-15);
    close("log2(8)", 3.0, 1e-15);
    close("log(8, 2)", 3.0, 1e-14);
    close("sqrt(2)", std::f64::consts::SQRT_2, 1e-15);
    close("cbrt(27)", 3.0, 1e-15);
    close("cbrt(-8)", -2.0, 1e-15);
    close("sinh(1)", 1.175_201_193_643_801_4, 1e-15);
    close("cosh(1)", 1.543_080_634_815_243_7, 1e-15);
    close("tanh(1)", 0.761_594_155_955_764_9, 1e-15);
    close("asinh(1)", 0.881_373_587_019_543, 1e-14);
    close("acosh(2)", 1.316_957_896_924_816_6, 1e-14);
    close("atanh(0.5)", 0.549_306_144_334_054_8, 1e-14);
    close("asin(1)", std::f64::consts::FRAC_PI_2, 1e-15);
    close("acos(0)", std::f64::consts::FRAC_PI_2, 1e-15);
    close("atan(1)", std::f64::consts::FRAC_PI_4, 1e-15);
    close("hypot(3, 4)", 5.0, 1e-15);
}

#[test]
fn rounding_and_sign() {
    assert_eq!(show("floor(2.7)"), "2");
    assert_eq!(show("floor(-2.1)"), "-3");
    assert_eq!(show("ceil(2.1)"), "3");
    assert_eq!(show("round(2.5)"), "3");
    assert_eq!(show("round(-2.5)"), "-3");
    assert_eq!(show("trunc(-2.9)"), "-2");
    assert_eq!(show("sign(-4)"), "-1");
    assert_eq!(show("sign(0)"), "0");
    assert_eq!(show("abs(-3 m)"), "3 m");
    assert_eq!(show("mod(7, 3)"), "1");
    assert_eq!(show("floor(2.7 m)"), "2 m");
}

#[test]
fn gamma_and_factorial() {
    close("gamma(5)", 24.0, 1e-15);
    close("gamma(0.5)", std::f64::consts::PI.sqrt(), 1e-14);
    close("gamma(1.5)", std::f64::consts::PI.sqrt() / 2.0, 1e-14);
    close("gamma(-0.5)", -2.0 * std::f64::consts::PI.sqrt(), 1e-13);
    close("gamma(10.5)", 1_133_278.388_948_785_8, 1e-13);
    close("factorial(0)", 1.0, 0.0);
    close("factorial(10)", 3_628_800.0, 0.0);
    close("factorial(170)", 7.257_415_615_307_994e306, 1e-14);
    assert_eq!(fails("factorial(171)").kind, ErrorKind::Numeric);
    assert_eq!(fails("factorial(-1)").kind, ErrorKind::Domain);
    assert_eq!(fails("factorial(2.5)").kind, ErrorKind::Domain);
    assert_eq!(fails("gamma(0)").kind, ErrorKind::Domain);
    assert_eq!(fails("gamma(-3)").kind, ErrorKind::Domain);
}

#[test]
fn domain_errors_name_the_function() {
    for (src, needle) in [
        ("ln(-1)", "ln"),
        ("ln(0)", "ln"),
        ("log10(-5)", "log10"),
        ("sqrt(-4)", "sqrt"),
        ("asin(2)", "asin"),
        ("acos(-1.5)", "acos"),
        ("acosh(0.5)", "acosh"),
        ("atanh(1)", "atanh"),
    ] {
        let e = fails(src);
        assert_eq!(e.kind, ErrorKind::Domain, "{src}");
        assert!(e.message.contains(needle), "{src}: {}", e.message);
    }
    assert_eq!(fails("1/0").kind, ErrorKind::Domain);
    assert_eq!(fails("0^-1").kind, ErrorKind::Domain);
    assert_eq!(fails("(-8)^(1/3)").kind, ErrorKind::Domain);
    assert_eq!(fails("10^400").kind, ErrorKind::Numeric);
    assert_eq!(fails("1e300 * 1e300").kind, ErrorKind::Numeric);
}

// ---------------------------------------------------------------- complex numbers

#[test]
fn complex_arithmetic() {
    assert_eq!(complex("(1+2i)*(3-1i)"), (5.0, 5.0));
    assert_eq!(complex("(3+4i) + (1-1i)"), (4.0, 3.0));
    assert_eq!(complex("i*i"), (-1.0, 0.0));
    assert_eq!(complex("(1+i)/(1-i)"), (0.0, 1.0));
    assert_eq!(complex("conj(3+4i)"), (3.0, -4.0));
    assert_eq!(complex("sqrt(-4+0i)"), (0.0, 2.0));
    assert_eq!(show("abs(3+4i)"), "5");
    assert_eq!(show("re(3+4i)"), "3");
    assert_eq!(show("im(3+4i)"), "4");
    close("arg(i)", std::f64::consts::FRAC_PI_2, 1e-15);
    close("arg(-1+0i)", std::f64::consts::PI, 1e-15);
    assert_eq!(show("3 + 4i"), "3 + 4i");
    assert_eq!(show("2i"), "2i");
    assert_eq!(show("complex(1, -1)"), "1 - i");
}

#[test]
fn eulers_identity() {
    let (re, im) = complex("exp(i*pi)");
    assert!((re + 1.0).abs() < 1e-15, "{re}");
    assert!(im.abs() < 2e-16, "{im}");
    let (re, im) = complex("exp(i*pi/2)");
    assert!(re.abs() < 1e-15 && (im - 1.0).abs() < 1e-15);
    let (re, im) = complex("i^i");
    assert!((re - (-std::f64::consts::FRAC_PI_2).exp()).abs() < 1e-15 && im.abs() < 1e-15);
}

#[test]
fn complex_with_units_is_rejected() {
    assert_eq!(fails("(1+2i) * 3 m").kind, ErrorKind::Dimension);
}

// ---------------------------------------------------------------- vectors and matrices

#[test]
fn vectors_and_indexing() {
    let mut env = Env::new();
    run_in("v = [10, 20, 30, 40]", &mut env).unwrap();
    assert_eq!(run_in("v[1]", &mut env).unwrap().to_string(), "10");
    assert_eq!(run_in("v[4]", &mut env).unwrap().to_string(), "40");
    assert_eq!(run_in("v[2:3]", &mut env).unwrap().to_string(), "[20, 30]");
    assert_eq!(
        run_in("v[:]", &mut env).unwrap().to_string(),
        "[10, 20, 30, 40]"
    );
    assert_eq!(
        run_in("v * 2", &mut env).unwrap().to_string(),
        "[20, 40, 60, 80]"
    );
    assert_eq!(
        run_in("v + v", &mut env).unwrap().to_string(),
        "[20, 40, 60, 80]"
    );
    assert_eq!(
        run_in("v .* v", &mut env).unwrap().to_string(),
        "[100, 400, 900, 1600]"
    );
    assert_eq!(
        run_in("v ./ 10", &mut env).unwrap().to_string(),
        "[1, 2, 3, 4]"
    );
    assert_eq!(
        run_in("[1,2,3] .^ 2", &mut env).unwrap().to_string(),
        "[1, 4, 9]"
    );
    assert_eq!(
        run_in("[1 m, 2 m] * 3", &mut env).unwrap().to_string(),
        "[3 m, 6 m]"
    );
    assert_eq!(run_in("sum(v)", &mut env).unwrap().to_string(), "100");
    assert_eq!(run_in("length(v)", &mut env).unwrap().to_string(), "4");
    assert_eq!(
        run_in("[v, 50]", &mut env).unwrap().to_string(),
        "[10, 20, 30, 40, 50]"
    );
    for bad in ["v[0]", "v[5]", "v[1.5]", "v[-1]"] {
        let e = run_in(bad, &mut env).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Index, "{bad}");
    }
    let e = run_in("v[0]", &mut env).unwrap_err();
    assert!(e.message.contains("start at 1"), "{}", e.message);
    let e = run_in("v[5]", &mut env).unwrap_err();
    assert!(e.message.contains("length is 4"), "{}", e.message);
    // the span of an index error is the index expression
    assert_eq!(e.span.map(|s| (s.start, s.end)), Some((2, 3)));
}

#[test]
fn matrices() {
    let mut env = Env::new();
    run_in("A = [1, 2; 3, 4]", &mut env).unwrap();
    assert_eq!(run_in("A[1,2]", &mut env).unwrap().to_string(), "2");
    assert_eq!(run_in("A[2,1]", &mut env).unwrap().to_string(), "3");
    assert_eq!(run_in("A[2,:]", &mut env).unwrap().to_string(), "[3, 4]");
    assert_eq!(run_in("A[:,2]", &mut env).unwrap().to_string(), "[2, 4]");
    assert_eq!(
        run_in("A[1:2,1:2]", &mut env).unwrap().to_string(),
        "[1, 2; 3, 4]"
    );
    assert_eq!(
        run_in("A * A", &mut env).unwrap().to_string(),
        "[7, 10; 15, 22]"
    );
    assert_eq!(
        run_in("A .* A", &mut env).unwrap().to_string(),
        "[1, 4; 9, 16]"
    );
    assert_eq!(
        run_in("A ^ 2", &mut env).unwrap().to_string(),
        "[7, 10; 15, 22]"
    );
    assert_eq!(
        run_in("A + 1", &mut env).unwrap().to_string(),
        "[2, 3; 4, 5]"
    );
    assert_eq!(
        run_in("transpose(A)", &mut env).unwrap().to_string(),
        "[1, 3; 2, 4]"
    );
    assert_eq!(
        run_in("A * [1; 1]", &mut env).unwrap().to_string(),
        "[3; 7]"
    );
    assert_eq!(
        run_in("A * [1, 1]", &mut env).unwrap().to_string(),
        "[3, 7]"
    );
    assert_eq!(run_in("det(A)", &mut env).unwrap().to_string(), "-2");
    assert_eq!(run_in("size(A)", &mut env).unwrap().to_string(), "[2, 2]");
    assert_eq!(
        run_in("eye(2)", &mut env).unwrap().to_string(),
        "[1, 0; 0, 1]"
    );
    assert_eq!(
        run_in("zeros(2, 3)", &mut env).unwrap().to_string(),
        "[0, 0, 0; 0, 0, 0]"
    );
    assert_eq!(run_in("ones(2)", &mut env).unwrap().to_string(), "[1, 1]");
    assert_eq!(
        run_in("ones(2, 2)", &mut env).unwrap().to_string(),
        "[1, 1; 1, 1]"
    );
    assert!(run_in("A[3,1]", &mut env).is_err());
    assert!(run_in("A[1]", &mut env).is_err());
    assert_eq!(
        run_in("A[1,1:2] * 2", &mut env).unwrap().to_string(),
        "[2, 4]"
    );
    assert!(run_in("A * [1, 2, 3]", &mut env).is_err());
    assert!(run_in("[1,2;3]", &mut env).is_err());
}

#[test]
fn linear_algebra_against_known_answers() {
    // det and inverse of a 3x3 with a hand-computed answer
    let mut env = Env::new();
    run_in("M = [2, -1, 0; -1, 2, -1; 0, -1, 2]", &mut env).unwrap();
    let d = run_in("det(M)", &mut env).unwrap();
    assert!(
        matches!(d, Value::Real(x) if (x - 4.0).abs() < 1e-12),
        "{d}"
    );
    // inv(M) = 1/4 * [3 2 1; 2 4 2; 1 2 3]
    let inv = run_in("inv(M)", &mut env).unwrap();
    let want = [0.75, 0.5, 0.25, 0.5, 1.0, 0.5, 0.25, 0.5, 0.75];
    let Value::Matrix(m) = inv else {
        panic!("not a matrix")
    };
    for (got, w) in m.data.iter().zip(want) {
        let Value::Real(g) = got else { panic!() };
        assert!((g - w).abs() < 1e-12, "{g} vs {w}");
    }
    // solve: M x = [1; 0; 1] has x = [1; 1; 1]
    let x = run_in("solve(M, [1; 0; 1])", &mut env).unwrap();
    let Value::Matrix(v) = x else {
        panic!("not a matrix: {x}")
    };
    assert_eq!((v.rows, v.cols), (3, 1));
    for e in v.data {
        let Value::Real(g) = e else { panic!() };
        assert!((g - 1.0).abs() < 1e-12);
    }
    // A * inv(A) = I
    let p = run_in("M * inv(M)", &mut env).unwrap();
    let Value::Matrix(m) = p else { panic!() };
    for (k, e) in m.data.iter().enumerate() {
        let Value::Real(g) = e else { panic!() };
        let w = if k % 4 == 0 { 1.0 } else { 0.0 };
        assert!((g - w).abs() < 1e-12);
    }
}

#[test]
fn singular_matrices_are_reported() {
    let e = fails("inv([1, 2; 2, 4])");
    assert_eq!(e.kind, ErrorKind::Numeric);
    assert!(
        e.message.to_lowercase().contains("singular"),
        "{}",
        e.message
    );
    assert_eq!(
        fails("solve([1, 2; 2, 4], [1; 2])").kind,
        ErrorKind::Numeric
    );
    assert_eq!(fails("det([1, 2, 3; 4, 5, 6])").kind, ErrorKind::Index);
}

#[test]
fn vector_functions() {
    assert_eq!(show("dot([1,2,3], [4,5,6])"), "32");
    assert_eq!(show("cross([1,0,0], [0,1,0])"), "[0, 0, 1]");
    assert_eq!(show("norm([3,4])"), "5");
    assert_eq!(show("norm([3 m, 4 m])"), "5 m");
    assert_eq!(show("sum([1,2,3,4])"), "10");
    assert_eq!(show("prod([1,2,3,4])"), "24");
    assert_eq!(show("mean([1,2,3,4])"), "2.5");
    assert_eq!(show("median([3,1,2])"), "2");
    assert_eq!(show("median([4,1,3,2])"), "2.5");
    assert_eq!(show("min(3, 1, 2)"), "1");
    assert_eq!(show("max([3, 9, 2])"), "9");
    assert_eq!(show("max(1 m, 50 cm)"), "1 m");
    close("var([2,4,4,4,5,5,7,9])", 32.0 / 7.0, 1e-14);
    close("std([2,4,4,4,5,5,7,9])", (32.0f64 / 7.0).sqrt(), 1e-14);
    assert_eq!(show("linspace(0, 1, 5)"), "[0, 0.25, 0.5, 0.75, 1]");
    assert_eq!(show("logspace(0, 3, 4)"), "[1, 10, 100, 1000]");
    assert_eq!(show("range(1, 4)"), "[1, 2, 3, 4]");
    assert_eq!(show("1:5"), "[1, 2, 3, 4, 5]");
    assert_eq!(show("1:2:10"), "[1, 3, 5, 7, 9]");
    assert_eq!(show("10:-3:0"), "[10, 7, 4, 1]");
    assert_eq!(show("0:0.25:1"), "[0, 0.25, 0.5, 0.75, 1]");
    assert_eq!(fails("1:0:5").kind, ErrorKind::Domain);
    assert_eq!(fails("1:1e9").kind, ErrorKind::Limit);
    assert!(run("max([])").is_err());
}

// ---------------------------------------------------------------- statements and functions

#[test]
fn assignment_and_use() {
    let mut env = Env::new();
    assert_eq!(run_in("m1 = 3 kg", &mut env).unwrap().to_string(), "3 kg");
    assert_eq!(
        run_in("a = 2 m/s^2", &mut env).unwrap().to_string(),
        "2 m/s^2"
    );
    assert_eq!(
        run_in("F = m1 * a", &mut env).unwrap().to_string(),
        "6 kg*m/s^2"
    );
    assert_eq!(run_in("F -> N", &mut env).unwrap().to_string(), "6 N");
    // statements separated by ; or newlines; the last value is the result
    assert_eq!(
        run_in("p = 2; q = 3; p * q", &mut env).unwrap().to_string(),
        "6"
    );
    assert_eq!(run_in("r = 5\nr + 1", &mut env).unwrap().to_string(), "6");
}

#[test]
fn failed_cell_changes_nothing() {
    let mut env = Env::new();
    run_in("keep = 1", &mut env).unwrap();
    let e = eval_str("keep = 2; fresh = 3; 1/0", &mut env).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Domain);
    assert_eq!(env.var("keep"), Some(&Value::Real(1.0)));
    assert!(env.var("fresh").is_none());
}

#[test]
fn user_functions() {
    let mut env = Env::new();
    assert!(eval_str("f(x, y) = x^2 + y", &mut env)
        .unwrap()
        .value
        .is_none());
    assert_eq!(run_in("f(3, 1)", &mut env).unwrap().to_string(), "10");
    assert_eq!(
        run_in("f(2 m, 1 m^2)", &mut env).unwrap().to_string(),
        "5 m^2"
    );
    // an error inside a body is reported at the call, naming the function
    let e = run_in("f([1,2], 1)", &mut env).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Type);
    assert!(e.message.starts_with("in `f`:"), "{}", e.message);
    assert_eq!(e.span.map(|s| (s.start, s.end)), Some((0, 11)));
    run_in("h2(v) = v .^ 2 + 1", &mut env).unwrap();
    assert_eq!(run_in("h2([1,2])", &mut env).unwrap().to_string(), "[2, 5]");
    // a function calling a function
    run_in("g(x) = f(x, 1) * 2", &mut env).unwrap();
    assert_eq!(run_in("g(3)", &mut env).unwrap().to_string(), "20");
    // arity
    let e = run_in("f(1)", &mut env).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Arity);
    assert!(e.message.contains("f(x, y)"), "{}", e.message);
    // a function is not a value
    let e = run_in("f", &mut env).unwrap_err();
    assert!(e.message.contains("function"), "{}", e.message);
    // built-ins cannot be redefined
    assert_eq!(
        run_in("sin(x) = x", &mut env).unwrap_err().kind,
        ErrorKind::Name
    );
    let e = run_in("h(x, x) = x", &mut env).unwrap_err();
    assert!(e.message.contains("appears twice"), "{}", e.message);
}

#[test]
fn functions_capture_their_definition() {
    let mut env = Env::new();
    run_in("k = 10", &mut env).unwrap();
    run_in("scale(x) = k * x", &mut env).unwrap();
    run_in("k = 99", &mut env).unwrap();
    assert_eq!(run_in("scale(2)", &mut env).unwrap().to_string(), "20");
    // a parameter shadows a global of the same name
    run_in("sq(k) = k^2", &mut env).unwrap();
    assert_eq!(run_in("sq(3)", &mut env).unwrap().to_string(), "9");
    // a name that did not exist at definition time is an error at the call
    run_in("late(x) = x + later", &mut env).unwrap();
    let e = run_in("late(1)", &mut env).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Name);
    assert!(e.message.contains("later"), "{}", e.message);
}

#[test]
fn recursion_works_and_is_bounded() {
    let mut env = Env::new();
    run_in("fact(n) = if(n <= 1, 1, n * fact(n - 1))", &mut env).unwrap();
    assert_eq!(run_in("fact(10)", &mut env).unwrap().to_string(), "3628800");
    run_in("fib(n) = if(n < 2, n, fib(n-1) + fib(n-2))", &mut env).unwrap();
    assert_eq!(run_in("fib(15)", &mut env).unwrap().to_string(), "610");
    run_in("forever(n) = forever(n + 1)", &mut env).unwrap();
    let e = run_in("forever(0)", &mut env).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    // the error is reported at the call, not inside the body's source positions
    assert_eq!(e.span.map(|s| s.start), Some(0));
}

#[test]
fn exponential_work_hits_the_step_budget() {
    let mut env = Env::new();
    run_in("fib(n) = if(n < 2, n, fib(n-1) + fib(n-2))", &mut env).unwrap();
    let start = std::time::Instant::now();
    let e = run_in("fib(45)", &mut env).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    assert!(start.elapsed().as_secs() < 20, "took {:?}", start.elapsed());
}

// ---------------------------------------------------------------- errors with positions

#[test]
fn unknown_names_suggest_a_fix() {
    let e = fails("sinn(2)");
    assert_eq!(e.kind, ErrorKind::Name);
    assert!(
        e.message.contains("did you mean `sin`") || e.message.contains("did you mean `sign`"),
        "{}",
        e.message
    );
    let e = fails("Sin(1)");
    assert!(e.message.contains("`sin`"), "{}", e.message);
    let e = fails("1 + sqr(4)");
    assert!(e.message.contains("sqrt"), "{}", e.message);
    let mut env = Env::new();
    run_in("velocity = 3 m/s", &mut env).unwrap();
    let e = run_in("velocty * 2 s", &mut env).unwrap_err();
    assert!(e.message.contains("velocity"), "{}", e.message);
    assert_eq!(e.span.map(|s| (s.start, s.end)), Some((0, 7)));
    let e = fails("foo");
    assert!(e.message.contains("unknown name `foo`"), "{}", e.message);
    let e = fails("2 kgg");
    assert!(e.message.contains("kg"), "{}", e.message);
}

#[test]
fn error_positions_are_line_and_column() {
    let src = "1 + 2\n3 * (4 +\n";
    let e = eval_str(src, &mut Env::new()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Syntax);
    assert_eq!(e.line_col(src).map(|(l, _)| l), Some(3));

    let src = "x = 1\ny = 2 +* 3";
    let e = eval_str(src, &mut Env::new()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Syntax);
    assert_eq!(e.line_col(src), Some((2, 8)));

    let src = "a = 1\nb = 1 / 0";
    let e = eval_str(src, &mut Env::new()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Domain);
    assert_eq!(e.line_col(src), Some((2, 5)));
    let shown = e.render(src);
    assert!(shown.contains("2:5") && shown.contains("^^^^^"), "{shown}");

    // columns count characters, not bytes
    let src = "\"ñ\" + 1";
    let e = eval_str(src, &mut Env::new()).unwrap_err();
    assert_eq!(e.line_col(src), Some((1, 1)));
    let src = "ñ + 1";
    let e = eval_str(src, &mut Env::new()).unwrap_err();
    assert_eq!(e.line_col(src), Some((1, 1)));
}

#[test]
fn syntax_errors() {
    for (src, kind) in [
        ("1 +", ErrorKind::Syntax),
        ("(1", ErrorKind::Syntax),
        ("1)", ErrorKind::Syntax),
        ("[1, 2", ErrorKind::Syntax),
        ("1 $ 2", ErrorKind::Syntax),
        ("\"abc", ErrorKind::Syntax),
        ("1..2", ErrorKind::Syntax),
        ("= 3", ErrorKind::Syntax),
        ("3 = x", ErrorKind::Syntax),
        ("f(,)", ErrorKind::Syntax),
        ("and", ErrorKind::Syntax),
    ] {
        let e = fails(src);
        assert_eq!(e.kind, kind, "{src}: {}", e.message);
        assert!(e.span.is_some(), "{src} has no span");
    }
    let empty = eval_str("  # nothing\n", &mut Env::new()).unwrap();
    assert!(empty.value.is_none());
}

#[test]
fn deep_nesting_is_rejected_not_crashed() {
    let deep = format!("{}1{}", "(".repeat(5_000), ")".repeat(5_000));
    assert_eq!(fails(&deep).kind, ErrorKind::Limit);
    let deep = format!("{}1", "-".repeat(5_000));
    assert_eq!(fails(&deep).kind, ErrorKind::Limit);
    let deep = format!("{}1", "1+".repeat(5_000));
    assert_eq!(fails(&deep).kind, ErrorKind::Limit);
    let deep = format!("{}1{}", "[".repeat(2_000), "]".repeat(2_000));
    assert_eq!(fails(&deep).kind, ErrorKind::Limit);
    let deep = format!("{}x{}", "abs(".repeat(5_000), ")".repeat(5_000));
    assert_eq!(fails(&deep).kind, ErrorKind::Limit);
}

#[test]
fn huge_sources_and_sizes_are_limited() {
    let big = "1+".repeat(60_000) + "1";
    assert_eq!(fails(&big).kind, ErrorKind::Limit);
    assert_eq!(fails("zeros(1000000)").kind, ErrorKind::Limit);
    assert_eq!(fails("zeros(400, 400)").kind, ErrorKind::Limit);
    assert_eq!(fails("linspace(0, 1, 1e9)").kind, ErrorKind::Limit);
    assert_eq!(fails("eye(1e6)").kind, ErrorKind::Limit);
}

#[test]
fn json_output_describes_values() {
    let v = run("9.81 m/s^2").unwrap().to_json(6);
    assert_eq!(v["type"], "quantity");
    assert_eq!(v["unit"], "m/s^2");
    assert_eq!(v["text"], "9.81 m/s^2");
    let v = run("[1, 2; 3, 4]").unwrap().to_json(6);
    assert_eq!(v["type"], "matrix");
    assert_eq!(v["rows"], 2);
    let v = run("3 + 4i").unwrap().to_json(6);
    assert_eq!(v["re"], 3.0);
    assert_eq!(v["im"], 4.0);
}
