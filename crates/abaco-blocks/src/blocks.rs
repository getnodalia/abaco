// SPDX-License-Identifier: MIT OR Apache-2.0
//! What each block computes: its outputs, its state derivatives, its discrete updates and the
//! functions whose sign changes mark a discontinuity the solver must land on.

use std::f64::consts::PI;

use crate::compile::{product_ops, sum_signs, Lti, Node};
use crate::model::{
    BlockKind, DiscreteRule, Extremum, LogicOp, MathFunc, RelOp, RoundMode, SwitchRule, WaveShape,
};

/// The values a transport delay has seen, one row per accepted time.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    pub t: Vec<f64>,
    pub v: Vec<f64>,
    pub w: usize,
}

impl History {
    pub fn push(&mut self, t: f64, v: &[f64]) {
        let n = self.t.len();
        if let Some(&last) = self.t.last() {
            if t <= last {
                // The same instant again. The first repeat records a jump (the value just
                // before the event, then the value after it); further repeats keep the newest.
                let before_last = n.checked_sub(2).and_then(|i| self.t.get(i)).copied();
                if before_last.is_some_and(|b| b >= t) || t < last {
                    self.t.truncate(n - 1);
                    self.v.truncate(self.v.len().saturating_sub(self.w));
                }
            }
        }
        self.t.push(t);
        self.v.extend_from_slice(v);
    }

    /// The value at `t`, interpolated, or `None` before the first record.
    pub fn at(&self, t: f64, out: &mut [f64]) -> bool {
        let n = self.t.len();
        let (Some(&first), Some(&last)) = (self.t.first(), self.t.last()) else {
            return false;
        };
        if t < first {
            return false;
        }
        let row = |i: usize| self.v.get(i * self.w..(i + 1) * self.w).unwrap_or(&[]);
        if n == 1 || t >= last {
            // Extrapolate the last segment when asked slightly beyond the record.
            let (i0, i1) = if n >= 2 { (n - 2, n - 1) } else { (0, 0) };
            let (t0, t1) = (
                self.t.get(i0).copied().unwrap_or(0.0),
                self.t.get(i1).copied().unwrap_or(0.0),
            );
            let s = if t1 > t0 { (t - t1) / (t1 - t0) } else { 0.0 };
            for (k, o) in out.iter_mut().enumerate() {
                let a = row(i0).get(k).copied().unwrap_or(0.0);
                let b = row(i1).get(k).copied().unwrap_or(0.0);
                *o = b + s * (b - a);
            }
            return true;
        }
        let hi = self.t.partition_point(|&x| x <= t).clamp(1, n - 1);
        let lo = hi - 1;
        let (t0, t1) = (
            self.t.get(lo).copied().unwrap_or(0.0),
            self.t.get(hi).copied().unwrap_or(1.0),
        );
        let s = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
        for (k, o) in out.iter_mut().enumerate() {
            let a = row(lo).get(k).copied().unwrap_or(0.0);
            let b = row(hi).get(k).copied().unwrap_or(0.0);
            *o = a + s * (b - a);
        }
        true
    }
}

/// What a block can see while it computes.
pub(crate) struct Ctx<'a> {
    pub t: f64,
    pub u_ext: &'a [f64],
    pub hist: Option<&'a History>,
}

/// Element `i` of a signal, with a scalar broadcast.
#[inline]
fn el(v: &[f64], i: usize) -> f64 {
    if v.len() == 1 {
        v.first().copied().unwrap_or(0.0)
    } else {
        v.get(i).copied().unwrap_or(0.0)
    }
}

/// The smallest time difference the sources treat as "the same instant".
pub(crate) fn time_tol(t: f64) -> f64 {
    1e-15 * t.abs().max(1.0)
}

/// A time just before `t`, used to evaluate the left limit at a discontinuity.
pub(crate) fn left_limit(t: f64) -> f64 {
    t - 3.5e-15 * t.abs().max(1.0)
}

/// The fractional part of `x`, with values within `tol` below 1 counted as 0 (the new cycle).
fn frac(x: f64, tol: f64) -> f64 {
    let f = x - x.floor();
    if f > 1.0 - tol {
        0.0
    } else {
        f
    }
}

fn apply_math(f: MathFunc, u: f64) -> f64 {
    match f {
        MathFunc::Exp => u.exp(),
        MathFunc::Log => u.ln(),
        MathFunc::Log10 => u.log10(),
        MathFunc::Sqrt => u.sqrt(),
        MathFunc::Square => u * u,
        MathFunc::Reciprocal => 1.0 / u,
        MathFunc::Sin => u.sin(),
        MathFunc::Cos => u.cos(),
        MathFunc::Tan => u.tan(),
        MathFunc::Asin => u.asin(),
        MathFunc::Acos => u.acos(),
        MathFunc::Atan => u.atan(),
        MathFunc::Sinh => u.sinh(),
        MathFunc::Cosh => u.cosh(),
        MathFunc::Tanh => u.tanh(),
        MathFunc::Pow10 => 10f64.powf(u),
        MathFunc::Abs => u.abs(),
        MathFunc::Sign => {
            if u > 0.0 {
                1.0
            } else if u < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
    }
}

fn rel(op: RelOp, a: f64, b: f64) -> bool {
    match op {
        RelOp::Eq => a == b,
        RelOp::Ne => a != b,
        RelOp::Lt => a < b,
        RelOp::Le => a <= b,
        RelOp::Gt => a > b,
        RelOp::Ge => a >= b,
    }
}

fn lti_apply(l: &Lti, x: &[f64], u: &[f64], y: &mut [f64]) {
    for i in 0..l.p {
        let mut s = 0.0;
        for j in 0..l.n {
            s += l.c.get(i * l.n + j).copied().unwrap_or(0.0) * x.get(j).copied().unwrap_or(0.0);
        }
        for j in 0..l.m {
            s += l.d.get(i * l.m + j).copied().unwrap_or(0.0) * u.get(j).copied().unwrap_or(0.0);
        }
        if let Some(o) = y.get_mut(i) {
            *o = s;
        }
    }
}

/// `A x + B u` (continuous derivative or discrete next state).
fn lti_state(l: &Lti, x: &[f64], u: &[f64], out: &mut [f64]) {
    for i in 0..l.n {
        let mut s = 0.0;
        for j in 0..l.n {
            s += l.a.get(i * l.n + j).copied().unwrap_or(0.0) * x.get(j).copied().unwrap_or(0.0);
        }
        for j in 0..l.m {
            s += l.b.get(i * l.m + j).copied().unwrap_or(0.0) * u.get(j).copied().unwrap_or(0.0);
        }
        if let Some(o) = out.get_mut(i) {
            *o = s;
        }
    }
}

// ───────────────────── pseudo-random numbers ─────────────────────

const MASK48: u64 = (1 << 48) - 1;

fn lcg_next(state: f64) -> (f64, f64) {
    let s = (state as u64) & MASK48;
    let n = s.wrapping_mul(25_214_903_917).wrapping_add(11) & MASK48;
    ((n >> 16) as f64 / 4_294_967_296.0, n as f64)
}

fn gaussian(state: f64) -> (f64, f64) {
    let (u1, s1) = lcg_next(state);
    let (u2, s2) = lcg_next(s1);
    let r = (-2.0 * (1.0 - u1).ln()).sqrt();
    (r * (2.0 * PI * u2).cos(), s2)
}

// ───────────────────── initial states ─────────────────────

pub(crate) fn init_xc(nd: &Node, x: &mut [f64]) {
    match &nd.kind {
        BlockKind::Integrator { initial, .. } => {
            for (i, v) in x.iter_mut().enumerate() {
                *v = el(initial, i);
            }
        }
        BlockKind::Pid { initial, .. } => {
            if let Some(v) = x.first_mut() {
                *v = *initial;
            }
        }
        BlockKind::TransferFcn { .. }
        | BlockKind::ZeroPole { .. }
        | BlockKind::StateSpace { .. } => {
            if let Some(l) = &nd.lti {
                for (v, s) in x.iter_mut().zip(&l.x0) {
                    *v = *s;
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn init_xd(nd: &Node, x: &mut [f64]) {
    match &nd.kind {
        BlockKind::UnitDelay { initial, .. } => {
            for (i, v) in x.iter_mut().enumerate() {
                *v = el(initial, i);
            }
        }
        BlockKind::Delay { initial, .. } => x.fill(*initial),
        BlockKind::DiscreteIntegrator { initial, .. } => x.fill(*initial),
        BlockKind::DiscreteTransferFcn { .. } | BlockKind::DiscreteStateSpace { .. } => {
            if let Some(l) = &nd.lti {
                for (v, s) in x.iter_mut().zip(&l.x0) {
                    *v = *s;
                }
            }
        }
        BlockKind::Noise {
            mean,
            variance,
            seed,
            ..
        } => {
            let mut s = ((*seed as u64) ^ 0x5DEE_CE66D) & MASK48;
            for _ in 0..3 {
                s = lcg_next(s as f64).1 as u64;
            }
            let (g, s) = gaussian(s as f64);
            if let Some(v) = x.get_mut(0) {
                *v = mean + variance.sqrt() * g;
            }
            if let Some(v) = x.get_mut(1) {
                *v = s;
            }
        }
        _ => {}
    }
}

// ───────────────────── outputs ─────────────────────

/// Computes the outputs of `nd` into `out` (all ports, concatenated).
pub(crate) fn outputs(
    nd: &Node,
    ctx: &Ctx,
    ins: &[&[f64]],
    xc: &[f64],
    xd: &[f64],
    out: &mut [f64],
) {
    use BlockKind as K;
    let t = ctx.t;
    let u0: &[f64] = ins.first().copied().unwrap_or(&[]);
    let map1 = |out: &mut [f64], f: &dyn Fn(f64) -> f64| {
        for (i, o) in out.iter_mut().enumerate() {
            *o = f(el(u0, i));
        }
    };
    match &nd.kind {
        K::Constant { value } => {
            for (o, v) in out.iter_mut().zip(value) {
                *o = *v;
            }
        }
        K::Step {
            time,
            before,
            after,
        } => out.fill(if t >= *time { *after } else { *before }),
        K::Ramp {
            slope,
            start,
            initial,
        } => out.fill(if t <= *start {
            *initial
        } else {
            initial + slope * (t - start)
        }),
        K::Sine {
            amplitude,
            frequency,
            phase,
            bias,
        } => out.fill(amplitude * (frequency * t + phase).sin() + bias),
        K::Pulse {
            amplitude,
            period,
            duty,
            delay,
        } => {
            let v = if t < *delay {
                0.0
            } else {
                let ph = frac((t - delay) / period, time_tol(t) / period) * period;
                if ph < duty / 100.0 * period - time_tol(t) {
                    *amplitude
                } else {
                    0.0
                }
            };
            out.fill(v);
        }
        K::Wave {
            shape,
            amplitude,
            frequency,
        } => {
            let tol_x = time_tol(t) * frequency;
            let ph = frac(frequency * t, tol_x);
            let v = match shape {
                WaveShape::Sine => (2.0 * PI * frequency * t).sin(),
                WaveShape::Square => {
                    if ph < 0.5 - tol_x {
                        1.0
                    } else {
                        -1.0
                    }
                }
                WaveShape::Sawtooth => 2.0 * ph - 1.0,
                WaveShape::Triangle => {
                    if ph < 0.5 {
                        4.0 * ph - 1.0
                    } else {
                        3.0 - 4.0 * ph
                    }
                }
            };
            out.fill(amplitude * v);
        }
        K::Chirp {
            amplitude,
            f0,
            target_time,
            f1,
        } => {
            let phase = 2.0 * PI * (f0 * t + (f1 - f0) * t * t / (2.0 * target_time));
            out.fill(amplitude * phase.sin());
        }
        K::Clock => out.fill(t),
        K::Ground => out.fill(0.0),
        K::Noise { .. } => out.fill(xd.first().copied().unwrap_or(0.0)),
        K::Table {
            times,
            values,
            hold,
        } => {
            let n = times.len();
            let i = times.partition_point(|&x| x <= t);
            let v = if i == 0 {
                values.first().copied().unwrap_or(0.0)
            } else if i >= n {
                values.last().copied().unwrap_or(0.0)
            } else if *hold {
                values.get(i - 1).copied().unwrap_or(0.0)
            } else {
                let (t0, t1) = (
                    times.get(i - 1).copied().unwrap_or(0.0),
                    times.get(i).copied().unwrap_or(1.0),
                );
                let (a, b) = (
                    values.get(i - 1).copied().unwrap_or(0.0),
                    values.get(i).copied().unwrap_or(0.0),
                );
                a + (b - a) * (t - t0) / (t1 - t0)
            };
            out.fill(v);
        }
        K::Inport { index } => {
            out.fill(
                ctx.u_ext
                    .get(index.saturating_sub(1))
                    .copied()
                    .unwrap_or(0.0),
            );
        }
        K::Identity => {
            for (o, v) in out.iter_mut().zip(u0) {
                *o = *v;
            }
        }
        K::Scope { .. }
        | K::ToWorkspace
        | K::Display
        | K::Terminator
        | K::Stop
        | K::Outport { .. }
        | K::Goto { .. }
        | K::From { .. }
        | K::Subsystem { .. } => {}
        K::Sum { signs } => {
            let s = sum_signs("", signs).unwrap_or_default();
            for (i, o) in out.iter_mut().enumerate() {
                let mut acc = 0.0;
                for (k, u) in ins.iter().enumerate() {
                    let x = el(u, i);
                    if s.get(k) == Some(&'-') {
                        acc -= x;
                    } else {
                        acc += x;
                    }
                }
                *o = acc;
            }
        }
        K::Product { ops } => {
            let s = product_ops("", ops).unwrap_or_default();
            for (i, o) in out.iter_mut().enumerate() {
                let mut acc = 1.0;
                for (k, u) in ins.iter().enumerate() {
                    let x = el(u, i);
                    if s.get(k) == Some(&'/') {
                        acc /= x;
                    } else {
                        acc *= x;
                    }
                }
                *o = acc;
            }
        }
        K::Gain { gain } => map1(out, &|u| gain * u),
        K::MatrixGain { matrix } => {
            for (i, o) in out.iter_mut().enumerate() {
                *o = matrix
                    .get(i)
                    .map(|row| row.iter().zip(u0).map(|(a, b)| a * b).sum())
                    .unwrap_or(0.0);
            }
        }
        K::Math { func } => map1(out, &|u| apply_math(*func, u)),
        K::Power { exponent } => map1(out, &|u| u.powf(*exponent)),
        K::Atan2 => {
            let u1: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            for (i, o) in out.iter_mut().enumerate() {
                *o = el(u0, i).atan2(el(u1, i));
            }
        }
        K::Mod => {
            let u1: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            for (i, o) in out.iter_mut().enumerate() {
                let (a, b) = (el(u0, i), el(u1, i));
                *o = if b == 0.0 { a } else { a - (a / b).floor() * b };
            }
        }
        K::Saturation { lower, upper } => map1(out, &|u| u.clamp(*lower, *upper)),
        K::DeadZone { start, end } => map1(out, &|u| {
            if u < *start {
                u - start
            } else if u > *end {
                u - end
            } else {
                0.0
            }
        }),
        K::Bias { bias } => map1(out, &|u| u + bias),
        K::Rounding { mode } => map1(out, &|u| match mode {
            RoundMode::Floor => u.floor(),
            RoundMode::Ceil => u.ceil(),
            RoundMode::Round => u.round(),
            RoundMode::Fix => u.trunc(),
        }),
        K::MinMax { mode, .. } => {
            for (i, o) in out.iter_mut().enumerate() {
                let mut it = ins.iter().map(|u| el(u, i));
                let first = it.next().unwrap_or(0.0);
                *o = it.fold(first, |a, b| match mode {
                    Extremum::Min => a.min(b),
                    Extremum::Max => a.max(b),
                });
            }
        }
        K::Relational { op } => {
            let u1: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            for (i, o) in out.iter_mut().enumerate() {
                *o = f64::from(rel(*op, el(u0, i), el(u1, i)));
            }
        }
        K::CompareConst { op, value } => map1(out, &|u| f64::from(rel(*op, u, *value))),
        K::Logical { op, .. } => {
            for (i, o) in out.iter_mut().enumerate() {
                let bits: Vec<bool> = ins.iter().map(|u| el(u, i) != 0.0).collect();
                let all = bits.iter().all(|&b| b);
                let any = bits.iter().any(|&b| b);
                let odd = bits.iter().filter(|&&b| b).count() % 2 == 1;
                *o = f64::from(match op {
                    LogicOp::And => all,
                    LogicOp::Or => any,
                    LogicOp::Xor => odd,
                    LogicOp::Nand => !all,
                    LogicOp::Nor => !any,
                    LogicOp::Not => !any,
                });
            }
        }
        K::Switch { rule, threshold } => {
            let c: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            let u2: &[f64] = ins.get(2).copied().unwrap_or(&[]);
            for (i, o) in out.iter_mut().enumerate() {
                let x = el(c, i);
                let pass = match rule {
                    SwitchRule::Ge => x >= *threshold,
                    SwitchRule::Gt => x > *threshold,
                    SwitchRule::NonZero => x != 0.0,
                };
                *o = if pass { el(u0, i) } else { el(u2, i) };
            }
        }
        K::Relay {
            on_output,
            off_output,
            ..
        } => {
            for (o, m) in out.iter_mut().zip(xd) {
                *o = if *m > 0.5 { *on_output } else { *off_output };
            }
        }
        K::Quantizer { interval } => map1(out, &|u| interval * (u / interval).round()),
        K::Lookup1D { x, y } => map1(out, &|u| {
            let n = x.len();
            let i = x.partition_point(|&p| p <= u);
            if i == 0 {
                y.first().copied().unwrap_or(0.0)
            } else if i >= n {
                y.last().copied().unwrap_or(0.0)
            } else {
                let (x0, x1) = (
                    x.get(i - 1).copied().unwrap_or(0.0),
                    x.get(i).copied().unwrap_or(1.0),
                );
                let (y0, y1) = (
                    y.get(i - 1).copied().unwrap_or(0.0),
                    y.get(i).copied().unwrap_or(0.0),
                );
                y0 + (y1 - y0) * (u - x0) / (x1 - x0)
            }
        }),
        K::DotProduct => {
            let u1: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            if let Some(o) = out.first_mut() {
                *o = u0.iter().zip(u1).map(|(a, b)| a * b).sum();
            }
        }
        K::SumElements => {
            if let Some(o) = out.first_mut() {
                *o = u0.iter().sum();
            }
        }
        K::Mux { .. } => {
            let mut k = 0;
            for u in ins {
                for v in *u {
                    if let Some(o) = out.get_mut(k) {
                        *o = *v;
                    }
                    k += 1;
                }
            }
        }
        K::Demux { .. } => {
            for (o, v) in out.iter_mut().zip(u0) {
                *o = *v;
            }
        }
        K::Selector { indices } => {
            for (o, &i) in out.iter_mut().zip(indices) {
                *o = u0.get(i.saturating_sub(1)).copied().unwrap_or(0.0);
            }
        }
        K::Integrator { lower, upper, .. } => {
            for (o, x) in out.iter_mut().zip(xc) {
                let mut v = *x;
                if let Some(l) = lower {
                    v = v.max(*l);
                }
                if let Some(h) = upper {
                    v = v.min(*h);
                }
                *o = v;
            }
        }
        K::Derivative { n } => {
            for (i, o) in out.iter_mut().enumerate() {
                *o = n * (el(u0, i) - xc.get(i).copied().unwrap_or(0.0));
            }
        }
        K::TransferFcn { .. } | K::ZeroPole { .. } | K::StateSpace { .. } => {
            if let Some(l) = &nd.lti {
                lti_apply(l, xc, u0, out);
            }
        }
        K::DiscreteTransferFcn { .. } | K::DiscreteStateSpace { .. } => {
            if let Some(l) = &nd.lti {
                lti_apply(l, xd, u0, out);
            }
        }
        K::Pid { kp, kd, n, .. } => {
            let e = el(u0, 0);
            let integ = xc.first().copied().unwrap_or(0.0);
            let filt = xc.get(1).copied().unwrap_or(0.0);
            if let Some(o) = out.first_mut() {
                *o = kp * e + integ + kd * n * (e - filt);
            }
        }
        K::TransportDelay { delay, initial } => {
            let ok = ctx.hist.is_some_and(|h| h.at(t - delay, out));
            if !ok {
                out.fill(*initial);
            }
        }
        K::UnitDelay { .. } | K::ZeroOrderHold { .. } => {
            for (o, v) in out.iter_mut().zip(xd) {
                *o = *v;
            }
        }
        K::Delay { .. } => {
            for (o, v) in out.iter_mut().zip(xd) {
                *o = *v;
            }
        }
        K::DiscreteIntegrator {
            rule,
            gain,
            sample_time,
            ..
        } => {
            let a = match rule {
                DiscreteRule::ForwardEuler => 0.0,
                DiscreteRule::BackwardEuler => gain * sample_time,
                DiscreteRule::Trapezoidal => gain * sample_time / 2.0,
            };
            for (i, o) in out.iter_mut().enumerate() {
                *o = xd.get(i).copied().unwrap_or(0.0) + a * el(u0, i);
            }
        }
        K::DiscreteDerivative { sample_time } => {
            for (i, o) in out.iter_mut().enumerate() {
                *o = (el(u0, i) - xd.get(i).copied().unwrap_or(0.0)) / sample_time;
            }
        }
    }
}

// ───────────────────── derivatives ─────────────────────

/// The time derivative of the continuous states of `nd`.
pub(crate) fn derivatives(nd: &Node, ins: &[&[f64]], xc: &[f64], dx: &mut [f64]) {
    use BlockKind as K;
    let u0: &[f64] = ins.first().copied().unwrap_or(&[]);
    match &nd.kind {
        K::Integrator { lower, upper, .. } => {
            for (i, d) in dx.iter_mut().enumerate() {
                let u = el(u0, i);
                let x = xc.get(i).copied().unwrap_or(0.0);
                let at_hi = upper.is_some_and(|h| x >= h && u > 0.0);
                let at_lo = lower.is_some_and(|l| x <= l && u < 0.0);
                *d = if at_hi || at_lo { 0.0 } else { u };
            }
        }
        K::Derivative { n } => {
            for (i, d) in dx.iter_mut().enumerate() {
                *d = n * (el(u0, i) - xc.get(i).copied().unwrap_or(0.0));
            }
        }
        K::Pid { ki, n, .. } => {
            let e = el(u0, 0);
            if let Some(d) = dx.get_mut(0) {
                *d = ki * e;
            }
            if let Some(d) = dx.get_mut(1) {
                *d = n * (e - xc.get(1).copied().unwrap_or(0.0));
            }
        }
        K::TransferFcn { .. } | K::ZeroPole { .. } | K::StateSpace { .. } => {
            if let Some(l) = &nd.lti {
                lti_state(l, xc, u0, dx);
            }
        }
        _ => {}
    }
}

// ───────────────────── discrete updates ─────────────────────

/// Latches what must be known before the outputs of a major step: the sample of a
/// zero-order hold (at its sample hit) and the state of a relay. Returns whether anything changed.
pub(crate) fn latch(nd: &Node, hit: bool, ins: &[&[f64]], xd: &mut [f64]) -> bool {
    let u0: &[f64] = ins.first().copied().unwrap_or(&[]);
    match &nd.kind {
        BlockKind::ZeroOrderHold { .. } if hit => {
            let mut changed = false;
            for (i, x) in xd.iter_mut().enumerate() {
                let u = el(u0, i);
                if *x != u {
                    *x = u;
                    changed = true;
                }
            }
            changed
        }
        BlockKind::Relay {
            on_switch,
            off_switch,
            ..
        } => {
            let mut changed = false;
            for (i, x) in xd.iter_mut().enumerate() {
                let u = el(u0, i);
                let next = if u >= *on_switch {
                    1.0
                } else if u <= *off_switch {
                    0.0
                } else {
                    *x
                };
                if next != *x {
                    *x = next;
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

/// The discrete update at a sample hit: the state for the next sample.
pub(crate) fn update(nd: &Node, ins: &[&[f64]], xd: &mut [f64]) {
    use BlockKind as K;
    let u0: &[f64] = ins.first().copied().unwrap_or(&[]);
    match &nd.kind {
        K::UnitDelay { .. } | K::DiscreteDerivative { .. } => {
            for (i, x) in xd.iter_mut().enumerate() {
                *x = el(u0, i);
            }
        }
        K::Delay { .. } => {
            let w = nd.out_w.first().copied().unwrap_or(1).max(1);
            let n = xd.len();
            if n >= w {
                xd.copy_within(w.., 0);
                for i in 0..w {
                    if let Some(x) = xd.get_mut(n - w + i) {
                        *x = el(u0, i);
                    }
                }
            }
        }
        K::DiscreteIntegrator {
            gain, sample_time, ..
        } => {
            // Forward, backward and trapezoidal Euler all advance the state by K·Ts·u.
            let b = gain * sample_time;
            for (i, x) in xd.iter_mut().enumerate() {
                *x += b * el(u0, i);
            }
        }
        K::DiscreteTransferFcn { .. } | K::DiscreteStateSpace { .. } => {
            if let Some(l) = &nd.lti {
                let x = xd.to_vec();
                lti_state(l, &x, u0, xd);
            }
        }
        K::Noise { mean, variance, .. } => {
            let rng = xd.get(1).copied().unwrap_or(0.0);
            let (g, s) = gaussian(rng);
            if let Some(v) = xd.get_mut(0) {
                *v = mean + variance.sqrt() * g;
            }
            if let Some(v) = xd.get_mut(1) {
                *v = s;
            }
        }
        _ => {}
    }
}

// ───────────────────── zero crossings ─────────────────────

/// The functions whose sign changes mark a discontinuity of this block.
pub(crate) fn crossings(nd: &Node, ins: &[&[f64]], xc: &[f64], out: &mut Vec<f64>) {
    use BlockKind as K;
    let u0: &[f64] = ins.first().copied().unwrap_or(&[]);
    let w = nd.out_w.first().copied().unwrap_or(1);
    match &nd.kind {
        K::Saturation { lower, upper } => {
            for i in 0..w {
                out.push(el(u0, i) - lower);
                out.push(el(u0, i) - upper);
            }
        }
        K::DeadZone { start, end } => {
            for i in 0..w {
                out.push(el(u0, i) - start);
                out.push(el(u0, i) - end);
            }
        }
        K::Switch { rule, threshold } => {
            let c: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            for i in 0..w {
                out.push(match rule {
                    SwitchRule::NonZero => el(c, i),
                    _ => el(c, i) - threshold,
                });
            }
        }
        K::Relational { .. } => {
            let u1: &[f64] = ins.get(1).copied().unwrap_or(&[]);
            for i in 0..w {
                out.push(el(u0, i) - el(u1, i));
            }
        }
        K::CompareConst { value, .. } => {
            for i in 0..w {
                out.push(el(u0, i) - value);
            }
        }
        K::Math {
            func: MathFunc::Abs | MathFunc::Sign,
        } => {
            for i in 0..w {
                out.push(el(u0, i));
            }
        }
        K::Relay {
            on_switch,
            off_switch,
            ..
        } => {
            for i in 0..w {
                out.push(el(u0, i) - on_switch);
                out.push(el(u0, i) - off_switch);
            }
        }
        K::Integrator { lower, upper, .. } => {
            for i in 0..w {
                let x = xc.get(i).copied().unwrap_or(0.0);
                if let Some(l) = lower {
                    out.push(x - l);
                }
                if let Some(h) = upper {
                    out.push(x - h);
                }
            }
        }
        _ => {}
    }
}

/// Whether a `Stop` block's input asks for the run to end.
pub(crate) fn stop_requested(nd: &Node, ins: &[&[f64]]) -> bool {
    matches!(nd.kind, BlockKind::Stop) && ins.first().is_some_and(|u| u.iter().any(|&v| v != 0.0))
}
