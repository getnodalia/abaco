// SPDX-License-Identifier: MIT OR Apache-2.0
//! Control-systems functions against closed forms and published reference values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::Interp;

fn run(src: &str) -> Interp {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i
}

fn get(i: &Interp, name: &str) -> Vec<f64> {
    i.get(name).unwrap().to_real().unwrap().d
}

fn real(src: &str) -> Vec<f64> {
    get(&run(src), "ans")
}

fn scalar(src: &str) -> f64 {
    let v = real(src);
    assert_eq!(v.len(), 1, "{src} gave {v:?}");
    v[0]
}

fn near(a: f64, b: f64, tol: f64) {
    assert!(
        (a - b).abs() <= tol * b.abs().max(1.0),
        "{a} vs {b} (tol {tol})"
    );
}

fn fails(src: &str) -> String {
    let mut i = Interp::new();
    match i.run(src) {
        Ok(()) => panic!("`{src}` should have failed"),
        Err(e) => e.message,
    }
}

#[test]
fn first_order_step_matches_closed_form() {
    let i = run("g = tf(1,[1 1]); t = linspace(0,5,51)'; y = step(g,t);");
    let (y, t) = (get(&i, "y"), get(&i, "t"));
    assert_eq!(y.len(), t.len());
    for (yk, tk) in y.iter().zip(&t) {
        near(*yk, 1.0 - (-tk).exp(), 1e-6);
    }
}

#[test]
fn first_order_impulse_matches_closed_form() {
    let i = run("g = tf(2,[1 2]); t = linspace(0,2,21)'; y = impulse(g,t);");
    let (y, t) = (get(&i, "y"), get(&i, "t"));
    for (yk, tk) in y.iter().zip(&t) {
        near(*yk, 2.0 * (-2.0 * tk).exp(), 1e-5);
    }
}

#[test]
fn poles_zeros_and_dc_gain() {
    let i = run("g = tf([1 3],[1 3 2]); p = pole(g); z = zero(g); k = dcgain(g);");
    let mut p = get(&i, "p");
    p.sort_by(f64::total_cmp);
    near(p[0], -2.0, 1e-9);
    near(p[1], -1.0, 1e-9);
    near(get(&i, "z")[0], -3.0, 1e-9);
    near(get(&i, "k")[0], 1.5, 1e-9);
}

#[test]
fn tf_algebra_series_parallel_feedback() {
    // series: 1/(s+1) * 1/(s+2) = 1/(s^2+3s+2)
    let i = run("g = series(tf(1,[1 1]), tf(1,[1 2])); [n,d] = tfdata(g);");
    assert_eq!(get(&i, "n"), vec![1.0]);
    assert_eq!(get(&i, "d"), vec![1.0, 3.0, 2.0]);
    // parallel: 1/(s+1) + 1/(s+2) = (2s+3)/(s^2+3s+2)
    let i = run("g = parallel(tf(1,[1 1]), tf(1,[1 2])); [n,d] = tfdata(g);");
    assert_eq!(get(&i, "n"), vec![2.0, 3.0]);
    assert_eq!(get(&i, "d"), vec![1.0, 3.0, 2.0]);
    // negative feedback: (1/s)/(1+1/s) = 1/(s+1)
    let i = run("g = feedback(tf(1,[1 0]), 1); [n,d] = tfdata(g);");
    assert_eq!(get(&i, "n"), vec![1.0]);
    assert_eq!(get(&i, "d"), vec![1.0, 1.0]);
}

#[test]
fn bode_of_a_first_order_lag_at_its_corner() {
    let i = run("g = tf(1,[1 1]); [m,p,w] = bode(g,[0.01 1 100]);");
    let (m, p) = (get(&i, "m"), get(&i, "p"));
    near(20.0 * m[1].log10(), -10.0 * 2f64.log10(), 1e-9);
    near(p[1], -45.0, 1e-9);
    near(m[0], 1.0, 1e-3);
    near(p[2], -89.427, 1e-3);
}

#[test]
fn margins_of_a_third_order_loop() {
    // G = 1/(s(s+1)(s+2)): phase crosses -180 at w = sqrt 2 where |G| = 1/6, so Gm = 6.
    let i = run("[gm,pm,wcg,wcp] = margin(tf(1,[1 3 2 0]));");
    near(get(&i, "gm")[0], 6.0, 1e-6);
    near(get(&i, "wcg")[0], 2f64.sqrt(), 1e-6);
    // Gain crossover of 1/(w sqrt(w^2+1) sqrt(w^2+4)) = 1 is w = 0.4465, phase margin about 53.4 deg.
    near(get(&i, "wcp")[0], 0.446_5, 1e-3);
    near(get(&i, "pm")[0], 53.4, 5e-3);
}

#[test]
fn stepinfo_of_a_second_order_system() {
    let zeta: f64 = 0.3;
    let wn: f64 = 2.0;
    let i = run("g = tf(4,[1 1.2 4]); s = stepinfo(g); os = s.Overshoot; tp = s.PeakTime; ss = s.SteadyState;");
    near(
        get(&i, "os")[0],
        100.0 * (-std::f64::consts::PI * zeta / (1.0 - zeta * zeta).sqrt()).exp(),
        2e-2,
    );
    near(
        get(&i, "tp")[0],
        std::f64::consts::PI / (wn * (1.0 - zeta * zeta).sqrt()),
        2e-2,
    );
    near(get(&i, "ss")[0], 1.0, 1e-9);
}

#[test]
fn damp_reports_natural_frequency_and_damping() {
    let i = run("[wn,z] = damp(tf(4,[1 1.2 4]));");
    near(get(&i, "wn")[0], 2.0, 1e-9);
    near(get(&i, "z")[0], 0.3, 1e-9);
}

#[test]
fn ss_tf_round_trip() {
    let i = run("[A,B,C,D] = tf2ss([1 3],[1 3 2]); [n,d] = ss2tf(A,B,C,D); g = ss(A,B,C,D); [n2,d2] = ss2tf(g);");
    let (n, d) = (get(&i, "n"), get(&i, "d"));
    for (a, b) in n.iter().zip([1.0, 3.0]) {
        near(*a, b, 1e-9);
    }
    for (a, b) in d.iter().zip([1.0, 3.0, 2.0]) {
        near(*a, b, 1e-9);
    }
    assert_eq!(get(&i, "n2").len(), n.len());
    near(get(&i, "d2")[1], 3.0, 1e-9);
}

#[test]
fn pole_placement_puts_the_closed_loop_poles_where_asked() {
    let i = run("A = [0 1; 0 0]; B = [0; 1]; K = place(A,B,[-1 -2]); e = sort(eig(A-B*K));");
    let k = get(&i, "K");
    near(k[0], 2.0, 1e-9);
    near(k[1], 3.0, 1e-9);
    let e = get(&i, "e");
    near(e[0], -2.0, 1e-9);
    near(e[1], -1.0, 1e-9);
}

#[test]
fn lqr_of_a_double_integrator_matches_the_known_gain() {
    // For x'' = u with Q = I, R = 1 the optimal gain is [1 sqrt 3].
    let i = run("[K,S,e] = lqr([0 1;0 0],[0;1],eye(2),1);");
    let k = get(&i, "K");
    near(k[0], 1.0, 1e-8);
    near(k[1], 3f64.sqrt(), 1e-8);
}

#[test]
fn riccati_and_lyapunov_residuals_vanish() {
    let r = scalar(
        "A=[0 1;-2 -3]; B=[0;1]; Q=eye(2); R=1; X=care(A,B,Q,R); ans = norm(A'*X+X*A-X*B*(R\\B')*X+Q);",
    );
    assert!(r < 1e-8, "care residual {r}");
    let r = scalar("A=[-1 2;0 -3]; Q=[2 1;1 2]; X=lyap(A,Q); ans = norm(A*X+X*A'+Q);");
    assert!(r < 1e-9, "lyap residual {r}");
    let r = scalar("A=[0.5 0.1;0 0.3]; Q=eye(2); X=dlyap(A,Q); ans = norm(A*X*A'-X+Q);");
    assert!(r < 1e-9, "dlyap residual {r}");
}

#[test]
fn zero_order_hold_matches_the_closed_form() {
    // 1/(s+1) sampled at Ts gives (1-e^-Ts)/(z-e^-Ts).
    let i = run("d = c2d(tf(1,[1 1]),0.1,'zoh'); [n,den] = tfdata(d);");
    let a = (-0.1f64).exp();
    let (n, den) = (get(&i, "n"), get(&i, "den"));
    near(*n.last().unwrap(), 1.0 - a, 1e-9);
    near(den[0], 1.0, 1e-9);
    near(den[1], -a, 1e-9);
}

#[test]
fn lsim_of_a_ramp_through_an_integrator() {
    let i = run("t = (0:0.1:2)'; y = lsim(tf(1,[1 0]), t, t);");
    let (y, t) = (get(&i, "y"), get(&i, "t"));
    for (yk, tk) in y.iter().zip(&t) {
        near(*yk, tk * tk / 2.0, 1e-3);
    }
}

#[test]
fn root_locus_starts_at_the_poles() {
    let i = run("[r,k] = rlocus(tf(1,[1 3 2]),[0 1 10]);");
    let r = i.get("r").unwrap().to_complex().unwrap();
    let mut first: Vec<f64> = (0..2).map(|j| r.d[j].re).collect();
    first.sort_by(f64::total_cmp);
    near(first[0], -2.0, 1e-9);
    near(first[1], -1.0, 1e-9);
}

#[test]
fn stability_check() {
    assert_eq!(scalar("ans = isstable(tf(1,[1 2 1]));"), 1.0);
    assert_eq!(scalar("ans = isstable(tf(1,[1 -1]));"), 0.0);
}

#[test]
fn hostile_arguments_are_errors_not_panics() {
    for src in [
        "tf(1,[0 0])",
        "step(tf([1 2],[1]))",
        "step(5)",
        "step(tf(1,[1 1]),[0 0 0])",
        "step(tf(1,[1 1]),NaN)",
        "bode(tf(1,[1 1]),-1)",
        "place([1 2;3 4],[1;1],[1 2 3])",
        "lqr([0 1;0 0],[0;1],-eye(2),1)",
        "lyap(1,[1 2])",
        "c2d(tf(1,[1 1]),-1)",
        "c2d(tf(1,[1 1]),0.1,'nope')",
        "lsim(tf(1,[1 1]),[1 2],[0 1 2])",
        "pid(1,1,1,0)",
        "ss(zeros(2,3),1,1,0)",
    ] {
        let msg = fails(src);
        assert!(!msg.is_empty(), "{src}");
    }
}
