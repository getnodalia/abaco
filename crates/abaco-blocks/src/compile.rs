// SPDX-License-Identifier: MIT OR Apache-2.0
//! From a [`Model`] to something that can run: subsystems are inlined, `Goto`/`From` pairs are
//! wired, signal widths are inferred, states are laid out and the blocks are sorted so that
//! every block runs after the blocks it feeds through from.

use std::collections::HashMap;

use nodalia_numerics::control::{self, StateSpace};
use nodalia_numerics::Matrix;

use crate::error::{BlockError, Result};
use crate::model::{
    BlockKind, DiscreteRule, LogicOp, Model, SolverConfig, WaveShape, MAX_BLOCKS, MAX_DEPTH,
};

/// Most switching times kept for the solver to land on exactly.
const MAX_BREAKPOINTS: usize = 200_000;
/// Widest signal.
pub(crate) const MAX_WIDTH: usize = 4096;

/// A linear model in flat row-major form.
#[derive(Clone, Debug)]
pub(crate) struct Lti {
    pub n: usize,
    pub m: usize,
    pub p: usize,
    pub a: Vec<f64>,
    pub b: Vec<f64>,
    pub c: Vec<f64>,
    pub d: Vec<f64>,
    pub x0: Vec<f64>,
}

impl Lti {
    fn from_ss(ss: &StateSpace, x0: Vec<f64>) -> Lti {
        Lti {
            n: ss.states(),
            m: ss.inputs(),
            p: ss.outputs(),
            a: ss.a.as_slice().to_vec(),
            b: ss.b.as_slice().to_vec(),
            c: ss.c.as_slice().to_vec(),
            d: ss.d.as_slice().to_vec(),
            x0,
        }
    }
}

/// A block of the flattened model.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub name: String,
    pub kind: BlockKind,
    pub src: Vec<(usize, usize)>,
    pub in_w: Vec<usize>,
    pub out_w: Vec<usize>,
    pub port_off: Vec<usize>,
    pub out_len: usize,
    pub out_base: usize,
    pub xc_off: usize,
    pub n_xc: usize,
    pub xd_off: usize,
    pub n_xd: usize,
    pub lti: Option<Lti>,
    pub ts: Option<f64>,
    pub feedthrough: bool,
}

/// A model ready to simulate.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub(crate) nodes: Vec<Node>,
    pub(crate) order: Vec<usize>,
    pub(crate) cfg: SolverConfig,
    pub(crate) n_xc: usize,
    pub(crate) n_xd: usize,
    pub(crate) sig_len: usize,
    pub(crate) inports: Vec<usize>,
    pub(crate) outports: Vec<usize>,
    pub(crate) breakpoints: Vec<f64>,
}

impl Compiled {
    /// Number of continuous states.
    pub fn continuous_states(&self) -> usize {
        self.n_xc
    }
    /// Number of discrete states.
    pub fn discrete_states(&self) -> usize {
        self.n_xd
    }
    /// Number of blocks after subsystems are inlined.
    pub fn block_count(&self) -> usize {
        self.nodes.len()
    }
    /// The solver settings.
    pub fn solver(&self) -> &SolverConfig {
        &self.cfg
    }
    /// The (flattened) block paths in execution order.
    pub fn execution_order(&self) -> Vec<String> {
        self.order
            .iter()
            .filter_map(|&i| self.nodes.get(i).map(|n| n.name.clone()))
            .collect()
    }
    /// Whether the model has no discrete-time blocks.
    pub fn is_continuous(&self) -> bool {
        self.nodes.iter().all(|n| n.ts.is_none())
    }
}

// ───────────────────────── ports ─────────────────────────

fn parse_signs(name: &str, s: &str, allowed: &str) -> Result<Vec<char>> {
    let v: Vec<char> = s
        .chars()
        .filter(|c| *c != '|' && !c.is_whitespace())
        .collect();
    if v.is_empty() || v.len() > 64 || v.iter().any(|c| !allowed.contains(*c)) {
        return Err(BlockError::block(
            name,
            format!("\"{s}\" must list 1 to 64 of the characters {allowed:?}"),
        ));
    }
    Ok(v)
}

pub(crate) fn sum_signs(name: &str, s: &str) -> Result<Vec<char>> {
    parse_signs(name, s, "+-")
}

pub(crate) fn product_ops(name: &str, s: &str) -> Result<Vec<char>> {
    parse_signs(name, s, "*/")
}

fn count(name: &str, what: &str, n: usize, max: usize) -> Result<usize> {
    if n == 0 || n > max {
        return Err(BlockError::block(
            name,
            format!("{what} must be between 1 and {max}"),
        ));
    }
    Ok(n)
}

/// `(inputs, outputs)` of a block.
pub(crate) fn port_counts(name: &str, kind: &BlockKind) -> Result<(usize, usize)> {
    use BlockKind as K;
    Ok(match kind {
        K::Constant { .. }
        | K::Step { .. }
        | K::Ramp { .. }
        | K::Sine { .. }
        | K::Pulse { .. }
        | K::Wave { .. }
        | K::Chirp { .. }
        | K::Clock
        | K::Ground
        | K::Noise { .. }
        | K::Table { .. }
        | K::Inport { .. }
        | K::From { .. } => (0, 1),
        K::Scope { inputs } => (count(name, "the number of inputs", *inputs, 64)?, 0),
        K::ToWorkspace
        | K::Display
        | K::Terminator
        | K::Stop
        | K::Outport { .. }
        | K::Goto { .. } => (1, 0),
        K::Sum { signs } => (sum_signs(name, signs)?.len(), 1),
        K::Product { ops } => (product_ops(name, ops)?.len(), 1),
        K::Atan2 | K::Mod | K::Relational { .. } | K::DotProduct => (2, 1),
        K::MinMax { inputs, .. } => (count(name, "the number of inputs", *inputs, 64)?, 1),
        K::Logical { op, inputs } => match op {
            LogicOp::Not => (1, 1),
            _ => (count(name, "the number of inputs", *inputs, 64)?, 1),
        },
        K::Switch { .. } => (3, 1),
        K::Mux { inputs } => (count(name, "the number of inputs", *inputs, 256)?, 1),
        K::Demux { outputs } => (1, count(name, "the number of outputs", *outputs, 256)?),
        K::Subsystem { .. } => (0, 0),
        _ => (1, 1),
    })
}

// ───────────────────────── flattening ─────────────────────────

enum Entry {
    Node(usize),
    Sub { ins: Vec<usize>, outs: Vec<usize> },
}

/// A line between two flattened ports (block index, port), with a label for messages.
type FlatLine = ((usize, usize), (usize, usize), String);

#[derive(Default)]
struct Flat {
    nodes: Vec<(String, BlockKind)>,
    lines: Vec<FlatLine>,
    gotos: HashMap<String, usize>,
    froms: Vec<(usize, String)>,
}

fn ordered_ports(name: &str, mut v: Vec<(usize, usize)>, what: &str) -> Result<Vec<usize>> {
    v.sort_unstable();
    for (k, (idx, _)) in v.iter().enumerate() {
        if *idx != k + 1 {
            return Err(BlockError::Model(format!(
                "{what} blocks of \"{name}\" must be numbered 1, 2, 3… without gaps or repeats (found {idx} where {} was expected)",
                k + 1
            )));
        }
    }
    Ok(v.into_iter().map(|(_, n)| n).collect())
}

fn flatten(
    model: &Model,
    prefix: &str,
    depth: usize,
    flat: &mut Flat,
) -> Result<(Vec<usize>, Vec<usize>)> {
    if depth > MAX_DEPTH {
        return Err(BlockError::Model(format!(
            "subsystems are nested more than {MAX_DEPTH} levels deep (is one inside itself?)"
        )));
    }
    let mut entries: HashMap<&str, Entry> = HashMap::new();
    let mut ins: Vec<(usize, usize)> = Vec::new();
    let mut outs: Vec<(usize, usize)> = Vec::new();
    for b in &model.blocks {
        if flat.nodes.len() > MAX_BLOCKS {
            return Err(BlockError::Model(format!(
                "the model has more than {MAX_BLOCKS} blocks"
            )));
        }
        let full = format!("{prefix}{}", b.name);
        if b.name.is_empty() || b.name.contains('/') {
            return Err(BlockError::Model(format!(
                "\"{full}\" is not a valid block name (it is empty or contains \"/\")"
            )));
        }
        let entry = match &b.kind {
            BlockKind::Subsystem { model: inner } => {
                let (i, o) = flatten(inner, &format!("{full}/"), depth + 1, flat)?;
                Entry::Sub { ins: i, outs: o }
            }
            kind => {
                let idx = flat.nodes.len();
                let kind = match kind {
                    BlockKind::Inport { index } if depth > 0 => {
                        ins.push((*index, idx));
                        BlockKind::Identity
                    }
                    BlockKind::Outport { index } if depth > 0 => {
                        outs.push((*index, idx));
                        BlockKind::Identity
                    }
                    BlockKind::Inport { index } => {
                        ins.push((*index, idx));
                        kind.clone()
                    }
                    BlockKind::Outport { index } => {
                        outs.push((*index, idx));
                        kind.clone()
                    }
                    BlockKind::Goto { tag } => {
                        let key = format!("{prefix}{tag}");
                        if flat.gotos.insert(key, idx).is_some() {
                            return Err(BlockError::block(
                                &full,
                                format!("another Goto already uses the tag \"{tag}\""),
                            ));
                        }
                        kind.clone()
                    }
                    BlockKind::From { tag } => {
                        flat.froms.push((idx, format!("{prefix}{tag}")));
                        kind.clone()
                    }
                    other => other.clone(),
                };
                flat.nodes.push((full.clone(), kind));
                Entry::Node(idx)
            }
        };
        if entries.insert(b.name.as_str(), entry).is_some() {
            return Err(BlockError::Model(format!(
                "there are two blocks named \"{full}\""
            )));
        }
    }
    for l in &model.lines {
        let label = format!(
            "{prefix}{}:{} → {prefix}{}:{}",
            l.from.0,
            l.from.1 + 1,
            l.to.0,
            l.to.1 + 1
        );
        let src = endpoint(prefix, &entries, &l.from, false, flat)?;
        let dst = endpoint(prefix, &entries, &l.to, true, flat)?;
        flat.lines.push((src, dst, label));
    }
    let top_ports = |v: Vec<(usize, usize)>, what: &str| ordered_ports(&model.name, v, what);
    Ok((top_ports(ins, "Inport")?, top_ports(outs, "Outport")?))
}

fn endpoint(
    prefix: &str,
    entries: &HashMap<&str, Entry>,
    r: &(String, usize),
    is_dst: bool,
    flat: &Flat,
) -> Result<(usize, usize)> {
    let (name, port) = (&r.0, r.1);
    let side = if is_dst { "input" } else { "output" };
    match entries.get(name.as_str()) {
        None => Err(BlockError::Model(format!(
            "a line refers to the block \"{prefix}{name}\", which does not exist"
        ))),
        Some(Entry::Sub { ins, outs }) => {
            let list = if is_dst { ins } else { outs };
            list.get(port).map(|&n| (n, 0)).ok_or_else(|| {
                BlockError::Model(format!(
                    "subsystem \"{prefix}{name}\" has no {side} {} (it has {})",
                    port + 1,
                    list.len()
                ))
            })
        }
        Some(Entry::Node(idx)) => {
            let (nm, kind) = flat
                .nodes
                .get(*idx)
                .ok_or_else(|| BlockError::Model("internal: node index out of range".into()))?;
            // A nested Inport or Outport became an Identity and has one input and one output.
            let (n_in, n_out) = match kind {
                BlockKind::Identity => (1, 1),
                k => port_counts(nm, k)?,
            };
            let n = if is_dst { n_in } else { n_out };
            if port >= n {
                return Err(BlockError::Model(format!(
                    "block \"{nm}\" has no {side} {} (it has {n})",
                    port + 1
                )));
            }
            Ok((*idx, port))
        }
    }
}

// ───────────────────────── widths ─────────────────────────

fn broadcast(name: &str, ws: &[Option<usize>]) -> Result<Option<usize>> {
    let mut w: Option<usize> = None;
    for &x in ws.iter().flatten() {
        match (w, x) {
            (None, x) => w = Some(x),
            (Some(a), x) if a == x || x == 1 => {}
            (Some(1), x) => w = Some(x),
            (Some(a), x) => {
                return Err(BlockError::block(
                    name,
                    format!("input widths {a} and {x} do not match (only a scalar can be combined with a vector)"),
                ))
            }
        }
    }
    Ok(w)
}

fn rect(name: &str, what: &str, m: &[Vec<f64>]) -> Result<(usize, usize)> {
    let r = m.len();
    let c = m.first().map_or(0, Vec::len);
    if m.iter().any(|row| row.len() != c) {
        return Err(BlockError::block(
            name,
            format!("the rows of {what} have different lengths"),
        ));
    }
    if r > 400 || c > 400 {
        return Err(BlockError::block(
            name,
            format!("{what} is larger than 400 × 400"),
        ));
    }
    Ok((r, c))
}

fn to_matrix(
    name: &str,
    what: &str,
    m: &[Vec<f64>],
    rows: usize,
    cols: usize,
) -> Result<Matrix<f64>> {
    let (r, c) = rect(name, what, m)?;
    if (r, c) != (rows, cols) {
        return Err(BlockError::block(
            name,
            format!("{what} is {r} × {c} but it must be {rows} × {cols}"),
        ));
    }
    let data: Vec<f64> = m.iter().flatten().copied().collect();
    if data.iter().any(|v| !v.is_finite()) {
        return Err(BlockError::block(
            name,
            format!("{what} holds a value that is not finite"),
        ));
    }
    Matrix::new(rows, cols, data).map_err(|e| BlockError::block(name, e.to_string()))
}

fn ss_from_parts(
    name: &str,
    a: &[Vec<f64>],
    b: &[Vec<f64>],
    c: &[Vec<f64>],
    d: &[Vec<f64>],
    x0: &[f64],
) -> Result<Lti> {
    let n = a.len();
    let m = b.first().map_or(0, Vec::len);
    let p = c.len();
    let ma = to_matrix(name, "A", a, n, n)?;
    let mb = to_matrix(name, "B", b, n, m)?;
    let mc = to_matrix(name, "C", c, p, n)?;
    let md = to_matrix(name, "D", d, p, m)?;
    let ss = StateSpace::new(ma, mb, mc, md).map_err(|e| BlockError::block(name, e.to_string()))?;
    let x0 = if x0.is_empty() {
        vec![0.0; n]
    } else if x0.len() == n && x0.iter().all(|v| v.is_finite()) {
        x0.to_vec()
    } else {
        return Err(BlockError::block(
            name,
            format!(
                "the initial state has {} entries but the model has {n} states",
                x0.len()
            ),
        ));
    };
    if m == 0 || p == 0 {
        return Err(BlockError::block(
            name,
            "the model needs at least one input and one output",
        ));
    }
    Ok(Lti::from_ss(&ss, x0))
}

fn tf_lti(name: &str, num: &[f64], den: &[f64]) -> Result<Lti> {
    let ss = control::tf2ss(num, den).map_err(|e| BlockError::block(name, e.to_string()))?;
    let n = ss.states();
    Ok(Lti::from_ss(&ss, vec![0.0; n]))
}

fn poly_from_roots(name: &str, roots: &[[f64; 2]], gain: f64) -> Result<Vec<f64>> {
    let mut p: Vec<(f64, f64)> = vec![(gain, 0.0)];
    for r in roots {
        if !(r[0].is_finite() && r[1].is_finite()) {
            return Err(BlockError::block(name, "a zero or pole is not finite"));
        }
        let mut next = vec![(0.0, 0.0); p.len() + 1];
        for (i, &(re, im)) in p.iter().enumerate() {
            if let Some(x) = next.get_mut(i) {
                x.0 += re;
                x.1 += im;
            }
            if let Some(x) = next.get_mut(i + 1) {
                // multiply by −root
                x.0 -= re * r[0] - im * r[1];
                x.1 -= re * r[1] + im * r[0];
            }
        }
        p = next;
    }
    let scale = p
        .iter()
        .fold(0.0f64, |m, c| m.max(c.0.abs()).max(c.1.abs()))
        .max(1e-300);
    if p.iter().any(|c| c.1.abs() > 1e-9 * scale) {
        return Err(BlockError::block(
            name,
            "complex zeros and poles must come in conjugate pairs",
        ));
    }
    Ok(p.into_iter().map(|c| c.0).collect())
}

fn zp_lti(name: &str, zeros: &[[f64; 2]], poles: &[[f64; 2]], gain: f64) -> Result<Lti> {
    if poles.len() > 200 || zeros.len() > poles.len() {
        return Err(BlockError::block(
            name,
            "there must be at most 200 poles and no more zeros than poles",
        ));
    }
    let den = poly_from_roots(name, poles, 1.0)?;
    let num = poly_from_roots(name, zeros, gain)?;
    tf_lti(name, &num, &den)
}

/// Output widths of a block when enough input widths are known.
fn out_widths(name: &str, kind: &BlockKind, in_w: &[Option<usize>]) -> Result<Vec<Option<usize>>> {
    use BlockKind as K;
    let w0 = in_w.first().copied().flatten();
    let same = || Ok(vec![w0]);
    let scalar = || Ok(vec![Some(1)]);
    let init_or = |initial: &[f64]| -> Result<Vec<Option<usize>>> {
        if initial.is_empty() {
            return Err(BlockError::block(name, "the initial value is empty"));
        }
        Ok(vec![if initial.len() > 1 {
            Some(initial.len())
        } else {
            w0
        }])
    };
    match kind {
        K::Constant { value } => {
            if value.is_empty() || value.len() > MAX_WIDTH {
                return Err(BlockError::block(
                    name,
                    "the value must have between 1 and 4096 entries",
                ));
            }
            Ok(vec![Some(value.len())])
        }
        K::Step { .. }
        | K::Ramp { .. }
        | K::Sine { .. }
        | K::Pulse { .. }
        | K::Wave { .. }
        | K::Chirp { .. }
        | K::Clock
        | K::Ground
        | K::Noise { .. }
        | K::Table { .. }
        | K::Inport { .. } => scalar(),
        K::Scope { .. }
        | K::ToWorkspace
        | K::Display
        | K::Terminator
        | K::Stop
        | K::Outport { .. }
        | K::Goto { .. } => Ok(vec![]),
        K::Identity => same(),
        K::From { .. } => Ok(vec![None]),
        K::Sum { .. }
        | K::Product { .. }
        | K::MinMax { .. }
        | K::Atan2
        | K::Mod
        | K::Relational { .. } => Ok(vec![broadcast(name, in_w)?]),
        K::Logical { op, .. } => match op {
            LogicOp::Not => same(),
            _ => Ok(vec![broadcast(name, in_w)?]),
        },
        K::Switch { .. } => Ok(vec![broadcast(
            name,
            &[
                in_w.first().copied().flatten(),
                in_w.get(2).copied().flatten(),
            ],
        )?]),
        K::Gain { .. }
        | K::Math { .. }
        | K::Power { .. }
        | K::Saturation { .. }
        | K::DeadZone { .. }
        | K::Bias { .. }
        | K::Rounding { .. }
        | K::CompareConst { .. }
        | K::Relay { .. }
        | K::Quantizer { .. }
        | K::Lookup1D { .. }
        | K::Derivative { .. }
        | K::TransportDelay { .. }
        | K::Delay { .. }
        | K::ZeroOrderHold { .. }
        | K::DiscreteIntegrator { .. }
        | K::DiscreteDerivative { .. } => same(),
        K::MatrixGain { matrix } => Ok(vec![Some(rect(name, "the matrix", matrix)?.0)]),
        K::DotProduct | K::SumElements => scalar(),
        K::Mux { .. } => {
            if in_w.iter().all(Option::is_some) {
                Ok(vec![Some(in_w.iter().flatten().sum())])
            } else {
                Ok(vec![None])
            }
        }
        K::Demux { outputs } => match w0 {
            Some(w) if w % outputs != 0 => Err(BlockError::block(
                name,
                format!("a signal of width {w} cannot be split into {outputs} equal parts"),
            )),
            Some(w) => Ok(vec![Some(w / outputs); *outputs]),
            None => Ok(vec![None; *outputs]),
        },
        K::Selector { indices } => {
            if indices.is_empty() {
                return Err(BlockError::block(name, "the list of elements is empty"));
            }
            Ok(vec![Some(indices.len())])
        }
        K::Integrator { initial, .. } | K::UnitDelay { initial, .. } => init_or(initial),
        K::TransferFcn { .. }
        | K::ZeroPole { .. }
        | K::Pid { .. }
        | K::DiscreteTransferFcn { .. } => scalar(),
        K::StateSpace { c, .. } | K::DiscreteStateSpace { c, .. } => {
            Ok(vec![Some(rect(name, "C", c)?.0)])
        }
        K::Subsystem { .. } => Ok(vec![]),
    }
}

/// Checks the input widths once all are known.
fn check_inputs(name: &str, kind: &BlockKind, in_w: &[usize]) -> Result<()> {
    use BlockKind as K;
    let need_scalar = |what: &str| -> Result<()> {
        if in_w.iter().any(|&w| w != 1) {
            Err(BlockError::block(
                name,
                format!("{what} takes scalar signals"),
            ))
        } else {
            Ok(())
        }
    };
    let w0 = in_w.first().copied().unwrap_or(1);
    match kind {
        K::TransferFcn { .. } | K::ZeroPole { .. } | K::DiscreteTransferFcn { .. } => {
            need_scalar("a transfer function")
        }
        K::Pid { .. } => need_scalar("a PID controller"),
        K::Atan2
        | K::Mod
        | K::Relational { .. }
        | K::Sum { .. }
        | K::Product { .. }
        | K::MinMax { .. } => {
            let ws: Vec<Option<usize>> = in_w.iter().map(|&w| Some(w)).collect();
            broadcast(name, &ws).map(|_| ())
        }
        K::Logical { .. } => {
            let ws: Vec<Option<usize>> = in_w.iter().map(|&w| Some(w)).collect();
            broadcast(name, &ws).map(|_| ())
        }
        K::Switch { .. } => {
            let (a, c) = (w0, in_w.get(2).copied().unwrap_or(1));
            let control = in_w.get(1).copied().unwrap_or(1);
            if broadcast(name, &[Some(a), Some(c)])?.is_none()
                || (control != 1 && control != a.max(c))
            {
                return Err(BlockError::block(
                    name,
                    "the control input must be a scalar or as wide as the data inputs",
                ));
            }
            Ok(())
        }
        K::MatrixGain { matrix } => {
            let cols = rect(name, "the matrix", matrix)?.1;
            if cols != w0 {
                return Err(BlockError::block(
                    name,
                    format!("the matrix has {cols} columns but the input is {w0} wide"),
                ));
            }
            Ok(())
        }
        K::DotProduct => {
            if in_w.first() != in_w.get(1) {
                return Err(BlockError::block(
                    name,
                    "the two vectors must have the same width",
                ));
            }
            Ok(())
        }
        K::Selector { indices } => {
            if let Some(bad) = indices.iter().find(|&&i| i == 0 || i > w0) {
                return Err(BlockError::block(
                    name,
                    format!("element {bad} is outside the input (width {w0}); elements are numbered from 1"),
                ));
            }
            Ok(())
        }
        K::StateSpace { b, .. } | K::DiscreteStateSpace { b, .. } => {
            let m = b.first().map_or(0, Vec::len);
            if m != w0 {
                return Err(BlockError::block(
                    name,
                    format!("the model has {m} inputs but the signal is {w0} wide"),
                ));
            }
            Ok(())
        }
        K::Lookup1D { x, y } => {
            if x.len() != y.len()
                || x.len() < 2
                || x.windows(2).any(|p| p[0] >= p[1])
                || x.iter().chain(y).any(|v| !v.is_finite())
            {
                return Err(BlockError::block(
                    name,
                    "x and y need the same number (at least 2) of finite values, with x strictly increasing",
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// `(continuous states, discrete states)` given the signal width.
fn state_counts(
    name: &str,
    kind: &BlockKind,
    w: usize,
    lti: Option<&Lti>,
) -> Result<(usize, usize)> {
    use BlockKind as K;
    Ok(match kind {
        K::Integrator { .. } | K::Derivative { .. } => (w, 0),
        K::Pid { .. } => (2, 0),
        K::TransferFcn { .. } | K::ZeroPole { .. } | K::StateSpace { .. } => {
            (lti.map_or(0, |l| l.n), 0)
        }
        K::DiscreteTransferFcn { .. } | K::DiscreteStateSpace { .. } => (0, lti.map_or(0, |l| l.n)),
        K::UnitDelay { .. }
        | K::ZeroOrderHold { .. }
        | K::DiscreteIntegrator { .. }
        | K::DiscreteDerivative { .. }
        | K::Relay { .. } => (0, w),
        K::Delay { length, .. } => {
            if *length == 0 || length.saturating_mul(w) > 1_000_000 {
                return Err(BlockError::block(
                    name,
                    "the length must be at least 1 and the delay line at most a million values",
                ));
            }
            (0, length * w)
        }
        K::Noise { .. } => (0, 2),
        _ => (0, 0),
    })
}

fn feedthrough(kind: &BlockKind, lti: Option<&Lti>) -> bool {
    use BlockKind as K;
    match kind {
        K::Integrator { .. }
        | K::TransportDelay { .. }
        | K::UnitDelay { .. }
        | K::Delay { .. }
        | K::ZeroOrderHold { .. }
        | K::Relay { .. }
        | K::Noise { .. } => false,
        K::DiscreteIntegrator { rule, .. } => *rule != DiscreteRule::ForwardEuler,
        K::TransferFcn { .. }
        | K::ZeroPole { .. }
        | K::StateSpace { .. }
        | K::DiscreteTransferFcn { .. }
        | K::DiscreteStateSpace { .. } => lti.is_some_and(|l| l.d.iter().any(|&v| v != 0.0)),
        K::Pid { kp, kd, .. } => *kp != 0.0 || *kd != 0.0,
        _ => true,
    }
}

fn sample_time(name: &str, kind: &BlockKind) -> Result<Option<f64>> {
    use BlockKind as K;
    let ts = match kind {
        K::UnitDelay { sample_time, .. }
        | K::Delay { sample_time, .. }
        | K::ZeroOrderHold { sample_time }
        | K::DiscreteIntegrator { sample_time, .. }
        | K::DiscreteTransferFcn { sample_time, .. }
        | K::DiscreteStateSpace { sample_time, .. }
        | K::DiscreteDerivative { sample_time }
        | K::Noise { sample_time, .. } => *sample_time,
        _ => return Ok(None),
    };
    if !(ts.is_finite() && ts > 0.0) {
        return Err(BlockError::block(
            name,
            "the sample time must be a positive number",
        ));
    }
    Ok(Some(ts))
}

fn lti_for(name: &str, kind: &BlockKind) -> Result<Option<Lti>> {
    use BlockKind as K;
    Ok(match kind {
        K::TransferFcn { num, den } | K::DiscreteTransferFcn { num, den, .. } => {
            Some(tf_lti(name, num, den)?)
        }
        K::ZeroPole { zeros, poles, gain } => Some(zp_lti(name, zeros, poles, *gain)?),
        K::StateSpace { a, b, c, d, x0 } | K::DiscreteStateSpace { a, b, c, d, x0, .. } => {
            Some(ss_from_parts(name, a, b, c, d, x0)?)
        }
        _ => None,
    })
}

// ───────────────────────── switching times ─────────────────────────

fn source_breakpoints(
    name: &str,
    kind: &BlockKind,
    t0: f64,
    tf: f64,
    out: &mut Vec<f64>,
) -> Result<()> {
    use BlockKind as K;
    let push = |t: f64, out: &mut Vec<f64>| -> Result<()> {
        if t.is_finite() && t > t0 && t < tf {
            if out.len() >= MAX_BREAKPOINTS {
                return Err(BlockError::block(
                    name,
                    format!("switches more than {MAX_BREAKPOINTS} times in the run; lengthen the period or shorten the stop time"),
                ));
            }
            out.push(t);
        }
        Ok(())
    };
    match kind {
        K::Step { time, .. } => push(*time, out)?,
        K::Ramp { start, .. } => push(*start, out)?,
        K::Table { times, .. } => {
            for &t in times {
                push(t, out)?;
            }
        }
        K::Pulse {
            period,
            duty,
            delay,
            ..
        } => {
            if !(period.is_finite() && *period > 0.0 && (0.0..=100.0).contains(duty)) {
                return Err(BlockError::block(
                    name,
                    "the period must be positive and the width between 0 and 100 %",
                ));
            }
            push(*delay, out)?;
            let mut k = 0.0f64;
            loop {
                let start = delay + k * period;
                if start >= tf {
                    break;
                }
                push(start, out)?;
                push(start + duty / 100.0 * period, out)?;
                k += 1.0;
            }
        }
        K::Wave {
            shape, frequency, ..
        } => {
            if !(frequency.is_finite() && *frequency > 0.0) {
                return Err(BlockError::block(name, "the frequency must be positive"));
            }
            let period = 1.0 / frequency;
            let step = match shape {
                WaveShape::Square => period / 2.0,
                WaveShape::Sawtooth => period,
                _ => return Ok(()),
            };
            let mut k = 1.0f64;
            while k * step < tf {
                push(k * step, out)?;
                k += 1.0;
            }
        }
        _ => {}
    }
    Ok(())
}

// ───────────────────────── compile ─────────────────────────

/// Builds a runnable model; every problem is reported with the block that causes it.
pub fn compile(model: &Model) -> Result<Compiled> {
    let cfg = model.solver.clone();
    validate_solver(&cfg)?;
    let mut flat = Flat::default();
    let (top_in, top_out) = flatten(model, "", 0, &mut flat)?;
    let n = flat.nodes.len();
    if n == 0 {
        return Err(BlockError::Model("the model has no blocks".into()));
    }

    // Nodes with their port counts.
    let mut nodes: Vec<Node> = Vec::with_capacity(n);
    for (name, kind) in &flat.nodes {
        let (n_in, n_out) = match kind {
            BlockKind::Identity => (1, 1),
            k => port_counts(name, k)?,
        };
        nodes.push(Node {
            name: name.clone(),
            kind: kind.clone(),
            src: Vec::new(),
            in_w: vec![0; n_in],
            out_w: vec![0; n_out],
            port_off: Vec::new(),
            out_len: 0,
            out_base: 0,
            xc_off: 0,
            n_xc: 0,
            xd_off: 0,
            n_xd: 0,
            lti: None,
            ts: None,
            feedthrough: true,
        });
    }
    // Wires.
    let mut src: Vec<Vec<Option<(usize, usize)>>> =
        nodes.iter().map(|nd| vec![None; nd.in_w.len()]).collect();
    for (from, to, label) in &flat.lines {
        let slot = src
            .get_mut(to.0)
            .and_then(|v| v.get_mut(to.1))
            .ok_or_else(|| BlockError::Model("internal: line to a missing port".into()))?;
        if slot.is_some() {
            return Err(BlockError::Model(format!(
                "input {} of \"{}\" is driven by two lines (second: {label}); use a Sum block to combine signals",
                to.1 + 1,
                nodes.get(to.0).map_or("?", |x| x.name.as_str())
            )));
        }
        *slot = Some(*from);
    }
    // From blocks become pass-throughs of their Goto's source.
    for (idx, key) in &flat.froms {
        let g = flat.gotos.get(key).ok_or_else(|| {
            BlockError::block(
                nodes.get(*idx).map_or("?", |x| x.name.as_str()),
                format!(
                    "no Goto block with the tag \"{}\"",
                    key.rsplit('/').next().unwrap_or(key)
                ),
            )
        })?;
        let s = src.get(*g).and_then(|v| v.first()).copied().flatten();
        if let Some(nd) = nodes.get_mut(*idx) {
            nd.kind = BlockKind::Identity;
            nd.in_w = vec![0];
        }
        if let Some(slot) = src.get_mut(*idx) {
            *slot = vec![s];
        }
    }
    // Every input must be wired.
    let mut wired: Vec<Vec<(usize, usize)>> = Vec::with_capacity(n);
    for (i, s) in src.iter().enumerate() {
        let mut v = Vec::with_capacity(s.len());
        for (p, e) in s.iter().enumerate() {
            match e {
                Some(x) => v.push(*x),
                None => {
                    return Err(BlockError::block(
                        nodes.get(i).map_or("?", |x| x.name.as_str()),
                        format!(
                            "input {} is not connected (use a Ground block to tie it to zero)",
                            p + 1
                        ),
                    ))
                }
            }
        }
        wired.push(v);
    }
    for (nd, w) in nodes.iter_mut().zip(wired) {
        nd.src = w;
    }

    infer_widths(&mut nodes)?;

    // Layout.
    let mut sig_len = 0;
    let (mut n_xc, mut n_xd) = (0, 0);
    let mut breakpoints: Vec<f64> = Vec::new();
    for nd in nodes.iter_mut() {
        let mut off = 0;
        nd.port_off = nd
            .out_w
            .iter()
            .map(|&w| {
                let o = off;
                off += w;
                o
            })
            .collect();
        nd.out_len = off;
        nd.out_base = sig_len;
        sig_len += off;
        nd.lti = lti_for(&nd.name, &nd.kind)?;
        let w = nd.out_w.first().copied().unwrap_or(1);
        let (cx, dx) = state_counts(&nd.name, &nd.kind, w, nd.lti.as_ref())?;
        nd.n_xc = cx;
        nd.n_xd = dx;
        nd.xc_off = n_xc;
        nd.xd_off = n_xd;
        n_xc += cx;
        n_xd += dx;
        nd.ts = sample_time(&nd.name, &nd.kind)?;
        nd.feedthrough = feedthrough(&nd.kind, nd.lti.as_ref());
        validate_params(nd)?;
        source_breakpoints(
            &nd.name,
            &nd.kind,
            cfg.start_time,
            cfg.stop_time,
            &mut breakpoints,
        )?;
    }
    if n_xc > 20_000 {
        return Err(BlockError::Model(format!(
            "{n_xc} continuous states is more than the solver accepts (20000)"
        )));
    }
    let base = breakpoints.clone();
    for nd in &nodes {
        if let BlockKind::TransportDelay { delay, .. } = nd.kind {
            for b in &base {
                let t = b + delay;
                if t < cfg.stop_time && breakpoints.len() < MAX_BREAKPOINTS {
                    breakpoints.push(t);
                }
            }
        }
    }
    breakpoints.sort_by(f64::total_cmp);
    breakpoints.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * a.abs().max(1.0));

    let order = sort_blocks(&nodes)?;
    Ok(Compiled {
        nodes,
        order,
        cfg,
        n_xc,
        n_xd,
        sig_len,
        inports: top_in,
        outports: top_out,
        breakpoints,
    })
}

fn validate_solver(c: &SolverConfig) -> Result<()> {
    let bad = |m: &str| Err(BlockError::Model(format!("solver settings: {m}")));
    if !(c.start_time.is_finite() && c.stop_time.is_finite()) || c.stop_time <= c.start_time {
        return bad("the stop time must be greater than the start time");
    }
    if !(c.rtol.is_finite() && c.rtol > 0.0 && c.atol.is_finite() && c.atol > 0.0) {
        return bad("the tolerances must be positive");
    }
    for (what, v) in [("step", c.step), ("max_step", c.max_step)] {
        if let Some(h) = v {
            if !(h.is_finite() && h > 0.0) {
                return bad(&format!("{what} must be positive"));
            }
        }
    }
    Ok(())
}

fn validate_params(nd: &Node) -> Result<()> {
    use BlockKind as K;
    let name = nd.name.as_str();
    let fin = |vals: &[f64]| -> Result<()> {
        if vals.iter().any(|v| !v.is_finite()) {
            Err(BlockError::block(
                name,
                "a parameter is not a finite number",
            ))
        } else {
            Ok(())
        }
    };
    match &nd.kind {
        K::Constant { value } => fin(value),
        K::Step {
            time,
            before,
            after,
        } => fin(&[*time, *before, *after]),
        K::Ramp {
            slope,
            start,
            initial,
        } => fin(&[*slope, *start, *initial]),
        K::Sine {
            amplitude,
            frequency,
            phase,
            bias,
        } => fin(&[*amplitude, *frequency, *phase, *bias]),
        K::Gain { gain } => fin(&[*gain]),
        K::Bias { bias } => fin(&[*bias]),
        K::Saturation { lower, upper } => {
            fin(&[*lower, *upper])?;
            if lower > upper {
                return Err(BlockError::block(
                    name,
                    "the lower limit is above the upper limit",
                ));
            }
            Ok(())
        }
        K::DeadZone { start, end } => {
            fin(&[*start, *end])?;
            if start > end {
                return Err(BlockError::block(
                    name,
                    "the start of the dead zone is after its end",
                ));
            }
            Ok(())
        }
        K::Relay {
            on_switch,
            off_switch,
            on_output,
            off_output,
        } => {
            fin(&[*on_switch, *off_switch, *on_output, *off_output])?;
            if on_switch < off_switch {
                return Err(BlockError::block(
                    name,
                    "the switch-on point is below the switch-off point",
                ));
            }
            Ok(())
        }
        K::Quantizer { interval } => {
            if !(interval.is_finite() && *interval > 0.0) {
                return Err(BlockError::block(name, "the interval must be positive"));
            }
            Ok(())
        }
        K::Integrator {
            initial,
            lower,
            upper,
        } => {
            fin(initial)?;
            if let (Some(l), Some(u)) = (lower, upper) {
                if l > u {
                    return Err(BlockError::block(
                        name,
                        "the lower limit is above the upper limit",
                    ));
                }
            }
            Ok(())
        }
        K::Derivative { n } | K::Pid { n, .. } => {
            if !(n.is_finite() && *n > 0.0) {
                return Err(BlockError::block(
                    name,
                    "the filter coefficient must be positive",
                ));
            }
            if let K::Pid {
                kp,
                ki,
                kd,
                initial,
                ..
            } = &nd.kind
            {
                fin(&[*kp, *ki, *kd, *initial])?;
            }
            Ok(())
        }
        K::TransportDelay { delay, initial } => {
            fin(&[*initial])?;
            if !(delay.is_finite() && *delay > 0.0) {
                return Err(BlockError::block(name, "the delay must be positive"));
            }
            Ok(())
        }
        K::UnitDelay { initial, .. } => fin(initial),
        K::Delay { initial, .. } => fin(&[*initial]),
        K::DiscreteIntegrator { gain, initial, .. } => fin(&[*gain, *initial]),
        K::Table { times, values, .. } => {
            if times.len() != values.len()
                || times.is_empty()
                || times.windows(2).any(|p| p[0] >= p[1])
            {
                return Err(BlockError::block(name, "times and values need the same number of entries, at least one, with times strictly increasing"));
            }
            fin(times)?;
            fin(values)
        }
        K::Chirp {
            amplitude,
            f0,
            target_time,
            f1,
        } => {
            fin(&[*amplitude, *f0, *f1])?;
            if !(target_time.is_finite() && *target_time > 0.0) {
                return Err(BlockError::block(name, "the target time must be positive"));
            }
            Ok(())
        }
        K::Noise { mean, variance, .. } => {
            fin(&[*mean])?;
            if !(variance.is_finite() && *variance >= 0.0) {
                return Err(BlockError::block(name, "the variance must not be negative"));
            }
            Ok(())
        }
        K::Switch { threshold, .. } => fin(&[*threshold]),
        K::CompareConst { value, .. } => fin(&[*value]),
        K::Power { exponent } => fin(&[*exponent]),
        K::Wave { amplitude, .. } | K::Pulse { amplitude, .. } => fin(&[*amplitude]),
        K::MatrixGain { matrix } => {
            let flat: Vec<f64> = matrix.iter().flatten().copied().collect();
            if flat.is_empty() {
                return Err(BlockError::block(name, "the matrix is empty"));
            }
            fin(&flat)
        }
        _ => Ok(()),
    }
}

fn infer_widths(nodes: &mut [Node]) -> Result<()> {
    let n = nodes.len();
    let mut known: Vec<Vec<Option<usize>>> =
        nodes.iter().map(|nd| vec![None; nd.out_w.len()]).collect();
    let mut guard = 0;
    loop {
        let mut changed = false;
        for i in 0..n {
            let in_w: Vec<Option<usize>> = nodes[i]
                .src
                .iter()
                .map(|&(s, p)| known.get(s).and_then(|v| v.get(p)).copied().flatten())
                .collect();
            let outs = out_widths(&nodes[i].name, &nodes[i].kind, &in_w)?;
            // An Identity made from a From block has its width from the pass-through.
            for (p, w) in outs.into_iter().enumerate() {
                if let (Some(w), Some(slot)) = (w, known.get_mut(i).and_then(|v| v.get_mut(p))) {
                    if slot.is_none() {
                        if w == 0 || w > MAX_WIDTH {
                            return Err(BlockError::block(
                                &nodes[i].name,
                                format!(
                                    "a signal of width {w} is not supported (limit {MAX_WIDTH})"
                                ),
                            ));
                        }
                        *slot = Some(w);
                        changed = true;
                    } else if *slot != Some(w) {
                        return Err(BlockError::block(
                            &nodes[i].name,
                            format!("the output width changed from {} to {w}; the diagram's signal widths are inconsistent", slot.unwrap_or(0)),
                        ));
                    }
                }
            }
        }
        if changed {
            guard += 1;
            if guard > n + 8 {
                return Err(BlockError::Model("signal widths did not settle".into()));
            }
            continue;
        }
        // A loop of state blocks with scalar defaults and nothing to fix their width: scalar.
        // Any other unanchored loop (an algebraic one, reported later) defaults to scalar too.
        let unresolved = |i: usize| known.get(i).is_some_and(|v| v.iter().any(Option::is_none));
        let stuck = (0..n)
            .find(|&i| {
                unresolved(i)
                    && matches!(
                        nodes[i].kind,
                        BlockKind::Integrator { .. } | BlockKind::UnitDelay { .. }
                    )
            })
            .or_else(|| (0..n).find(|&i| unresolved(i)));
        match stuck {
            Some(i) => {
                if let Some(slot) = known
                    .get_mut(i)
                    .and_then(|v| v.iter_mut().find(|s| s.is_none()))
                {
                    *slot = Some(1);
                }
            }
            None => break,
        }
    }
    for (i, nd) in nodes.iter_mut().enumerate() {
        let mut outs = Vec::with_capacity(nd.out_w.len());
        for w in known.get(i).map(Vec::as_slice).unwrap_or(&[]) {
            outs.push(w.ok_or_else(|| {
                BlockError::block(&nd.name, "cannot tell how wide the signal is; give a block in the loop a vector initial value or constant")
            })?);
        }
        nd.out_w = outs;
    }
    let widths: Vec<Vec<usize>> = nodes.iter().map(|nd| nd.out_w.clone()).collect();
    for nd in nodes.iter_mut() {
        nd.in_w = nd
            .src
            .iter()
            .map(|&(s, p)| widths.get(s).and_then(|v| v.get(p)).copied().unwrap_or(1))
            .collect();
        check_inputs(&nd.name, &nd.kind, &nd.in_w)?;
    }
    // Delay lines with a vector initial value must match the input width.
    for nd in nodes.iter() {
        if let BlockKind::UnitDelay { initial, .. } = &nd.kind {
            let w = nd.out_w.first().copied().unwrap_or(1);
            if initial.len() != 1 && initial.len() != w {
                return Err(BlockError::block(
                    &nd.name,
                    format!(
                        "the initial value has {} entries but the signal is {w} wide",
                        initial.len()
                    ),
                ));
            }
        }
        if let BlockKind::Integrator { initial, .. } = &nd.kind {
            let w = nd.out_w.first().copied().unwrap_or(1);
            let iw = nd.in_w.first().copied().unwrap_or(1);
            if (initial.len() != 1 && initial.len() != w) || iw != w {
                return Err(BlockError::block(&nd.name, format!("the input is {iw} wide and the initial value has {} entries; they must agree", initial.len())));
            }
        }
    }
    Ok(())
}

fn sort_blocks(nodes: &[Node]) -> Result<Vec<usize>> {
    let n = nodes.len();
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, nd) in nodes.iter().enumerate() {
        if !nd.feedthrough {
            continue;
        }
        for &(s, _) in &nd.src {
            if let Some(v) = succ.get_mut(s) {
                v.push(i);
            }
            if let Some(d) = indeg.get_mut(i) {
                *d += 1;
            }
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| indeg.get(i) == Some(&0)).collect();
    ready.reverse();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop() {
        order.push(i);
        for &j in succ.get(i).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(d) = indeg.get_mut(j) {
                *d -= 1;
                if *d == 0 {
                    ready.push(j);
                }
            }
        }
    }
    if order.len() == n {
        return Ok(order);
    }
    let left: Vec<bool> = {
        let mut v = vec![true; n];
        for &i in &order {
            if let Some(x) = v.get_mut(i) {
                *x = false;
            }
        }
        v
    };
    // Walk back through unsorted blocks until a block repeats: that is the loop.
    let start = (0..n).find(|&i| left.get(i) == Some(&true)).unwrap_or(0);
    let mut path: Vec<usize> = Vec::new();
    let mut cur = start;
    loop {
        if let Some(pos) = path.iter().position(|&p| p == cur) {
            let names: Vec<&str> = path
                .get(pos..)
                .unwrap_or(&[])
                .iter()
                .filter_map(|&i| nodes.get(i).map(|x| x.name.as_str()))
                .collect();
            return Err(BlockError::AlgebraicLoop(names.join(" → ")));
        }
        path.push(cur);
        let next = nodes.get(cur).and_then(|nd| {
            nd.src
                .iter()
                .map(|&(s, _)| s)
                .find(|&s| left.get(s) == Some(&true))
        });
        match next {
            Some(s) if nodes.get(cur).is_some_and(|nd| nd.feedthrough) => cur = s,
            _ => {
                return Err(BlockError::AlgebraicLoop(
                    nodes
                        .get(start)
                        .map_or_else(String::new, |x| x.name.clone()),
                ))
            }
        }
    }
}
