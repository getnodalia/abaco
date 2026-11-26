// SPDX-License-Identifier: MIT OR Apache-2.0
//! The simulation: a machine that evaluates the compiled blocks, and the loop that advances
//! time with fixed or variable steps, lands exactly on sample hits, switching times and zero
//! crossings, and records what the scopes see.

use nodalia_numerics::{Lu, Matrix};

use crate::blocks::{self, Ctx, History};
use crate::compile::{Compiled, Node};
use crate::error::{BlockError, Result};
use crate::model::{BlockKind, Method, Model};

/// Most accepted steps before the run is given up.
const MAX_STEPS: u64 = 20_000_000;
/// Most points a run records.
const MAX_POINTS: usize = 5_000_000;

/// One recorded signal.
#[derive(Clone, Debug, PartialEq)]
pub struct Signal {
    /// Block path (`Scope`, `Plant/Scope`), with `:1`, `:2`… for blocks with several inputs.
    pub name: String,
    /// Number of elements per sample.
    pub width: usize,
    /// Row-major: `time.len()` rows of `width` values.
    pub data: Vec<f64>,
}

impl Signal {
    /// Number of samples.
    pub fn rows(&self) -> usize {
        self.data.len() / self.width.max(1)
    }
    /// Sample `i`.
    pub fn row(&self, i: usize) -> &[f64] {
        let w = self.width.max(1);
        self.data.get(i * w..(i + 1) * w).unwrap_or(&[])
    }
    /// Element `k` over time.
    pub fn column(&self, k: usize) -> Vec<f64> {
        let w = self.width.max(1);
        self.data.iter().skip(k).step_by(w).copied().collect()
    }
}

/// Counters of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Accepted steps.
    pub steps: u64,
    /// Steps rejected for error.
    pub rejected: u64,
    /// Zero crossings located.
    pub zero_crossings: u64,
    /// Evaluations of the block diagram.
    pub evaluations: u64,
}

/// The outcome of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct SimResult {
    /// Recorded times.
    pub time: Vec<f64>,
    /// Recorded signals (scopes, `ToWorkspace`, displays and the diagram's outports).
    pub signals: Vec<Signal>,
    /// Continuous states at the end.
    pub final_state: Vec<f64>,
    /// Discrete states at the end.
    pub final_discrete: Vec<f64>,
    /// Counters.
    pub stats: Stats,
    /// The block that stopped the run early, if one did.
    pub stopped_by: Option<String>,
}

impl SimResult {
    /// The signal with this name.
    pub fn signal(&self, name: &str) -> Option<&Signal> {
        self.signals.iter().find(|s| s.name == name)
    }
}

/// Settings of a run that are not part of the model.
#[derive(Clone, Debug, Default)]
pub struct SimOptions {
    /// Constant values for the diagram's top-level `Inport` blocks (zero when missing).
    pub inputs: Vec<f64>,
}

/// Compiles and runs a model.
pub fn simulate(model: &Model) -> Result<SimResult> {
    crate::compile::compile(model)?.simulate(&SimOptions::default())
}

fn port_slice<'s>(c: &Compiled, src: (usize, usize), sig: &'s [f64]) -> &'s [f64] {
    let Some(n) = c.nodes.get(src.0) else {
        return &[];
    };
    let off = n.out_base + n.port_off.get(src.1).copied().unwrap_or(0);
    let w = n.out_w.get(src.1).copied().unwrap_or(0);
    sig.get(off..off + w).unwrap_or(&[])
}

fn gather<'s>(c: &Compiled, nd: &Node, sig: &'s [f64]) -> Vec<&'s [f64]> {
    nd.src.iter().map(|&s| port_slice(c, s, sig)).collect()
}

fn xc_of<'a>(nd: &Node, xc: &'a [f64]) -> &'a [f64] {
    xc.get(nd.xc_off..nd.xc_off + nd.n_xc).unwrap_or(&[])
}

fn xd_of<'a>(nd: &Node, xd: &'a [f64]) -> &'a [f64] {
    xd.get(nd.xd_off..nd.xd_off + nd.n_xd).unwrap_or(&[])
}

/// Evaluates the block diagram.
pub(crate) struct Machine<'a> {
    c: &'a Compiled,
    sig: Vec<f64>,
    tmp: Vec<f64>,
    hist: Vec<Option<History>>,
    u_ext: Vec<f64>,
    evals: u64,
}

impl<'a> Machine<'a> {
    pub fn new(c: &'a Compiled, u_ext: Vec<f64>) -> Machine<'a> {
        let hist = c
            .nodes
            .iter()
            .map(|n| {
                matches!(n.kind, BlockKind::TransportDelay { .. }).then(|| History {
                    w: n.in_w.first().copied().unwrap_or(1),
                    ..History::default()
                })
            })
            .collect();
        Machine {
            c,
            sig: vec![0.0; c.sig_len],
            tmp: Vec::new(),
            hist,
            u_ext,
            evals: 0,
        }
    }

    /// Computes every block's outputs at `(t, xc, xd)`.
    pub fn eval_outputs(&mut self, t: f64, xc: &[f64], xd: &[f64]) -> Result<()> {
        self.evals += 1;
        let c = self.c;
        for &i in &c.order {
            let Some(nd) = c.nodes.get(i) else { continue };
            if nd.out_len == 0 {
                continue;
            }
            self.tmp.clear();
            self.tmp.resize(nd.out_len, 0.0);
            {
                let ins = gather(c, nd, &self.sig);
                let ctx = Ctx {
                    t,
                    u_ext: &self.u_ext,
                    hist: self.hist.get(i).and_then(Option::as_ref),
                };
                blocks::outputs(nd, &ctx, &ins, xc_of(nd, xc), xd_of(nd, xd), &mut self.tmp);
            }
            if self.tmp.iter().any(|v| !v.is_finite()) {
                return Err(BlockError::NonFinite {
                    block: nd.name.clone(),
                    t,
                });
            }
            if let Some(dst) = self.sig.get_mut(nd.out_base..nd.out_base + nd.out_len) {
                dst.copy_from_slice(&self.tmp);
            }
        }
        Ok(())
    }

    /// Outputs, then the derivative of every continuous state.
    pub fn eval_deriv(&mut self, t: f64, xc: &[f64], xd: &[f64], dx: &mut [f64]) -> Result<()> {
        self.eval_outputs(t, xc, xd)?;
        self.derivatives_now(xc, dx);
        Ok(())
    }

    /// Derivatives from the signals already computed.
    fn derivatives_now(&self, xc: &[f64], dx: &mut [f64]) {
        dx.fill(0.0);
        for nd in &self.c.nodes {
            if nd.n_xc == 0 {
                continue;
            }
            let ins = gather(self.c, nd, &self.sig);
            if let Some(d) = dx.get_mut(nd.xc_off..nd.xc_off + nd.n_xc) {
                blocks::derivatives(nd, &ins, xc_of(nd, xc), d);
            }
        }
    }

    /// The zero-crossing functions at the signals already computed.
    fn crossings_now(&self, xc: &[f64], g: &mut Vec<f64>) {
        g.clear();
        for nd in &self.c.nodes {
            let ins = gather(self.c, nd, &self.sig);
            blocks::crossings(nd, &ins, xc_of(nd, xc), g);
        }
    }

    /// Latches zero-order holds (at a hit) and relays until the outputs settle.
    fn latch(&mut self, t: f64, xc: &[f64], xd: &mut [f64], hit: &[bool]) -> Result<()> {
        for _ in 0..4 {
            let mut changed = false;
            for (i, nd) in self.c.nodes.iter().enumerate() {
                if nd.n_xd == 0 {
                    continue;
                }
                let is_hit = hit.get(i).copied().unwrap_or(false);
                let ins = gather(self.c, nd, &self.sig);
                if let Some(x) = xd.get_mut(nd.xd_off..nd.xd_off + nd.n_xd) {
                    changed |= blocks::latch(nd, is_hit, &ins, x);
                }
            }
            if !changed {
                break;
            }
            self.eval_outputs(t, xc, xd)?;
        }
        Ok(())
    }

    /// Whether any block keeps a signal history (transport delays).
    fn keeps_history(&self) -> bool {
        self.hist.iter().any(Option::is_some)
    }

    fn record_history(&mut self, t: f64) {
        for (i, nd) in self.c.nodes.iter().enumerate() {
            if let Some(Some(h)) = self.hist.get_mut(i) {
                let u = nd
                    .src
                    .first()
                    .map(|&s| port_slice(self.c, s, &self.sig))
                    .unwrap_or(&[]);
                h.push(t, u);
            }
        }
    }

    pub fn outport_values(&self) -> Vec<f64> {
        let mut y = Vec::new();
        for &i in &self.c.outports {
            if let Some(nd) = self.c.nodes.get(i) {
                if let Some(&s) = nd.src.first() {
                    y.extend_from_slice(port_slice(self.c, s, &self.sig));
                }
            }
        }
        y
    }
}

// ───────────────────────── Runge–Kutta tableaus ─────────────────────────

struct Tab {
    c: &'static [f64],
    a: &'static [&'static [f64]],
    b: &'static [f64],
    bhat: Option<&'static [f64]>,
    /// Order of the error estimate plus one (the exponent in the step controller).
    exponent: f64,
}

const EULER: Tab = Tab {
    c: &[0.0],
    a: &[&[]],
    b: &[1.0],
    bhat: None,
    exponent: 2.0,
};
const HEUN: Tab = Tab {
    c: &[0.0, 1.0],
    a: &[&[], &[1.0]],
    b: &[0.5, 0.5],
    bhat: None,
    exponent: 3.0,
};
const BS3: Tab = Tab {
    c: &[0.0, 0.5, 0.75],
    a: &[&[], &[0.5], &[0.0, 0.75]],
    b: &[2.0 / 9.0, 1.0 / 3.0, 4.0 / 9.0],
    bhat: None,
    exponent: 4.0,
};
const RK4: Tab = Tab {
    c: &[0.0, 0.5, 0.5, 1.0],
    a: &[&[], &[0.5], &[0.0, 0.5], &[0.0, 0.0, 1.0]],
    b: &[1.0 / 6.0, 1.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0],
    bhat: None,
    exponent: 5.0,
};
const BS23: Tab = Tab {
    c: &[0.0, 0.5, 0.75, 1.0],
    a: &[
        &[],
        &[0.5],
        &[0.0, 0.75],
        &[2.0 / 9.0, 1.0 / 3.0, 4.0 / 9.0],
    ],
    b: &[2.0 / 9.0, 1.0 / 3.0, 4.0 / 9.0, 0.0],
    bhat: Some(&[7.0 / 24.0, 0.25, 1.0 / 3.0, 0.125]),
    exponent: 3.0,
};
const DOPRI5: Tab = Tab {
    c: &[0.0, 0.2, 0.3, 0.8, 8.0 / 9.0, 1.0, 1.0],
    a: &[
        &[],
        &[0.2],
        &[3.0 / 40.0, 9.0 / 40.0],
        &[44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0],
        &[
            19372.0 / 6561.0,
            -25360.0 / 2187.0,
            64448.0 / 6561.0,
            -212.0 / 729.0,
        ],
        &[
            9017.0 / 3168.0,
            -355.0 / 33.0,
            46732.0 / 5247.0,
            49.0 / 176.0,
            -5103.0 / 18656.0,
        ],
        &[
            35.0 / 384.0,
            0.0,
            500.0 / 1113.0,
            125.0 / 192.0,
            -2187.0 / 6784.0,
            11.0 / 84.0,
        ],
    ],
    b: &[
        35.0 / 384.0,
        0.0,
        500.0 / 1113.0,
        125.0 / 192.0,
        -2187.0 / 6784.0,
        11.0 / 84.0,
        0.0,
    ],
    bhat: Some(&[
        5179.0 / 57600.0,
        0.0,
        7571.0 / 16695.0,
        393.0 / 640.0,
        -92097.0 / 339200.0,
        187.0 / 2100.0,
        1.0 / 40.0,
    ]),
    exponent: 5.0,
};

fn tableau(m: Method) -> Option<&'static Tab> {
    Some(match m {
        Method::Euler => &EULER,
        Method::Heun => &HEUN,
        Method::Bs3 => &BS3,
        Method::Rk4 => &RK4,
        Method::Bs23 => &BS23,
        Method::Dopri5 => &DOPRI5,
        Method::Rosenbrock23 => return None,
    })
}

/// The result of one trial step.
struct Trial {
    x: Vec<f64>,
    err: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn rk_step(
    m: &mut Machine,
    tab: &Tab,
    t: f64,
    h: f64,
    land: bool,
    xc: &[f64],
    xd: &[f64],
    k1: &[f64],
) -> Result<Trial> {
    let n = xc.len();
    let mut k: Vec<Vec<f64>> = Vec::with_capacity(tab.b.len());
    k.push(k1.to_vec());
    let mut xs = vec![0.0; n];
    for st in 1..tab.b.len() {
        let row = tab.a.get(st).copied().unwrap_or(&[]);
        for i in 0..n {
            let acc: f64 = row
                .iter()
                .enumerate()
                .map(|(j, &a)| a * k.get(j).and_then(|kj| kj.get(i)).copied().unwrap_or(0.0))
                .sum();
            if let Some(x) = xs.get_mut(i) {
                *x = xc.get(i).copied().unwrap_or(0.0) + h * acc;
            }
        }
        let c = tab.c.get(st).copied().unwrap_or(0.0);
        let ts = if land && c == 1.0 {
            blocks::left_limit(t + h)
        } else {
            t + c * h
        };
        let mut ks = vec![0.0; n];
        m.eval_deriv(ts, &xs, xd, &mut ks)?;
        k.push(ks);
    }
    let mut x = vec![0.0; n];
    let mut err = vec![0.0; n];
    for i in 0..n {
        let mut sol = 0.0;
        let mut e = 0.0;
        for (j, kj) in k.iter().enumerate() {
            let kv = kj.get(i).copied().unwrap_or(0.0);
            let b = tab.b.get(j).copied().unwrap_or(0.0);
            sol += b * kv;
            if let Some(bh) = tab.bhat {
                e += (b - bh.get(j).copied().unwrap_or(0.0)) * kv;
            }
        }
        if let Some(v) = x.get_mut(i) {
            *v = xc.get(i).copied().unwrap_or(0.0) + h * sol;
        }
        if let Some(v) = err.get_mut(i) {
            *v = h * e;
        }
    }
    Ok(Trial { x, err })
}

fn ros_step(
    m: &mut Machine,
    t: f64,
    h: f64,
    land: bool,
    xc: &[f64],
    xd: &[f64],
    f0: &[f64],
) -> Result<Trial> {
    let n = xc.len();
    if n > 400 {
        return Err(BlockError::Solver(format!(
            "the stiff solver handles up to 400 states, this model has {n}; use ode45 or ode23"
        )));
    }
    let d = 1.0 / (2.0 + 2f64.sqrt());
    let e32 = 6.0 + 2f64.sqrt();
    // Jacobian by forward differences, time derivative likewise.
    let mut jac = vec![0.0; n * n];
    let mut xp = xc.to_vec();
    let mut fp = vec![0.0; n];
    for j in 0..n {
        let xj = xc.get(j).copied().unwrap_or(0.0);
        let delta = 1e-7 * xj.abs().max(1e-3);
        if let Some(v) = xp.get_mut(j) {
            *v = xj + delta;
        }
        m.eval_deriv(t, &xp, xd, &mut fp)?;
        for i in 0..n {
            if let Some(a) = jac.get_mut(i * n + j) {
                *a =
                    (fp.get(i).copied().unwrap_or(0.0) - f0.get(i).copied().unwrap_or(0.0)) / delta;
            }
        }
        if let Some(v) = xp.get_mut(j) {
            *v = xj;
        }
    }
    let dt = 1e-7 * t.abs().max(h).max(1e-3);
    m.eval_deriv(t + dt, xc, xd, &mut fp)?;
    let dfdt: Vec<f64> = (0..n)
        .map(|i| (fp.get(i).copied().unwrap_or(0.0) - f0.get(i).copied().unwrap_or(0.0)) / dt)
        .collect();
    let mut w = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            let id = if i == j { 1.0 } else { 0.0 };
            if let Some(a) = w.get_mut(i * n + j) {
                *a = id - h * d * jac.get(i * n + j).copied().unwrap_or(0.0);
            }
        }
    }
    let lu = Matrix::new(n, n, w)
        .and_then(|mat| Lu::factor(&mat))
        .map_err(|e| {
            BlockError::Solver(format!(
                "the stiff solver's Newton matrix is singular ({e}); reduce the step"
            ))
        })?;
    let solve = |rhs: Vec<f64>| -> Result<Vec<f64>> {
        lu.solve(&rhs)
            .map_err(|e| BlockError::Solver(e.to_string()))
    };
    let tt: Vec<f64> = dfdt.iter().map(|v| h * d * v).collect();
    let k1 = solve((0..n).map(|i| f0[i] + tt[i]).collect())?;
    let xm: Vec<f64> = (0..n).map(|i| xc[i] + 0.5 * h * k1[i]).collect();
    let mut f1 = vec![0.0; n];
    m.eval_deriv(t + 0.5 * h, &xm, xd, &mut f1)?;
    let r2 = solve((0..n).map(|i| f1[i] - k1[i]).collect())?;
    let k2: Vec<f64> = (0..n).map(|i| r2[i] + k1[i]).collect();
    let x: Vec<f64> = (0..n).map(|i| xc[i] + h * k2[i]).collect();
    let mut f2 = vec![0.0; n];
    let te = if land {
        blocks::left_limit(t + h)
    } else {
        t + h
    };
    m.eval_deriv(te, &x, xd, &mut f2)?;
    let k3 = solve(
        (0..n)
            .map(|i| f2[i] - e32 * (k2[i] - f1[i]) - 2.0 * (k1[i] - f0[i]) + tt[i])
            .collect(),
    )?;
    let err: Vec<f64> = (0..n)
        .map(|i| h / 6.0 * (k1[i] - 2.0 * k2[i] + k3[i]))
        .collect();
    Ok(Trial { x, err })
}

// ───────────────────────── the run ─────────────────────────

struct Logger {
    node: usize,
    port: usize,
    name: String,
    width: usize,
}

fn loggers(c: &Compiled) -> Vec<Logger> {
    let mut v = Vec::new();
    for (i, nd) in c.nodes.iter().enumerate() {
        let logs = matches!(
            nd.kind,
            BlockKind::Scope { .. }
                | BlockKind::ToWorkspace
                | BlockKind::Display
                | BlockKind::Outport { .. }
        );
        if !logs {
            continue;
        }
        for (p, &w) in nd.in_w.iter().enumerate() {
            let name = if nd.in_w.len() > 1 {
                format!("{}:{}", nd.name, p + 1)
            } else {
                nd.name.clone()
            };
            v.push(Logger {
                node: i,
                port: p,
                name,
                width: w,
            });
        }
    }
    v
}

fn error_norm(err: &[f64], x0: &[f64], x1: &[f64], rtol: f64, atol: f64) -> f64 {
    let mut m = 0.0f64;
    for (i, e) in err.iter().enumerate() {
        let scale = atol
            + rtol
                * x0.get(i)
                    .copied()
                    .unwrap_or(0.0)
                    .abs()
                    .max(x1.get(i).copied().unwrap_or(0.0).abs());
        let v = e.abs() / scale;
        if v.is_nan() {
            return f64::INFINITY;
        }
        m = m.max(v);
    }
    m
}

impl Compiled {
    /// Runs the model.
    pub fn simulate(&self, opts: &SimOptions) -> Result<SimResult> {
        run(self, opts)
    }

    /// The initial continuous and discrete states.
    pub fn initial_state(&self) -> (Vec<f64>, Vec<f64>) {
        let mut xc = vec![0.0; self.n_xc];
        let mut xd = vec![0.0; self.n_xd];
        for nd in &self.nodes {
            if let Some(x) = xc.get_mut(nd.xc_off..nd.xc_off + nd.n_xc) {
                blocks::init_xc(nd, x);
            }
            if let Some(x) = xd.get_mut(nd.xd_off..nd.xd_off + nd.n_xd) {
                blocks::init_xd(nd, x);
            }
        }
        (xc, xd)
    }

    /// Number of top-level `Inport` blocks (each one scalar).
    pub fn input_count(&self) -> usize {
        self.inports.len()
    }

    /// Number of scalar values the top-level `Outport` blocks produce.
    pub fn output_width(&self) -> usize {
        self.outports
            .iter()
            .filter_map(|&i| self.nodes.get(i))
            .map(|n| n.in_w.first().copied().unwrap_or(0))
            .sum()
    }

    /// The state derivatives and the top-level outputs at a point (used for linearisation).
    pub fn evaluate(
        &self,
        t: f64,
        xc: &[f64],
        xd: &[f64],
        u: &[f64],
    ) -> Result<(Vec<f64>, Vec<f64>)> {
        if xc.len() != self.n_xc || xd.len() != self.n_xd {
            return Err(BlockError::Solver(
                "the state vector has the wrong length".into(),
            ));
        }
        let mut m = Machine::new(self, u.to_vec());
        let mut dx = vec![0.0; self.n_xc];
        m.eval_deriv(t, xc, xd, &mut dx)?;
        Ok((dx, m.outport_values()))
    }
}

#[allow(clippy::too_many_lines)]
fn run(c: &Compiled, opts: &SimOptions) -> Result<SimResult> {
    let cfg = &c.cfg;
    let (t0, tf) = (cfg.start_time, cfg.stop_time);
    let span = tf - t0;
    let fixed = cfg.method.is_fixed();
    let (mut xc, mut xd) = c.initial_state();
    let mut m = Machine::new(c, opts.inputs.clone());
    let min_delay = c
        .nodes
        .iter()
        .filter_map(|n| match n.kind {
            BlockKind::TransportDelay { delay, .. } => Some(delay),
            _ => None,
        })
        .fold(f64::INFINITY, f64::min);
    let mut max_step = cfg.max_step.unwrap_or(span / 50.0).min(min_delay);
    let step0 = cfg
        .step
        .unwrap_or(if fixed { span / 1000.0 } else { span / 100.0 });
    if fixed && step0 > min_delay {
        return Err(BlockError::Solver(format!(
            "the step {step0} is larger than the shortest transport delay {min_delay}"
        )));
    }
    if fixed {
        max_step = step0;
    }
    let mut h_ctrl = step0.min(max_step);

    let logs = loggers(c);
    let mut data: Vec<Vec<f64>> = vec![Vec::new(); logs.len()];
    let mut time: Vec<f64> = Vec::new();
    let mut stats = Stats::default();
    let mut stopped_by = None;

    let hitters: Vec<usize> = c
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.ts.is_some())
        .map(|(i, _)| i)
        .collect();
    let mut counts = vec![0u64; c.nodes.len()];
    // The state a discrete block computed at its last sample; it takes effect at the next one.
    let mut pending: Vec<Option<Vec<f64>>> = vec![None; c.nodes.len()];
    let mut bp = 0usize;
    let mut t = t0;
    let mut k1 = vec![0.0; xc.len()];
    let mut g_old: Vec<f64> = Vec::new();
    let mut g_new: Vec<f64> = Vec::new();

    loop {
        let tol = blocks::time_tol(t);
        // Which discrete blocks sample now.
        let mut hit = vec![false; c.nodes.len()];
        for &i in &hitters {
            if let Some(ts) = c.nodes.get(i).and_then(|n| n.ts) {
                let next = t0 + counts.get(i).copied().unwrap_or(0) as f64 * ts;
                if next <= t + tol {
                    if let Some(h) = hit.get_mut(i) {
                        *h = true;
                    }
                }
            }
        }
        for (i, h) in hit.iter().enumerate() {
            if !*h {
                continue;
            }
            if let (Some(nd), Some(Some(x))) =
                (c.nodes.get(i), pending.get_mut(i).map(Option::take))
            {
                if let Some(dst) = xd.get_mut(nd.xd_off..nd.xd_off + nd.n_xd) {
                    dst.copy_from_slice(&x);
                }
            }
        }
        m.eval_outputs(t, &xc, &xd)?;
        m.latch(t, &xc, &mut xd, &hit)?;
        m.record_history(t);
        // Record.
        time.push(t);
        if time.len() > MAX_POINTS {
            return Err(BlockError::Solver(format!(
                "more than {MAX_POINTS} points recorded; shorten the run or lengthen the step"
            )));
        }
        for (lg, d) in logs.iter().zip(data.iter_mut()) {
            if let Some(nd) = c.nodes.get(lg.node) {
                if let Some(&s) = nd.src.get(lg.port) {
                    d.extend_from_slice(port_slice(c, s, &m.sig));
                }
            }
        }
        for nd in &c.nodes {
            let ins = gather(c, nd, &m.sig);
            if blocks::stop_requested(nd, &ins) {
                stopped_by = Some(nd.name.clone());
            }
        }
        if stopped_by.is_some() || t >= tf - tol {
            break;
        }
        // Discrete updates for the next sample.
        for &i in &hitters {
            if !hit.get(i).copied().unwrap_or(false) {
                continue;
            }
            if let Some(nd) = c.nodes.get(i) {
                let ins: Vec<Vec<f64>> = gather(c, nd, &m.sig)
                    .into_iter()
                    .map(<[f64]>::to_vec)
                    .collect();
                let refs: Vec<&[f64]> = ins.iter().map(Vec::as_slice).collect();
                if let Some(x) = xd.get(nd.xd_off..nd.xd_off + nd.n_xd) {
                    let mut next = x.to_vec();
                    blocks::update(nd, &refs, &mut next);
                    if let Some(p) = pending.get_mut(i) {
                        *p = Some(next);
                    }
                }
                if let (Some(ts), Some(k)) = (nd.ts, counts.get_mut(i)) {
                    *k += 1;
                    while t0 + *k as f64 * ts <= t + tol {
                        *k += 1;
                    }
                }
            }
        }
        // Next event: a sample hit, a switching time or the end.
        let mut ne = tf;
        for &i in &hitters {
            if let Some(ts) = c.nodes.get(i).and_then(|n| n.ts) {
                ne = ne.min(t0 + counts.get(i).copied().unwrap_or(0) as f64 * ts);
            }
        }
        while c.breakpoints.get(bp).is_some_and(|&b| b <= t + tol) {
            bp += 1;
        }
        if let Some(&b) = c.breakpoints.get(bp) {
            ne = ne.min(b);
        }
        let ne = ne.min(tf);

        if xc.is_empty() {
            // No continuous states: advance by the step (so crossings and stop conditions
            // are seen, and a fixed-step run logs every step), landing on events exactly.
            stats.steps += 1;
            if stats.steps > MAX_STEPS {
                return Err(BlockError::Solver(format!(
                    "more than {MAX_STEPS} steps; stopped at t = {t:e}"
                )));
            }
            let mut target = (t + h_ctrl.min(max_step)).min(ne);
            if ne - target <= tol {
                target = ne;
            }
            if !fixed {
                m.crossings_now(&xc, &mut g_old);
                m.eval_outputs(target, &xc, &xd)?;
                m.crossings_now(&xc, &mut g_new);
                let crossed = |a: &[f64], b: &[f64]| a.iter().zip(b).any(|(x, y)| x * y < 0.0);
                if crossed(&g_old, &g_new) {
                    stats.zero_crossings += 1;
                    let (mut lo, mut hi) = (t, target);
                    for _ in 0..80 {
                        if hi - lo <= 4e-16 * t.abs().max(1.0) {
                            break;
                        }
                        let mid = 0.5 * (lo + hi);
                        m.eval_outputs(mid, &xc, &xd)?;
                        m.crossings_now(&xc, &mut g_new);
                        if crossed(&g_old, &g_new) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    target = hi;
                }
            }
            if m.keeps_history() && target >= ne {
                m.eval_outputs(blocks::left_limit(target), &xc, &xd)?;
                m.record_history(target);
            }
            t = target;
            continue;
        }

        // One integration step from t towards the next event.
        m.eval_deriv(t, &xc, &xd, &mut k1)?;
        let variable = !fixed;
        if variable {
            m.crossings_now(&xc, &mut g_old);
        }
        let mut h = h_ctrl.min(max_step);
        let (x_new, h_used, landed) = loop {
            let remaining = ne - t;
            let land = remaining <= h * (1.0 + 1e-9);
            if land {
                h = remaining;
            }
            if h < 1e-14 * t.abs().max(1.0) && remaining > 1e-14 * t.abs().max(1.0) {
                return Err(BlockError::Solver(format!(
                    "step size became too small at t = {t:e}; the problem may be stiff (try the stiff solver) or have a singularity"
                )));
            }
            stats.steps += 1;
            if stats.steps > MAX_STEPS {
                return Err(BlockError::Solver(format!(
                    "more than {MAX_STEPS} steps; stopped at t = {t:e}"
                )));
            }
            let trial = match tableau(cfg.method) {
                Some(tab) => rk_step(&mut m, tab, t, h, land, &xc, &xd, &k1),
                None => ros_step(&mut m, t, h, land, &xc, &xd, &k1),
            };
            let trial = match trial {
                Ok(tr) => tr,
                Err(BlockError::NonFinite { .. }) if variable && h > 1e-9 * span => {
                    stats.rejected += 1;
                    h *= 0.25;
                    continue;
                }
                Err(e) => return Err(e),
            };
            if trial.x.iter().any(|v| !v.is_finite()) {
                if variable && h > 1e-9 * span {
                    stats.rejected += 1;
                    h *= 0.25;
                    continue;
                }
                return Err(BlockError::Solver(format!(
                    "the states became non-finite at t = {t:e}"
                )));
            }
            let has_err = tableau(cfg.method).is_none_or(|tab| tab.bhat.is_some());
            if variable && has_err {
                let en = error_norm(&trial.err, &xc, &trial.x, cfg.rtol, cfg.atol);
                let exp = tableau(cfg.method).map_or(3.0, |tab| tab.exponent);
                if en > 1.0 {
                    stats.rejected += 1;
                    h *= (0.9 * en.powf(-1.0 / exp)).clamp(0.2, 0.9);
                    continue;
                }
                let factor = if en == 0.0 {
                    5.0
                } else {
                    (0.9 * en.powf(-1.0 / exp)).clamp(0.2, 5.0)
                };
                let next = h * factor;
                h_ctrl = if land && next < h_ctrl { h_ctrl } else { next };
            }
            break (trial.x, h, land);
        };

        // Zero crossings inside the step.
        let mut x_end = x_new;
        let mut h_end = h_used;
        let mut land_end = landed;
        if variable {
            m.eval_outputs(t + h_end, &x_end, &xd)?;
            m.crossings_now(&x_end, &mut g_new);
            let crossed = |g_a: &[f64], g_b: &[f64]| g_a.iter().zip(g_b).any(|(a, b)| a * b < 0.0);
            if crossed(&g_old, &g_new) {
                stats.zero_crossings += 1;
                let (mut lo, mut hi) = (0.0f64, h_end);
                let mut hi_x = x_end.clone();
                for _ in 0..80 {
                    if hi - lo <= 1e-13 * h_end.max(1e-300) || hi - lo <= 4e-16 * t.abs().max(1.0) {
                        break;
                    }
                    let mid = 0.5 * (lo + hi);
                    let tr = match tableau(cfg.method) {
                        Some(tab) => rk_step(&mut m, tab, t, mid, false, &xc, &xd, &k1)?,
                        None => ros_step(&mut m, t, mid, false, &xc, &xd, &k1)?,
                    };
                    m.eval_outputs(t + mid, &tr.x, &xd)?;
                    m.crossings_now(&tr.x, &mut g_new);
                    if crossed(&g_old, &g_new) {
                        hi = mid;
                        hi_x = tr.x;
                    } else {
                        lo = mid;
                    }
                }
                x_end = hi_x;
                h_end = hi;
                land_end = false;
            }
        }
        xc = x_end;
        if land_end && m.keeps_history() {
            m.eval_outputs(blocks::left_limit(ne), &xc, &xd)?;
            m.record_history(ne);
        }
        t = if land_end { ne } else { t + h_end };
    }

    let signals = logs
        .into_iter()
        .zip(data)
        .map(|(l, d)| Signal {
            name: l.name,
            width: l.width,
            data: d,
        })
        .collect();
    stats.evaluations = m.evals;
    Ok(SimResult {
        time,
        signals,
        final_state: xc,
        final_discrete: xd,
        stats,
        stopped_by,
    })
}
