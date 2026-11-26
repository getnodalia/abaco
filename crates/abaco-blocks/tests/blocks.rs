// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Runs diagrams whose answers are known analytically and checks the solvers, the discrete
//! blocks, the events and the errors.

use abaco_blocks::*;

fn integ(init: f64) -> BlockKind {
    BlockKind::Integrator {
        initial: vec![init],
        lower: None,
        upper: None,
    }
}
fn scope() -> BlockKind {
    BlockKind::Scope { inputs: 1 }
}

/// `x' = -x`, `x(0) = 1`, logged by `scope`.
fn decay(method: Method, step: Option<f64>, rtol: f64) -> Model {
    let mut m = Model::new("decay");
    m.solver = SolverConfig {
        method,
        stop_time: 5.0,
        step,
        rtol,
        atol: rtol * 1e-3,
        ..SolverConfig::default()
    };
    m.add("int", integ(1.0)).unwrap();
    m.add("neg", BlockKind::Gain { gain: -1.0 }).unwrap();
    m.add("scope", scope()).unwrap();
    m.connect(("int", 0), ("neg", 0));
    m.connect(("neg", 0), ("int", 0));
    m.connect(("int", 0), ("scope", 0));
    m
}

fn last(r: &SimResult, name: &str) -> f64 {
    *r.signal(name).unwrap().data.last().unwrap()
}

#[test]
fn fixed_step_methods_converge_with_their_order() {
    // Global error at t = 5 for h = 0.1 against e^-5, and the ratio when h is halved.
    let exact = (-5.0f64).exp();
    for (method, order) in [
        (Method::Euler, 1.0),
        (Method::Heun, 2.0),
        (Method::Bs3, 3.0),
        (Method::Rk4, 4.0),
    ] {
        let e1 = (last(&simulate(&decay(method, Some(0.1), 1e-3)).unwrap(), "scope") - exact).abs();
        let e2 = (last(
            &simulate(&decay(method, Some(0.05), 1e-3)).unwrap(),
            "scope",
        ) - exact)
            .abs();
        let observed = (e1 / e2).log2();
        assert!(
            (observed - order).abs() < 0.35,
            "{method:?}: observed order {observed:.2}, expected {order}"
        );
    }
}

#[test]
fn variable_step_methods_meet_their_tolerance() {
    let exact = (-5.0f64).exp();
    for (method, rtol, bound) in [
        (Method::Dopri5, 1e-8, 1e-8),
        (Method::Bs23, 1e-6, 1e-5),
        (Method::Rosenbrock23, 1e-6, 1e-4),
    ] {
        let r = simulate(&decay(method, None, rtol)).unwrap();
        let err = (last(&r, "scope") - exact).abs();
        assert!(err < bound, "{method:?}: error {err:e}");
        assert!(r.time.windows(2).all(|w| w[1] > w[0]), "time must increase");
        assert_eq!(*r.time.last().unwrap(), 5.0);
    }
}

#[test]
fn second_order_system_matches_the_analytic_step_response() {
    // y'' + 2ζω y' + ω² y = ω² u, with ζ = 0.3, ω = 2: step response in closed form.
    let (zeta, w) = (0.3f64, 2.0f64);
    let mut m = Model::new("tf");
    m.solver = SolverConfig {
        stop_time: 6.0,
        rtol: 1e-9,
        atol: 1e-12,
        ..SolverConfig::default()
    };
    m.add(
        "u",
        BlockKind::Step {
            time: 0.0,
            before: 0.0,
            after: 1.0,
        },
    )
    .unwrap();
    m.add(
        "g",
        BlockKind::TransferFcn {
            num: vec![w * w],
            den: vec![1.0, 2.0 * zeta * w, w * w],
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("u", 0), ("g", 0));
    m.connect(("g", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    let s = r.signal("y").unwrap();
    let wd = w * (1.0 - zeta * zeta).sqrt();
    let mut worst = 0.0f64;
    for (i, &t) in r.time.iter().enumerate() {
        let exact = 1.0 - (-zeta * w * t).exp() * ((wd * t).cos() + zeta * w / wd * (wd * t).sin());
        worst = worst.max((s.row(i)[0] - exact).abs());
    }
    assert!(worst < 1e-7, "worst error {worst:e}");
}

#[test]
fn state_space_and_zero_pole_agree_with_the_transfer_function() {
    // 1/((s+1)(s+2)) three ways.
    let build = |kind: BlockKind| {
        let mut m = Model::new("x");
        m.solver.stop_time = 4.0;
        m.solver.rtol = 1e-9;
        m.solver.atol = 1e-12;
        m.add(
            "u",
            BlockKind::Step {
                time: 0.0,
                before: 0.0,
                after: 1.0,
            },
        )
        .unwrap();
        m.add("g", kind).unwrap();
        m.add("y", scope()).unwrap();
        m.connect(("u", 0), ("g", 0));
        m.connect(("g", 0), ("y", 0));
        simulate(&m).unwrap()
    };
    let tf = build(BlockKind::TransferFcn {
        num: vec![1.0],
        den: vec![1.0, 3.0, 2.0],
    });
    let zp = build(BlockKind::ZeroPole {
        zeros: vec![],
        poles: vec![[-1.0, 0.0], [-2.0, 0.0]],
        gain: 1.0,
    });
    let ss = build(BlockKind::StateSpace {
        a: vec![vec![0.0, 1.0], vec![-2.0, -3.0]],
        b: vec![vec![0.0], vec![1.0]],
        c: vec![vec![1.0, 0.0]],
        d: vec![vec![0.0]],
        x0: vec![],
    });
    // y(t) = 1/2 - e^-t + e^-2t / 2
    let exact = |t: f64| 0.5 - (-t).exp() + 0.5 * (-2.0 * t).exp();
    for r in [&tf, &zp, &ss] {
        let s = r.signal("y").unwrap();
        for (i, &t) in r.time.iter().enumerate() {
            assert!((s.row(i)[0] - exact(t)).abs() < 1e-7);
        }
    }
}

#[test]
fn complex_poles_must_be_conjugate_pairs() {
    let mut m = Model::new("zp");
    m.add("u", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add(
        "g",
        BlockKind::ZeroPole {
            zeros: vec![],
            poles: vec![[-1.0, 2.0]],
            gain: 1.0,
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("u", 0), ("g", 0));
    m.connect(("g", 0), ("y", 0));
    let e = simulate(&m).unwrap_err().to_string();
    assert!(e.contains("conjugate"), "{e}");
}

#[test]
fn stiff_problem_is_cheap_for_the_stiff_solver() {
    // y' = -k (y - cos t); y(0) = 0, k = 10⁴: the explicit method is limited by stability.
    let build = |method| {
        let mut m = Model::new("stiff");
        m.solver = SolverConfig {
            method,
            stop_time: 2.0,
            rtol: 1e-5,
            atol: 1e-8,
            ..SolverConfig::default()
        };
        m.add(
            "clk",
            BlockKind::Sine {
                amplitude: 1.0,
                frequency: 1.0,
                phase: std::f64::consts::FRAC_PI_2,
                bias: 0.0,
            },
        )
        .unwrap();
        m.add("sub", BlockKind::Sum { signs: "+-".into() }).unwrap();
        m.add("k", BlockKind::Gain { gain: 1.0e4 }).unwrap();
        m.add("int", integ(0.0)).unwrap();
        m.add("y", scope()).unwrap();
        // y' = k (cos t - y)
        m.connect(("clk", 0), ("sub", 0));
        m.connect(("int", 0), ("sub", 1));
        m.connect(("sub", 0), ("k", 0));
        m.connect(("k", 0), ("int", 0));
        m.connect(("int", 0), ("y", 0));
        simulate(&m).unwrap()
    };
    let stiff = build(Method::Rosenbrock23);
    let explicit = build(Method::Dopri5);
    let t = 2.0f64;
    // Exact: y = (k² cos t + k sin t)/(k²+1) - k²/(k²+1) e^(-k t)
    let k = 1.0e4f64;
    let exact =
        (k * k * t.cos() + k * t.sin()) / (k * k + 1.0) - k * k / (k * k + 1.0) * (-k * t).exp();
    assert!(
        (last(&stiff, "y") - exact).abs() < 2e-4,
        "{}",
        last(&stiff, "y") - exact
    );
    assert!(
        stiff.stats.steps * 3 < explicit.stats.steps,
        "stiff {} steps, explicit {}",
        stiff.stats.steps,
        explicit.stats.steps
    );
}

#[test]
fn discrete_blocks_sample_exactly() {
    // Ramp through a unit delay and a zero-order hold, Ts = 0.5.
    let mut m = Model::new("discrete");
    m.solver = SolverConfig {
        stop_time: 2.0,
        ..SolverConfig::default()
    };
    m.add("clk", BlockKind::Clock).unwrap();
    m.add("zoh", BlockKind::ZeroOrderHold { sample_time: 0.5 })
        .unwrap();
    m.add(
        "z",
        BlockKind::UnitDelay {
            initial: vec![-1.0],
            sample_time: 0.5,
        },
    )
    .unwrap();
    m.add("hold", scope()).unwrap();
    m.add("delayed", scope()).unwrap();
    m.connect(("clk", 0), ("zoh", 0));
    m.connect(("zoh", 0), ("z", 0));
    m.connect(("zoh", 0), ("hold", 0));
    m.connect(("z", 0), ("delayed", 0));
    let r = simulate(&m).unwrap();
    let at = |name: &str, t: f64| {
        let i = r
            .time
            .iter()
            .position(|&x| (x - t).abs() < 1e-12)
            .unwrap_or_else(|| panic!("no sample at {t}"));
        r.signal(name).unwrap().row(i)[0]
    };
    for k in 0..=4 {
        let t = 0.5 * k as f64;
        assert_eq!(at("hold", t), t, "the hold samples the clock at t = {t}");
        let expected = if k == 0 { -1.0 } else { 0.5 * (k - 1) as f64 };
        assert_eq!(at("delayed", t), expected);
    }
}

#[test]
fn discrete_integrator_rules_match_their_difference_equations() {
    // u = 1: forward y[n] = n·Ts; backward y[n] = (n+1)·Ts; trapezoidal y[n] = (n+1/2)·Ts.
    for (rule, f) in [
        (DiscreteRule::ForwardEuler, 0.0),
        (DiscreteRule::BackwardEuler, 1.0),
        (DiscreteRule::Trapezoidal, 0.5),
    ] {
        let mut m = Model::new("di");
        m.solver.stop_time = 1.0;
        m.add("one", BlockKind::Constant { value: vec![1.0] })
            .unwrap();
        m.add(
            "di",
            BlockKind::DiscreteIntegrator {
                rule,
                gain: 1.0,
                initial: 0.0,
                sample_time: 0.25,
            },
        )
        .unwrap();
        m.add("y", scope()).unwrap();
        m.connect(("one", 0), ("di", 0));
        m.connect(("di", 0), ("y", 0));
        let r = simulate(&m).unwrap();
        for (i, &t) in r.time.iter().enumerate() {
            let n = (t / 0.25 + 1e-9).floor();
            assert!(
                (r.signal("y").unwrap().row(i)[0] - (n + f) * 0.25).abs() < 1e-12,
                "{rule:?} at t = {t}"
            );
        }
    }
}

#[test]
fn discrete_transfer_function_follows_the_difference_equation() {
    // y[k] = 0.5 y[k-1] + u[k-1]  (H = 1/(z - 0.5)) for a unit step input.
    let mut m = Model::new("dtf");
    m.solver.stop_time = 6.0;
    m.add("u", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add(
        "h",
        BlockKind::DiscreteTransferFcn {
            num: vec![1.0],
            den: vec![1.0, -0.5],
            sample_time: 1.0,
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("u", 0), ("h", 0));
    m.connect(("h", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    // The output holds y[k] over [k, k+1) and changes only at the sample instants.
    let mut seq = vec![0.0f64];
    for k in 0..8 {
        seq.push(0.5 * seq[k] + 1.0);
    }
    for (k, &t) in r.time.iter().enumerate() {
        let n = (t + 1e-9).floor() as usize;
        assert!(
            (r.signal("y").unwrap().row(k)[0] - seq[n]).abs() < 1e-12,
            "y at t = {t}"
        );
    }
}

#[test]
fn saturation_is_located_exactly() {
    // ∫1 dt through a saturation at 0.5: the output's corner is at t = 0.5.
    let mut m = Model::new("sat");
    m.solver = SolverConfig {
        stop_time: 1.0,
        max_step: Some(0.3),
        ..SolverConfig::default()
    };
    m.add("one", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add("int", integ(0.0)).unwrap();
    m.add(
        "sat",
        BlockKind::Saturation {
            lower: -1.0,
            upper: 0.5,
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("one", 0), ("int", 0));
    m.connect(("int", 0), ("sat", 0));
    m.connect(("sat", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    assert!(r.stats.zero_crossings >= 1);
    let corner = r.time.iter().copied().find(|&t| (t - 0.5).abs() < 1e-9);
    assert!(
        corner.is_some(),
        "no recorded point at the saturation corner: {:?}",
        r.time
    );
}

#[test]
fn relay_switches_with_hysteresis() {
    // Sine into a relay (on at 0.5, off at -0.5): the output is high between the crossings.
    let mut m = Model::new("relay");
    m.solver = SolverConfig {
        stop_time: 4.0 * std::f64::consts::PI,
        ..SolverConfig::default()
    };
    m.add(
        "s",
        BlockKind::Sine {
            amplitude: 1.0,
            frequency: 1.0,
            phase: 0.0,
            bias: 0.0,
        },
    )
    .unwrap();
    m.add(
        "r",
        BlockKind::Relay {
            on_switch: 0.5,
            off_switch: -0.5,
            on_output: 1.0,
            off_output: 0.0,
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("s", 0), ("r", 0));
    m.connect(("r", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    let y = r.signal("y").unwrap();
    // Switches on at t = asin(0.5) = π/6, off at 2π - asin(0.5) ... = 7π/6 + ... : sin(t) = -0.5 at t = 7π/6.
    let on = std::f64::consts::PI / 6.0;
    let off = 7.0 * std::f64::consts::PI / 6.0;
    for (i, &t) in r.time.iter().enumerate() {
        let v = y.row(i)[0];
        if t > on + 1e-6 && t < off - 1e-6 {
            assert_eq!(v, 1.0, "relay should be on at t = {t}");
        }
        if t < on - 1e-6 {
            assert_eq!(v, 0.0, "relay should be off at t = {t}");
        }
    }
    assert!(r.stats.zero_crossings >= 3);
}

#[test]
fn step_switching_time_is_hit_exactly_and_not_smeared() {
    let mut m = Model::new("step");
    m.solver = SolverConfig {
        stop_time: 2.0,
        rtol: 1e-9,
        atol: 1e-12,
        ..SolverConfig::default()
    };
    m.add(
        "s",
        BlockKind::Step {
            time: 0.7,
            before: 0.0,
            after: 2.0,
        },
    )
    .unwrap();
    m.add("int", integ(0.0)).unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("s", 0), ("int", 0));
    m.connect(("int", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    // ∫ step = 2 (t - 0.7) after 0.7. The jump is landed on, so the error is round-off only.
    assert!(
        (last(&r, "y") - 2.0 * 1.3).abs() < 1e-12,
        "{}",
        last(&r, "y")
    );
}

#[test]
fn pulse_generator_integrates_to_its_area() {
    let mut m = Model::new("pulse");
    m.solver = SolverConfig {
        stop_time: 10.0,
        ..SolverConfig::default()
    };
    m.add(
        "p",
        BlockKind::Pulse {
            amplitude: 3.0,
            period: 1.0,
            duty: 25.0,
            delay: 0.0,
        },
    )
    .unwrap();
    m.add("int", integ(0.0)).unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("p", 0), ("int", 0));
    m.connect(("int", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    // 10 periods × 0.25 s × 3.
    assert!((last(&r, "y") - 7.5).abs() < 1e-9, "{}", last(&r, "y"));
}

#[test]
fn square_wave_integral_is_a_triangle() {
    let mut m = Model::new("square");
    m.solver = SolverConfig {
        stop_time: 3.0,
        ..SolverConfig::default()
    };
    m.add(
        "w",
        BlockKind::Wave {
            shape: WaveShape::Square,
            amplitude: 1.0,
            frequency: 1.0,
        },
    )
    .unwrap();
    m.add("int", integ(0.0)).unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("w", 0), ("int", 0));
    m.connect(("int", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    // +1 on [0, .5), -1 on [.5, 1): the integral peaks at 0.5 and returns to 0 at each period.
    let y = r.signal("y").unwrap();
    let at = |t: f64| {
        let i = r
            .time
            .iter()
            .position(|&x| (x - t).abs() < 1e-9)
            .unwrap_or(usize::MAX);
        y.row(i)[0]
    };
    assert!((at(0.5) - 0.5).abs() < 1e-9);
    assert!(at(1.0).abs() < 1e-9);
    assert!(at(2.0).abs() < 1e-9);
}

#[test]
fn transport_delay_delays_a_step() {
    let mut m = Model::new("delay");
    m.solver = SolverConfig {
        stop_time: 3.0,
        ..SolverConfig::default()
    };
    m.add(
        "s",
        BlockKind::Step {
            time: 0.5,
            before: 0.0,
            after: 1.0,
        },
    )
    .unwrap();
    m.add(
        "d",
        BlockKind::TransportDelay {
            delay: 1.0,
            initial: 0.0,
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("s", 0), ("d", 0));
    m.connect(("d", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    let y = r.signal("y").unwrap();
    for (i, &t) in r.time.iter().enumerate() {
        if t < 1.5 - 1e-3 {
            assert_eq!(y.row(i)[0], 0.0, "t = {t}");
        }
        if t > 1.5 + 1e-3 {
            assert_eq!(y.row(i)[0], 1.0, "t = {t}");
        }
    }
}

#[test]
fn pid_closed_loop_has_zero_steady_state_error_and_the_right_poles() {
    // Plant 1/(s+1) with PI control kp = 2, ki = 3: closed loop s² + 3s + 3, steady state 1.
    let mut m = Model::new("pid");
    m.solver = SolverConfig {
        stop_time: 20.0,
        rtol: 1e-8,
        atol: 1e-10,
        ..SolverConfig::default()
    };
    m.add("r", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add("e", BlockKind::Sum { signs: "+-".into() }).unwrap();
    m.add(
        "c",
        BlockKind::Pid {
            kp: 2.0,
            ki: 3.0,
            kd: 0.0,
            n: 100.0,
            initial: 0.0,
        },
    )
    .unwrap();
    m.add(
        "p",
        BlockKind::TransferFcn {
            num: vec![1.0],
            den: vec![1.0, 1.0],
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("r", 0), ("e", 0));
    m.connect(("p", 0), ("e", 1));
    m.connect(("e", 0), ("c", 0));
    m.connect(("c", 0), ("p", 0));
    m.connect(("p", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    assert!((last(&r, "y") - 1.0).abs() < 1e-6, "{}", last(&r, "y"));
    // Closed-loop T = (2s + 3)/(s² + 3s + 3); step response y = 1 - e^{-1.5t}(cos ωt + 0.5/ω... ) is checked
    // through the closed form of the underdamped step response with ζ = √3/2, ωn = √3.
    let (wn, zeta) = (3f64.sqrt(), 3f64.sqrt() / 2.0);
    let wd = wn * (1.0 - zeta * zeta).sqrt();
    let y = r.signal("y").unwrap();
    for (i, &t) in r.time.iter().enumerate() {
        // y(t) = 1 - e^{-σt}(cos wd t + b sin wd t), σ = 1.5; y'(0+) = 2 (the numerator's 2s) gives b.
        let sigma = zeta * wn;
        // y(0) = 0; y'(0) = 2 = σ - b·wd·... solve: derivative at 0 of -e^{-σt}(cos + b sin) = σ - b wd
        let b = (sigma - 2.0) / wd;
        let exact = 1.0 - (-sigma * t).exp() * ((wd * t).cos() + b * (wd * t).sin());
        assert!(
            (y.row(i)[0] - exact).abs() < 1e-5,
            "t = {t}: {} vs {exact}",
            y.row(i)[0]
        );
    }
}

#[test]
fn subsystems_goto_from_and_vectors() {
    // A subsystem computing 3u + 1, fed by a vector, split with Demux, rejoined with Mux.
    let mut inner = Model::new("affine");
    inner.add("in", BlockKind::Inport { index: 1 }).unwrap();
    inner.add("g", BlockKind::Gain { gain: 3.0 }).unwrap();
    inner.add("b", BlockKind::Bias { bias: 1.0 }).unwrap();
    inner.add("out", BlockKind::Outport { index: 1 }).unwrap();
    inner.connect(("in", 0), ("g", 0));
    inner.connect(("g", 0), ("b", 0));
    inner.connect(("b", 0), ("out", 0));
    let mut m = Model::new("outer");
    m.solver.stop_time = 1.0;
    m.add(
        "v",
        BlockKind::Constant {
            value: vec![1.0, 2.0, 3.0, 4.0],
        },
    )
    .unwrap();
    m.add("dm", BlockKind::Demux { outputs: 2 }).unwrap();
    m.add(
        "sub",
        BlockKind::Subsystem {
            model: Box::new(inner),
        },
    )
    .unwrap();
    m.add("tag", BlockKind::Goto { tag: "a".into() }).unwrap();
    m.add("from", BlockKind::From { tag: "a".into() }).unwrap();
    m.add("mux", BlockKind::Mux { inputs: 2 }).unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("v", 0), ("dm", 0));
    m.connect(("dm", 0), ("sub", 0));
    m.connect(("sub", 0), ("tag", 0));
    m.connect(("from", 0), ("mux", 0));
    m.connect(("dm", 1), ("mux", 1));
    m.connect(("mux", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    let y = r.signal("y").unwrap();
    assert_eq!(y.width, 4);
    assert_eq!(y.row(0), &[4.0, 7.0, 3.0, 4.0]);
}

#[test]
fn algebraic_loops_are_reported_with_the_blocks_in_them() {
    let mut m = Model::new("loop");
    m.add("a", BlockKind::Gain { gain: 2.0 }).unwrap();
    m.add("b", BlockKind::Gain { gain: 3.0 }).unwrap();
    m.connect(("a", 0), ("b", 0));
    m.connect(("b", 0), ("a", 0));
    let e = simulate(&m).unwrap_err();
    let text = e.to_string();
    assert!(matches!(e, BlockError::AlgebraicLoop(_)), "{text}");
    assert!(text.contains('a') && text.contains('b'));
}

#[test]
fn a_unit_delay_breaks_the_loop() {
    // y[k+1] = 0.5 y[k] + 1 → 2.
    let mut m = Model::new("fb");
    m.solver.stop_time = 40.0;
    m.add("one", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add("sum", BlockKind::Sum { signs: "++".into() }).unwrap();
    m.add(
        "z",
        BlockKind::UnitDelay {
            initial: vec![0.0],
            sample_time: 1.0,
        },
    )
    .unwrap();
    m.add("g", BlockKind::Gain { gain: 0.5 }).unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("one", 0), ("sum", 0));
    m.connect(("g", 0), ("sum", 1));
    m.connect(("sum", 0), ("z", 0));
    m.connect(("z", 0), ("g", 0));
    m.connect(("z", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    assert!((last(&r, "y") - 2.0).abs() < 1e-9);
}

#[test]
fn wiring_mistakes_name_the_block_and_the_port() {
    let mut m = Model::new("bad");
    m.add("sum", BlockKind::Sum { signs: "++".into() }).unwrap();
    m.add("c", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.connect(("c", 0), ("sum", 0));
    let e = simulate(&m).unwrap_err().to_string();
    assert!(e.contains("sum") && e.contains("input 2"), "{e}");

    let mut m = Model::new("bad2");
    m.add("c", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add("g", BlockKind::Gain { gain: 1.0 }).unwrap();
    m.connect(("c", 0), ("g", 0));
    m.connect(("c", 0), ("g", 0));
    assert!(simulate(&m).unwrap_err().to_string().contains("two lines"));

    let mut m = Model::new("bad3");
    m.add("c", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.connect(("c", 0), ("ghost", 0));
    assert!(simulate(&m).unwrap_err().to_string().contains("ghost"));

    let mut m = Model::new("bad4");
    m.add(
        "c",
        BlockKind::Constant {
            value: vec![1.0, 2.0],
        },
    )
    .unwrap();
    m.add(
        "tf",
        BlockKind::TransferFcn {
            num: vec![1.0],
            den: vec![1.0, 1.0],
        },
    )
    .unwrap();
    m.connect(("c", 0), ("tf", 0));
    assert!(simulate(&m).unwrap_err().to_string().contains("scalar"));
}

#[test]
fn a_non_finite_signal_is_reported_where_it_happens() {
    let mut m = Model::new("nan");
    m.add("c", BlockKind::Constant { value: vec![-1.0] })
        .unwrap();
    m.add(
        "log",
        BlockKind::Math {
            func: MathFunc::Log,
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("c", 0), ("log", 0));
    m.connect(("log", 0), ("y", 0));
    let e = simulate(&m).unwrap_err();
    assert!(
        matches!(&e, BlockError::NonFinite { block, .. } if block == "log"),
        "{e}"
    );
}

#[test]
fn stop_block_ends_the_run() {
    let mut m = Model::new("stop");
    m.solver.stop_time = 10.0;
    m.add("t", BlockKind::Clock).unwrap();
    m.add(
        "cmp",
        BlockKind::CompareConst {
            op: RelOp::Ge,
            value: 2.5,
        },
    )
    .unwrap();
    m.add("stop", BlockKind::Stop).unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("t", 0), ("cmp", 0));
    m.connect(("cmp", 0), ("stop", 0));
    m.connect(("t", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    assert_eq!(r.stopped_by.as_deref(), Some("stop"));
    let end = *r.time.last().unwrap();
    assert!((end - 2.5).abs() < 1e-6, "{end}");
}

#[test]
fn noise_repeats_with_its_seed_and_has_the_requested_moments() {
    let build = |seed| {
        let mut m = Model::new("noise");
        m.solver = SolverConfig {
            stop_time: 2000.0,
            method: Method::Euler,
            step: Some(1.0),
            ..SolverConfig::default()
        };
        m.add(
            "n",
            BlockKind::Noise {
                mean: 2.0,
                variance: 4.0,
                seed,
                sample_time: 1.0,
            },
        )
        .unwrap();
        m.add("y", scope()).unwrap();
        m.connect(("n", 0), ("y", 0));
        simulate(&m).unwrap()
    };
    let a = build(7);
    let b = build(7);
    let c = build(8);
    assert_eq!(a, b);
    assert_ne!(a.signal("y"), c.signal("y"));
    let v = a.signal("y").unwrap().column(0);
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64;
    assert!((mean - 2.0).abs() < 0.2, "mean {mean}");
    assert!((var - 4.0).abs() < 0.6, "variance {var}");
}

#[test]
fn integrator_limits_hold_the_state() {
    let mut m = Model::new("lim");
    m.solver.stop_time = 3.0;
    m.add("one", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add(
        "int",
        BlockKind::Integrator {
            initial: vec![0.0],
            lower: Some(-1.0),
            upper: Some(1.0),
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("one", 0), ("int", 0));
    m.connect(("int", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    assert!((last(&r, "y") - 1.0).abs() < 1e-9);
    assert!(r
        .signal("y")
        .unwrap()
        .data
        .iter()
        .all(|v| *v <= 1.0 + 1e-12));
}

#[test]
fn linearisation_recovers_the_transfer_function() {
    // u → [1/(s+1)] → [2/(s+3)] → y ; overall 2/((s+1)(s+3)).
    let mut m = Model::new("lin");
    m.add("u", BlockKind::Inport { index: 1 }).unwrap();
    m.add(
        "a",
        BlockKind::TransferFcn {
            num: vec![1.0],
            den: vec![1.0, 1.0],
        },
    )
    .unwrap();
    m.add(
        "b",
        BlockKind::TransferFcn {
            num: vec![2.0],
            den: vec![1.0, 3.0],
        },
    )
    .unwrap();
    m.add("y", BlockKind::Outport { index: 1 }).unwrap();
    m.connect(("u", 0), ("a", 0));
    m.connect(("a", 0), ("b", 0));
    m.connect(("b", 0), ("y", 0));
    let c = compile(&m).unwrap();
    let (x0, _) = c.initial_state();
    let ss = linearize(&c, 0.0, &x0, &[0.0]).unwrap();
    let (num, den) = nodalia_numerics::control::ss2tf(&ss).unwrap();
    let want_den = [1.0, 4.0, 3.0];
    for (a, b) in den.iter().zip(want_den) {
        assert!((a - b).abs() < 1e-6, "den {den:?}");
    }
    assert!(
        (num.last().copied().unwrap_or(0.0) - 2.0).abs() < 1e-6,
        "num {num:?}"
    );
}

#[test]
fn nonlinear_pendulum_linearises_about_the_bottom() {
    // θ'' = -sin θ + u: A = [[0,1],[-1,0]], B = [0,1]ᵀ, C = [1,0].
    let mut m = Model::new("pend");
    m.add("u", BlockKind::Inport { index: 1 }).unwrap();
    m.add("sum", BlockKind::Sum { signs: "+-".into() }).unwrap();
    m.add("w", integ(0.0)).unwrap();
    m.add("th", integ(0.0)).unwrap();
    m.add(
        "sin",
        BlockKind::Math {
            func: MathFunc::Sin,
        },
    )
    .unwrap();
    m.add("y", BlockKind::Outport { index: 1 }).unwrap();
    m.connect(("u", 0), ("sum", 0));
    m.connect(("sin", 0), ("sum", 1));
    m.connect(("sum", 0), ("w", 0));
    m.connect(("w", 0), ("th", 0));
    m.connect(("th", 0), ("sin", 0));
    m.connect(("th", 0), ("y", 0));
    let c = compile(&m).unwrap();
    let ss = linearize(&c, 0.0, &[0.0, 0.0], &[0.0]).unwrap();
    // States are ordered by block: w (angular velocity) then th.
    assert!((ss.a.get(0, 1).unwrap() + 1.0).abs() < 1e-6);
    assert!((ss.a.get(1, 0).unwrap() - 1.0).abs() < 1e-6);
    assert!((ss.b.get(0, 0).unwrap() - 1.0).abs() < 1e-6);
    assert!((ss.c.get(0, 1).unwrap() - 1.0).abs() < 1e-6);
}

#[test]
fn nonlinear_van_der_pol_conserves_the_limit_cycle_amplitude() {
    // μ = 1: the limit cycle has amplitude ≈ 2.0086 (Strogatz / Guckenheimer–Holmes).
    let mut m = Model::new("vdp");
    m.solver = SolverConfig {
        stop_time: 60.0,
        rtol: 1e-9,
        atol: 1e-11,
        ..SolverConfig::default()
    };
    m.add("v", integ(1.0)).unwrap(); // x' = v ... actually integrates x; see wiring
    m.add("w", integ(0.0)).unwrap();
    // x' = w ; w' = (1 - x²) w - x
    m.add(
        "sq",
        BlockKind::Math {
            func: MathFunc::Square,
        },
    )
    .unwrap();
    m.add("one", BlockKind::Constant { value: vec![1.0] })
        .unwrap();
    m.add("om", BlockKind::Sum { signs: "+-".into() }).unwrap();
    m.add("p", BlockKind::Product { ops: "**".into() }).unwrap();
    m.add("dw", BlockKind::Sum { signs: "+-".into() }).unwrap();
    m.add("x", scope()).unwrap();
    m.connect(("w", 0), ("v", 0));
    m.connect(("v", 0), ("sq", 0));
    m.connect(("one", 0), ("om", 0));
    m.connect(("sq", 0), ("om", 1));
    m.connect(("om", 0), ("p", 0));
    m.connect(("w", 0), ("p", 1));
    m.connect(("p", 0), ("dw", 0));
    m.connect(("v", 0), ("dw", 1));
    m.connect(("dw", 0), ("w", 0));
    m.connect(("v", 0), ("x", 0));
    let r = simulate(&m).unwrap();
    let x = r.signal("x").unwrap().column(0);
    let tail = &x[x.len() / 2..];
    let peak = tail.iter().fold(0.0f64, |a, b| a.max(b.abs()));
    assert!((peak - 2.0086).abs() < 5e-3, "peak {peak}");
}

#[test]
fn json_round_trip_and_hostile_files() {
    let m = decay(Method::Dopri5, None, 1e-6);
    let text = m.to_json().unwrap();
    assert_eq!(Model::from_json(&text).unwrap(), m);
    // Scalars may be written as bare numbers or lists.
    let parsed = Model::from_json(
        r#"{"blocks":[{"name":"c","type":"Constant","value":[1,2]},{"name":"d","type":"Constant","value":3}],"lines":[]}"#,
    )
    .unwrap();
    assert!(
        matches!(&parsed.blocks[0].kind, BlockKind::Constant { value } if value == &[1.0, 2.0])
    );
    for bad in [
        "",
        "{",
        "[]",
        r#"{"blocks":[{"name":"x","type":"Nope"}]}"#,
        r#"{"solver":{"method":"magic"}}"#,
    ] {
        assert!(Model::from_json(bad).is_err(), "{bad}");
    }
    let huge = " ".repeat(model::MAX_FILE_BYTES + 1);
    assert!(Model::from_json(&huge).is_err());
}

#[test]
fn hostile_parameters_give_errors_not_panics() {
    let bad: Vec<BlockKind> = vec![
        BlockKind::TransferFcn {
            num: vec![],
            den: vec![],
        },
        BlockKind::TransferFcn {
            num: vec![1.0, 1.0, 1.0],
            den: vec![1.0],
        },
        BlockKind::TransferFcn {
            num: vec![f64::NAN],
            den: vec![1.0, 1.0],
        },
        BlockKind::StateSpace {
            a: vec![vec![1.0, 2.0]],
            b: vec![vec![1.0]],
            c: vec![vec![1.0]],
            d: vec![vec![0.0]],
            x0: vec![],
        },
        BlockKind::Lookup1D {
            x: vec![1.0, 0.0],
            y: vec![0.0, 1.0],
        },
        BlockKind::Selector { indices: vec![0] },
        BlockKind::Sum { signs: "x".into() },
        BlockKind::Demux { outputs: 0 },
        BlockKind::Delay {
            length: usize::MAX,
            initial: 0.0,
            sample_time: 1.0,
        },
        BlockKind::UnitDelay {
            initial: vec![],
            sample_time: 1.0,
        },
        BlockKind::UnitDelay {
            initial: vec![0.0],
            sample_time: 0.0,
        },
        BlockKind::Pulse {
            amplitude: 1.0,
            period: 0.0,
            duty: 50.0,
            delay: 0.0,
        },
        BlockKind::Pulse {
            amplitude: 1.0,
            period: 1e-9,
            duty: 50.0,
            delay: 0.0,
        },
        BlockKind::Table {
            times: vec![1.0, 0.0],
            values: vec![0.0, 0.0],
            hold: false,
        },
        BlockKind::Constant {
            value: vec![f64::INFINITY],
        },
        BlockKind::Saturation {
            lower: 1.0,
            upper: -1.0,
        },
        BlockKind::TransportDelay {
            delay: 0.0,
            initial: 0.0,
        },
        BlockKind::Integrator {
            initial: vec![0.0, 0.0],
            lower: None,
            upper: None,
        },
    ];
    for kind in bad {
        let mut m = Model::new("hostile");
        m.solver.stop_time = 10.0;
        m.add("src", BlockKind::Constant { value: vec![1.0] })
            .unwrap();
        m.add("b", kind.clone()).unwrap();
        m.connect(("src", 0), ("b", 0));
        let _ = simulate(&m); // must not panic; most are errors
        let mut e = Model::new("hostile");
        e.solver.stop_time = 10.0;
        e.add("b", kind.clone()).unwrap();
        let _ = simulate(&e);
    }
    // And a few that must be rejected with a message.
    let mut m = Model::new("x");
    m.solver.stop_time = 1.0;
    m.add(
        "b",
        BlockKind::Lookup1D {
            x: vec![1.0, 0.0],
            y: vec![0.0, 1.0],
        },
    )
    .unwrap();
    m.add("c", BlockKind::Constant { value: vec![0.0] })
        .unwrap();
    m.connect(("c", 0), ("b", 0));
    assert!(simulate(&m).is_err());
    let mut m = Model::new("x");
    m.solver.stop_time = -1.0;
    m.add("c", BlockKind::Constant { value: vec![0.0] })
        .unwrap();
    assert!(simulate(&m).unwrap_err().to_string().contains("stop time"));
}

#[test]
fn lookup_table_interpolates_and_clips() {
    let mut m = Model::new("lut");
    m.solver.stop_time = 3.0;
    m.add("t", BlockKind::Clock).unwrap();
    m.add(
        "lut",
        BlockKind::Lookup1D {
            x: vec![0.0, 1.0, 2.0],
            y: vec![0.0, 10.0, 0.0],
        },
    )
    .unwrap();
    m.add("y", scope()).unwrap();
    m.connect(("t", 0), ("lut", 0));
    m.connect(("lut", 0), ("y", 0));
    let r = simulate(&m).unwrap();
    let y = r.signal("y").unwrap();
    for (i, &t) in r.time.iter().enumerate() {
        let want = if t <= 1.0 {
            10.0 * t
        } else if t <= 2.0 {
            10.0 * (2.0 - t)
        } else {
            0.0
        };
        assert!((y.row(i)[0] - want).abs() < 1e-12, "t = {t}");
    }
}

#[test]
fn the_bundled_servo_example_settles_on_its_reference() {
    // P gain 20, rate gain 2 on a double integrator: closed loop s² + 2s + 20, poles
    // -1 ± 4.36j, so after 5 s past the step the position is within the 2 % band of 1.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/servo.json");
    let text = std::fs::read_to_string(path).unwrap();
    let m = Model::from_json(&text).unwrap();
    let r = simulate(&m).unwrap();
    let p = last(&r, "Position");
    assert!((p - 1.0).abs() < 0.02, "final position {p}");
    // The torque limit (5) is reached right after the step: 20 × 1 > 5.
    let torque = r.signal("Torque").unwrap().column(0);
    assert!(torque.iter().any(|&u| (u - 5.0).abs() < 1e-12));
    assert!(torque.iter().all(|&u| u.abs() <= 5.0 + 1e-12));
}
