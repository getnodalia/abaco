// SPDX-License-Identifier: MIT OR Apache-2.0
//! Optimisation and ODE solvers against closed forms and published reference values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use abaco_script::Interp;

fn run(src: &str) -> Interp {
    let mut i = Interp::new();
    i.run(src).unwrap_or_else(|e| panic!("{}", e.render(src)));
    i
}

fn real(src: &str) -> Vec<f64> {
    run(src).get("ans").unwrap().to_real().unwrap().d
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
fn fminsearch_finds_rosenbrock_minimum() {
    let x = real("opts = optimset('TolX',1e-10,'TolFun',1e-10,'MaxIter',2000,'MaxFunEvals',4000); ans = fminsearch(@(p) 100*(p(2)-p(1)^2)^2 + (1-p(1))^2, [-1.2 1], opts);");
    near(x[0], 1.0, 1e-4);
    near(x[1], 1.0, 1e-4);
}

#[test]
fn fminunc_and_fminbnd_match_closed_forms() {
    let x = real("ans = fminunc(@(p) (p(1)-3)^2 + 2*(p(2)+1)^2, [0 0]);");
    near(x[0], 3.0, 1e-4);
    near(x[1], -1.0, 1e-4);
    near(
        scalar("ans = fminbnd(@cos, 3, 4);"),
        std::f64::consts::PI,
        1e-5,
    );
    let i = run("[x, fv, flag] = fminbnd(@(x) (x-2)^2 + 1, 0, 5);");
    near(i.get("fv").unwrap().scalar_f64("").unwrap(), 1.0, 1e-8);
    near(i.get("flag").unwrap().scalar_f64("").unwrap(), 1.0, 0.0);
}

#[test]
fn fsolve_solves_a_nonlinear_system() {
    // x^2 + y^2 = 4, x*y = 1  ->  x = sqrt(2+sqrt3), y = sqrt(2-sqrt3) from this start.
    let x = real("ans = fsolve(@(p) [p(1)^2+p(2)^2-4; p(1)*p(2)-1], [2 0.5]);");
    near(x[0], (2.0 + 3f64.sqrt()).sqrt(), 1e-6);
    near(x[1], (2.0 - 3f64.sqrt()).sqrt(), 1e-6);
    let i = run("[x, fv, flag] = fsolve(@(x) x^3 - 8, 1);");
    near(i.get("x").unwrap().scalar_f64("").unwrap(), 2.0, 1e-6);
    near(i.get("flag").unwrap().scalar_f64("").unwrap(), 1.0, 0.0);
}

#[test]
fn fsolve_flags_a_system_with_no_root() {
    let i = run("[x, fv, flag] = fsolve(@(x) x^2 + 1, 1);");
    assert!(i.get("flag").unwrap().scalar_f64("").unwrap() <= 0.0);
}

#[test]
fn lsqcurvefit_recovers_exponential_parameters() {
    let i = run("xd = 0:0.5:5; yd = 3*exp(-0.7*xd) + 0.25; \
         p = lsqcurvefit(@(p,x) p(1)*exp(-p(2)*x)+p(3), [1 1 0], xd, yd); \
         [q, rn] = lsqnonlin(@(p) p(1)*exp(-p(2)*xd)+p(3)-yd, [1 1 0]);");
    let p = i.get("p").unwrap().to_real().unwrap().d;
    near(p[0], 3.0, 1e-5);
    near(p[1], 0.7, 1e-5);
    near(p[2], 0.25, 1e-5);
    let rn = i.get("rn").unwrap().scalar_f64("").unwrap();
    assert!(rn < 1e-10, "{rn}");
}

#[test]
fn linprog_solves_textbook_programs() {
    // max 3x+5y s.t. x<=4, 2y<=12, 3x+2y<=18 (Hillier & Lieberman): optimum 36 at (2,6).
    let i = run("[x, fv, flag] = linprog([-3 -5], [1 0; 0 2; 3 2], [4; 12; 18]);");
    let x = i.get("x").unwrap().to_real().unwrap().d;
    near(x[0], 2.0, 1e-9);
    near(x[1], 6.0, 1e-9);
    near(i.get("fv").unwrap().scalar_f64("").unwrap(), -36.0, 1e-9);
    near(i.get("flag").unwrap().scalar_f64("").unwrap(), 1.0, 0.0);
    let infeasible = run("[x, fv, flag] = linprog([1 1], [1 1; -1 -1], [1; -3]);");
    near(
        infeasible.get("flag").unwrap().scalar_f64("").unwrap(),
        -2.0,
        0.0,
    );
    let unbounded = run("[x, fv, flag] = linprog([-1 0], [], []);");
    near(
        unbounded.get("flag").unwrap().scalar_f64("").unwrap(),
        -3.0,
        0.0,
    );
}

#[test]
fn ode45_matches_the_closed_form_at_requested_times() {
    let i = run(
        "[t, y] = ode45(@(t,y) -2*y, [0 0.5 1 2], 1, odeset('RelTol',1e-9,'AbsTol',1e-12)); r = y';",
    );
    let r = i.get("r").unwrap().to_real().unwrap().d;
    for (k, tk) in [0.0f64, 0.5, 1.0, 2.0].iter().enumerate() {
        near(r[k], (-2.0 * tk).exp(), 1e-7);
    }
    assert_eq!(
        i.get("t").unwrap().to_real().unwrap().d,
        vec![0.0, 0.5, 1.0, 2.0]
    );
}

#[test]
fn ode45_with_a_two_point_span_reports_every_step() {
    let i = run("[t, y] = ode45(@(t,y) [y(2); -y(1)], [0 pi], [0; 1]); n = numel(t); e = y(end,1); s = size(y);");
    assert!(i.get("n").unwrap().scalar_f64("").unwrap() > 5.0);
    near(i.get("e").unwrap().scalar_f64("").unwrap(), 0.0, 5e-3);
    let s = i.get("s").unwrap().to_real().unwrap().d;
    assert_eq!(s[1], 2.0);
}

#[test]
fn ode_struct_output_and_events_stop_a_falling_ball() {
    let i = run(
        "function [v, term, dir] = ground(t, y)\n v = y(1); term = 1; dir = -1;\nend\n\
         opts = odeset('Events', @ground, 'RelTol', 1e-9, 'AbsTol', 1e-11);\n\
         [t, y, te, ye, ie] = ode45(@(t,y) [y(2); -9.81], [0 10], [10; 0], opts);\n\
         tl = t(end);",
    );
    let want = (20.0f64 / 9.81).sqrt();
    near(i.get("te").unwrap().scalar_f64("").unwrap(), want, 1e-8);
    near(i.get("tl").unwrap().scalar_f64("").unwrap(), want, 1e-8);
    near(i.get("ie").unwrap().scalar_f64("").unwrap(), 1.0, 0.0);
    let ye = i.get("ye").unwrap().to_real().unwrap().d;
    assert!(ye[0].abs() < 1e-7);
    let s = run("sol = ode45(@(t,y) -y, [0 1], 1); a = sol.x(end); b = sol.y(end);");
    near(s.get("a").unwrap().scalar_f64("").unwrap(), 1.0, 1e-12);
    near(
        s.get("b").unwrap().scalar_f64("").unwrap(),
        (-1.0f64).exp(),
        1e-3,
    );
}

#[test]
fn stiff_solvers_handle_robertson_and_a_mass_matrix_dae() {
    let i = run(
        "f = @(t,y) [-0.04*y(1)+1e4*y(2)*y(3); 0.04*y(1)-1e4*y(2)*y(3)-3e7*y(2)^2; 3e7*y(2)^2];\n\
         [t, y] = ode15s(f, [0 40], [1 0 0], odeset('RelTol',1e-6,'AbsTol',1e-10)); r = y(end,:);",
    );
    let r = i.get("r").unwrap().to_real().unwrap().d;
    near(r[0], 0.7158, 3e-4);
    near(r.iter().sum::<f64>(), 1.0, 1e-8);
    let d = run(
        "opts = odeset('Mass', [1 0; 0 0], 'RelTol', 1e-6, 'AbsTol', 1e-9);\n\
         [t, y] = ode23s(@(t,y) [-y(1)+y(2); y(2)-2*y(1)], [0 1], [1; 2], opts); r = y(end,:);",
    );
    let r = d.get("r").unwrap().to_real().unwrap().d;
    near(r[0], 1f64.exp(), 5e-3);
    near(r[1], 2.0 * r[0], 1e-6);
}

#[test]
fn backward_integration_is_supported() {
    near(scalar("[t, y] = ode45(@(t,y) y, [1 0], exp(1), odeset('RelTol',1e-9,'AbsTol',1e-12)); ans = y(end);"), 1.0, 1e-6);
}

#[test]
fn odeset_and_odeget_round_trip() {
    near(
        scalar("o = odeset('RelTol', 1e-5); ans = odeget(o, 'reltol');"),
        1e-5,
        1e-12,
    );
    near(
        scalar("o = odeset(); ans = odeget(o, 'AbsTol', 7);"),
        7.0,
        0.0,
    );
    near(
        scalar("o = optimset('TolX', 1e-3); ans = optimget(o, 'TolX');"),
        1e-3,
        1e-12,
    );
}

#[test]
fn hostile_arguments_give_errors_not_panics() {
    for src in [
        "ode45(@(t,y) y, [0 0], 1)",
        "ode45(@(t,y) y, [0 1 0.5], 1)",
        "ode45(@(t,y) y, [0 Inf], 1)",
        "ode45(@(t,y) y, [0 1], [])",
        "ode45(@(t,y) [y; y], [0 1], 1)",
        "ode45(@(t,y) 1/0*y, [0 1], 1)",
        "ode45(3, [0 1], 1)",
        "ode45(@(t,y) y, [0 1], 1, odeset('RelTol', -1))",
        "ode45(@(t,y) y, [0 1], 1, odeset('Mass', [1 2]))",
        "ode45(@(t,y) y, [0 1], 1, odeset('Mass', @(t) 1))",
        "ode45(@(t,y) y, [0 1], 1, odeset('AbsTol', [1 2]))",
        "fminsearch(@(x) [x x], 1)",
        "fminsearch(@(x) x, [])",
        "fminsearch(@(x) x, NaN)",
        "fminbnd(@(x) x, 2, 1)",
        "lsqcurvefit(@(p,x) p*x, 1, [1 2 3], [1 2])",
        "linprog([1 2], [1 2 3], 1)",
        "optimset('TolX')",
    ] {
        let mut i = Interp::new();
        assert!(i.run(src).is_err(), "`{src}` should fail");
    }
    assert!(fails("fminsearch(@(x) error('boom'), 1)").contains("boom"));
}
