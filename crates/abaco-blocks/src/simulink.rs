// SPDX-License-Identifier: MIT OR Apache-2.0
//! Simulink models (`.mdl` and `.slx`) to Ábaco Blocks models and back.
//!
//! [`from_simulink`] turns the structure that [`abaco_formats::simulink`] reads into a [`Model`]:
//! block types become [`BlockKind`]s, parameter texts are evaluated (numbers, vectors, matrices
//! and the variables of a workspace), solver settings become a [`SolverConfig`], subsystems
//! become subsystems. [`to_simulink`] goes the other way and writes every parameter out, so the
//! model does not depend on Simulink's defaults.
//!
//! Nothing is guessed silently. A block that has no equivalent, a parameter that cannot be
//! evaluated or a setting that is only approximated is reported: in *strict* mode every one of
//! them is an error that lists them all, in lenient mode the block is dropped (with its lines)
//! and a warning says so.
//!
//! What this does **not** do: it does not run MATLAB code (a parameter is a number expression,
//! see [`crate::expr`]), it does not read masks or library links, and it has not been checked
//! against files opened by MATLAB itself: round trips and hand-written files of the documented
//! structure are what the tests cover.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use abaco_formats::mat::MatValue;
use abaco_formats::simulink::{SimBlock, SimLine, SimModel, SimSystem};

use crate::error::{BlockError, Result};
use crate::expr::{self, Mat, Workspace};
use crate::model::{
    Block, BlockKind, DiscreteRule, Extremum, Line, LogicOp, MathFunc, Method, Model, RelOp,
    RoundMode, SolverConfig, SwitchRule, WaveShape,
};

/// The most elements a parameter may expand to.
const MAX_ELEMENTS: usize = 1 << 20;

/// A converted model and what had to be approximated or dropped.
#[derive(Clone, Debug)]
pub struct Import {
    /// The model.
    pub model: Model,
    /// One line per approximation or dropped block.
    pub warnings: Vec<String>,
}

/// Reads an `.mdl` or `.slx` file (told apart by its first bytes) and converts it.
pub fn read(bytes: &[u8], workspace: &Workspace, strict: bool) -> Result<Import> {
    let sim = abaco_formats::simulink::read(bytes).map_err(|e| BlockError::Model(e.to_string()))?;
    from_simulink(&sim, workspace, strict)
}

/// The model as `.mdl` text.
pub fn write_mdl(model: &Model) -> Result<String> {
    Ok(abaco_formats::mdl::write(&to_simulink(model)?))
}

/// The model as an `.slx` archive.
pub fn write_slx(model: &Model) -> Result<Vec<u8>> {
    abaco_formats::slx::write(&to_simulink(model)?).map_err(|e| BlockError::Model(e.to_string()))
}

/// The real numeric and logical two-dimensional variables of a MAT-file as a workspace, ready to
/// supply the names that block parameters use. Complex, sparse, text, cell and struct variables
/// are left out: a parameter expression cannot use them.
pub fn workspace_from_mat(vars: &[(String, MatValue)]) -> Workspace {
    let mut ws = Workspace::new();
    for (name, v) in vars {
        let (dims, data): (&[usize], Vec<f64>) = match v {
            MatValue::Numeric {
                dims, re, im: None, ..
            } => (dims, re.clone()),
            MatValue::Logical { dims, data } => {
                (dims, data.iter().map(|b| f64::from(u8::from(*b))).collect())
            }
            _ => continue,
        };
        let (r, c) = match dims {
            [r, c] => (*r, *c),
            _ => continue,
        };
        if r.checked_mul(c) != Some(data.len()) || data.len() > MAX_ELEMENTS {
            continue;
        }
        let rows = (0..r)
            .map(|i| {
                (0..c)
                    .filter_map(|j| data.get(j * r + i).copied())
                    .collect()
            })
            .collect();
        ws.insert(name.clone(), rows);
    }
    ws
}

// ───────────────────────────── import ─────────────────────────────

struct Cx<'a> {
    ws: &'a Workspace,
    strict: bool,
    fixed_step: Option<f64>,
    warnings: Vec<String>,
    problems: Vec<String>,
}

/// Converts a Simulink model. `workspace` supplies the variables that block parameters name.
pub fn from_simulink(sim: &SimModel, workspace: &Workspace, strict: bool) -> Result<Import> {
    let mut cx = Cx {
        ws: workspace,
        strict,
        fixed_step: None,
        warnings: Vec::new(),
        problems: Vec::new(),
    };
    let solver = solver_config(&mut cx, sim);
    let mut model = convert_system(&mut cx, &sim.system, "");
    model.name = if sim.name.is_empty() {
        sim.system.name.clone()
    } else {
        sim.name.clone()
    };
    model.solver = solver;
    if !cx.problems.is_empty() {
        let n = cx.problems.len();
        let mut msg = format!(
            "the Simulink model cannot be converted exactly ({n} problem{}):",
            if n == 1 { "" } else { "s" }
        );
        for p in cx.problems.iter().take(50) {
            msg.push_str("\n  - ");
            msg.push_str(p);
        }
        if n > 50 {
            msg.push_str(&format!("\n  … and {} more", n - 50));
        }
        return Err(BlockError::Model(msg));
    }
    Ok(Import {
        model,
        warnings: cx.warnings,
    })
}

fn model_num(cx: &mut Cx, sim: &SimModel, key: &str) -> Option<f64> {
    let t = sim.param(key)?.trim();
    if t.is_empty() || t == "auto" {
        return None;
    }
    match expr::eval(t, cx.ws) {
        Ok(m) if m.data.len() == 1 && m.is_real() => m.data.first().map(|c| c.0),
        Ok(_) => {
            cx.problems
                .push(format!("model setting {key} = {t} is not a real number"));
            None
        }
        Err(e) => {
            cx.problems.push(format!("model setting {key} = {t}: {e}"));
            None
        }
    }
}

fn solver_config(cx: &mut Cx, sim: &SimModel) -> SolverConfig {
    let mut s = SolverConfig::default();
    let name = sim.param("Solver").map(str::trim).unwrap_or("ode45");
    let (method, note) = match name {
        "ode45" => (Method::Dopri5, None),
        "ode23" => (Method::Bs23, None),
        "ode23s" => (Method::Rosenbrock23, None),
        "ode1" => (Method::Euler, None),
        "ode2" => (Method::Heun, None),
        "ode3" => (Method::Bs3, None),
        "ode4" => (Method::Rk4, None),
        "VariableStepDiscrete" => (Method::Dopri5, None),
        "FixedStepDiscrete" | "discrete" => (Method::Euler, None),
        "ode113" => (Method::Dopri5, Some("Dormand–Prince 5(4) (ode45)")),
        "ode15s" | "ode23t" | "ode23tb" => (Method::Rosenbrock23, Some("Rosenbrock 2(3) (ode23s)")),
        "ode5" | "ode8" | "ode14x" => (Method::Rk4, Some("Runge–Kutta 4 (ode4)")),
        other => {
            cx.warnings.push(format!(
                "the solver \"{other}\" is not available; Dormand–Prince 5(4) (ode45) is used"
            ));
            (Method::Dopri5, None)
        }
    };
    if let Some(n) = note {
        cx.warnings
            .push(format!("the solver \"{name}\" is approximated by {n}"));
    }
    s.method = method;
    if let Some(v) = model_num(cx, sim, "StartTime") {
        s.start_time = v;
    }
    if let Some(v) = model_num(cx, sim, "StopTime") {
        s.stop_time = v;
    }
    if !s.start_time.is_finite() || !s.stop_time.is_finite() {
        cx.problems
            .push("the start and stop times must be finite".into());
    }
    let fixed = model_num(cx, sim, "FixedStep");
    cx.fixed_step = fixed.filter(|v| *v > 0.0 && v.is_finite());
    if method.is_fixed() {
        s.step = cx.fixed_step;
    } else {
        s.step = model_num(cx, sim, "InitialStep").filter(|v| *v > 0.0);
        s.max_step = model_num(cx, sim, "MaxStep").filter(|v| *v > 0.0 && v.is_finite());
    }
    if let Some(v) = model_num(cx, sim, "RelTol") {
        s.rtol = v;
    }
    if let Some(v) = model_num(cx, sim, "AbsTol") {
        s.atol = v;
    }
    s
}

/// A block's parameters, evaluated on demand.
struct P<'a> {
    b: &'a SimBlock,
    ws: &'a Workspace,
    fixed: Option<f64>,
    warnings: Vec<String>,
}

impl P<'_> {
    fn raw(&self, k: &str) -> Option<&str> {
        self.b.param(k).map(str::trim)
    }

    fn text(&self, k: &str, d: &str) -> String {
        self.raw(k).unwrap_or(d).to_string()
    }

    fn on(&self, k: &str, d: &str) -> bool {
        self.raw(k).unwrap_or(d).eq_ignore_ascii_case("on")
    }

    fn mat(&self, k: &str, d: &str) -> std::result::Result<Mat, String> {
        let t = self.raw(k).unwrap_or(d);
        let m = expr::eval(t, self.ws).map_err(|e| format!("parameter {k} = {t}: {e}"))?;
        if m.data.len() > MAX_ELEMENTS {
            return Err(format!("parameter {k} is too large"));
        }
        Ok(m)
    }

    fn real(&self, k: &str, d: &str) -> std::result::Result<Mat, String> {
        let m = self.mat(k, d)?;
        if !m.is_real() {
            return Err(format!(
                "parameter {k} = {} is complex",
                self.raw(k).unwrap_or(d)
            ));
        }
        Ok(m)
    }

    fn num(&self, k: &str, d: &str) -> std::result::Result<f64, String> {
        let m = self.real(k, d)?;
        match (m.data.len(), m.data.first()) {
            (1, Some(c)) => Ok(c.0),
            _ => Err(format!(
                "parameter {k} = {} must be a scalar",
                self.raw(k).unwrap_or(d)
            )),
        }
    }

    fn vec(&self, k: &str, d: &str) -> std::result::Result<Vec<f64>, String> {
        Ok(self.real(k, d)?.re())
    }

    fn rows(&self, k: &str, d: &str) -> std::result::Result<Vec<Vec<f64>>, String> {
        Ok(rows_of(&self.real(k, d)?))
    }

    fn count(&self, k: &str, d: &str) -> std::result::Result<usize, String> {
        let v = self.num(k, d)?;
        if v >= 0.0 && v.fract() == 0.0 && v <= 4096.0 {
            Ok(v as usize)
        } else {
            Err(format!(
                "parameter {k} = {} must be a whole number from 0 to 4096",
                self.raw(k).unwrap_or(d)
            ))
        }
    }

    /// A sample time that must be a positive number (or inherited from the fixed step).
    fn ts(&self, k: &str, d: &str) -> std::result::Result<f64, String> {
        let m = self.real(k, d)?;
        let v = m.re();
        if v.len() == 2 && v.get(1).copied() != Some(0.0) {
            return Err(format!("parameter {k} has a sample-time offset"));
        }
        let t = match v.first() {
            Some(t) if v.len() <= 2 => *t,
            _ => return Err(format!("parameter {k} is not a sample time")),
        };
        if t == -1.0 {
            return self.fixed.ok_or_else(|| {
                format!(
                    "parameter {k} inherits its sample time; set it, or give the model a fixed step"
                )
            });
        }
        if t > 0.0 && t.is_finite() {
            Ok(t)
        } else if t == 0.0 {
            Err(format!(
                "parameter {k} is 0 (continuous), but this block needs a sample time greater than 0"
            ))
        } else {
            Err(format!("parameter {k} = {t} is not a usable sample time"))
        }
    }

    fn signs(
        &self,
        k: &str,
        d: &str,
        allowed: &str,
        fill: char,
    ) -> std::result::Result<String, String> {
        let t = self.text(k, d).replace('|', "");
        let t = t.trim();
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
            let n: usize = t
                .parse()
                .map_err(|_| format!("parameter {k} is not valid"))?;
            if n == 0 || n > 4096 {
                return Err(format!("parameter {k} = {t} is out of range"));
            }
            return Ok(std::iter::repeat_n(fill, n).collect());
        }
        if t.is_empty() || !t.chars().all(|c| allowed.contains(c)) {
            return Err(format!("parameter {k} = {t} must use only {allowed}"));
        }
        Ok(t.to_string())
    }

    fn warn(&mut self, m: impl Into<String>) {
        self.warnings.push(m.into());
    }
}

fn rows_of(m: &Mat) -> Vec<Vec<f64>> {
    m.data
        .chunks(m.cols.max(1))
        .map(|r| r.iter().map(|c| c.0).collect())
        .collect()
}

fn rel_op(s: &str) -> std::result::Result<RelOp, String> {
    Ok(match s.trim() {
        "==" => RelOp::Eq,
        "~=" | "!=" => RelOp::Ne,
        "<" => RelOp::Lt,
        "<=" => RelOp::Le,
        ">" => RelOp::Gt,
        ">=" => RelOp::Ge,
        o => return Err(format!("the relational operator \"{o}\" is not known")),
    })
}

fn first_int(s: &str) -> Option<usize> {
    s.split(|c: char| !c.is_ascii_digit())
        .find(|t| !t.is_empty())
        .and_then(|t| t.parse().ok())
}

fn strip_wrap(s: &str) -> String {
    s.trim()
        .trim_matches(|c| matches!(c, '{' | '}' | '"' | '\''))
        .to_string()
}

/// A Simulink block as an Ábaco Blocks kind.
fn block_kind(p: &mut P) -> std::result::Result<BlockKind, String> {
    let ty = p.b.block_type.clone();
    Ok(match ty.as_str() {
        "Constant" => {
            let m = p.real("Value", "1")?;
            if m.rows > 1 && m.cols > 1 {
                return Err("a matrix constant is not supported (use a vector)".into());
            }
            BlockKind::Constant { value: m.re() }
        }
        "Step" => BlockKind::Step {
            time: p.num("Time", "1")?,
            before: p.num("Before", "0")?,
            after: p.num("After", "1")?,
        },
        "Ramp" => BlockKind::Ramp {
            slope: p.num("Slope", "1")?,
            start: p.num("Start", "0")?,
            initial: p.num("X0", "0")?,
        },
        "Sin" => {
            let kind = p.text("SineType", "Time based");
            if !kind.eq_ignore_ascii_case("time based") {
                return Err(format!("the Sine Wave type \"{kind}\" is not supported"));
            }
            if let Ok(ts) = p.num("SampleTime", "0") {
                if ts != 0.0 {
                    p.warn("a Sine Wave with a sample time is computed continuously");
                }
            }
            BlockKind::Sine {
                amplitude: p.num("Amplitude", "1")?,
                frequency: p.num("Frequency", "1")?,
                phase: p.num("Phase", "0")?,
                bias: p.num("Bias", "0")?,
            }
        }
        "DiscretePulseGenerator" => {
            let amplitude = p.num("Amplitude", "1")?;
            let period = p.num("Period", "10")?;
            let width = p.num("PulseWidth", "5")?;
            let delay = p.num("PhaseDelay", "0")?;
            let sample_based = p
                .text("PulseType", "Time based")
                .eq_ignore_ascii_case("sample based");
            if sample_based {
                let ts = p.ts("SampleTime", "1")?;
                if period <= 0.0 || !period.is_finite() {
                    return Err("the period must be greater than 0".into());
                }
                BlockKind::Pulse {
                    amplitude,
                    period: period * ts,
                    duty: width / period * 100.0,
                    delay: delay * ts,
                }
            } else {
                BlockKind::Pulse {
                    amplitude,
                    period,
                    duty: width,
                    delay,
                }
            }
        }
        "Clock" | "DigitalClock" => BlockKind::Clock,
        "Ground" => BlockKind::Ground,
        "RandomNumber" => BlockKind::Noise {
            mean: p.num("Mean", "0")?,
            variance: p.num("Variance", "1")?,
            seed: p.num("Seed", "0")?.abs().round() as usize,
            sample_time: p.ts("SampleTime", "0.1")?,
        },
        "SignalGenerator" => {
            let w = p.text("WaveForm", "sine");
            let shape = match w.as_str() {
                "sine" => WaveShape::Sine,
                "square" => WaveShape::Square,
                other => return Err(format!("the wave form \"{other}\" is not supported")),
            };
            let units = p.text("Units", "Hertz");
            let mut frequency = p.num("Frequency", "1")?;
            if units.to_ascii_lowercase().starts_with("rad") {
                frequency /= std::f64::consts::TAU;
            }
            BlockKind::Wave {
                shape,
                amplitude: p.num("Amplitude", "1")?,
                frequency,
            }
        }
        "Inport" => BlockKind::Inport {
            index: p.count("Port", "1")?.max(1),
        },
        "Outport" => BlockKind::Outport {
            index: p.count("Port", "1")?.max(1),
        },
        "Scope" => {
            let n = match p.raw("NumInputPorts") {
                Some(_) => p.count("NumInputPorts", "1")?,
                None => p.raw("Ports").and_then(first_int).unwrap_or(1),
            };
            BlockKind::Scope { inputs: n.max(1) }
        }
        "ToWorkspace" => BlockKind::ToWorkspace,
        "Display" => BlockKind::Display,
        "Terminator" => BlockKind::Terminator,
        "Stop" | "StopSimulation" => BlockKind::Stop,
        "Sum" => {
            let signs = p.signs("Inputs", "++", "+-", '+')?;
            match signs.as_str() {
                "+" => BlockKind::SumElements,
                "-" => return Err("a single negative input is not supported".into()),
                _ => BlockKind::Sum { signs },
            }
        }
        "Gain" => {
            let m = p.real("Gain", "1")?;
            let element_wise = p
                .text("Multiplication", "Element-wise(K.*u)")
                .starts_with("Element");
            if m.data.len() == 1 {
                BlockKind::Gain {
                    gain: m.data.first().map_or(1.0, |c| c.0),
                }
            } else if element_wise {
                let v = m.re();
                let n = v.len();
                let matrix = (0..n)
                    .map(|i| (0..n).map(|j| if i == j { v[i] } else { 0.0 }).collect())
                    .collect();
                BlockKind::MatrixGain { matrix }
            } else {
                BlockKind::MatrixGain {
                    matrix: rows_of(&m),
                }
            }
        }
        "Product" => {
            let ops = p.signs("Inputs", "2", "*/", '*')?;
            if p.text("Multiplication", "Element-wise(.*)")
                .starts_with("Matrix")
            {
                return Err("matrix multiplication in a Product block is not supported".into());
            }
            if ops.len() < 2 {
                return Err("a Product block with one input is not supported".into());
            }
            BlockKind::Product { ops }
        }
        "Math" => {
            let op = p.text("Operator", "exp");
            match op.as_str() {
                "exp" => BlockKind::Math {
                    func: MathFunc::Exp,
                },
                "log" => BlockKind::Math {
                    func: MathFunc::Log,
                },
                "log10" => BlockKind::Math {
                    func: MathFunc::Log10,
                },
                "10^u" => BlockKind::Math {
                    func: MathFunc::Pow10,
                },
                "square" | "magnitude^2" => BlockKind::Math {
                    func: MathFunc::Square,
                },
                "reciprocal" => BlockKind::Math {
                    func: MathFunc::Reciprocal,
                },
                "mod" => BlockKind::Mod,
                other => {
                    return Err(format!("the Math Function \"{other}\" is not supported"));
                }
            }
        }
        "Trigonometry" => {
            let op = p.text("Operator", "sin");
            let func = match op.as_str() {
                "sin" => MathFunc::Sin,
                "cos" => MathFunc::Cos,
                "tan" => MathFunc::Tan,
                "asin" => MathFunc::Asin,
                "acos" => MathFunc::Acos,
                "atan" => MathFunc::Atan,
                "sinh" => MathFunc::Sinh,
                "cosh" => MathFunc::Cosh,
                "tanh" => MathFunc::Tanh,
                "atan2" => return Ok(BlockKind::Atan2),
                other => {
                    return Err(format!(
                        "the Trigonometric Function \"{other}\" is not supported"
                    ))
                }
            };
            BlockKind::Math { func }
        }
        "Abs" => BlockKind::Math {
            func: MathFunc::Abs,
        },
        "Signum" => BlockKind::Math {
            func: MathFunc::Sign,
        },
        "Sqrt" => {
            let op = p.text("Operator", "sqrt");
            if op != "sqrt" {
                return Err(format!("the Sqrt operator \"{op}\" is not supported"));
            }
            BlockKind::Math {
                func: MathFunc::Sqrt,
            }
        }
        "Rounding" => {
            let op = p.text("Operator", "floor");
            let mode = match op.as_str() {
                "floor" => RoundMode::Floor,
                "ceil" => RoundMode::Ceil,
                "round" => RoundMode::Round,
                "fix" => RoundMode::Fix,
                other => {
                    return Err(format!(
                        "the rounding function \"{other}\" is not supported"
                    ))
                }
            };
            BlockKind::Rounding { mode }
        }
        "Bias" => BlockKind::Bias {
            bias: p.num("Bias", "0")?,
        },
        "Saturate" | "Saturation" => BlockKind::Saturation {
            lower: p.num("LowerLimit", "-0.5")?,
            upper: p.num("UpperLimit", "0.5")?,
        },
        "DeadZone" => BlockKind::DeadZone {
            start: p.num("LowerValue", "-0.5")?,
            end: p.num("UpperValue", "0.5")?,
        },
        "MinMax" => {
            let f = p.text("Function", "min");
            let mode = match f.as_str() {
                "min" => Extremum::Min,
                "max" => Extremum::Max,
                other => return Err(format!("the function \"{other}\" is not supported")),
            };
            let inputs = p.count("Inputs", "1")?;
            if inputs < 2 {
                return Err("a MinMax block with one input is not supported".into());
            }
            BlockKind::MinMax { mode, inputs }
        }
        "RelationalOperator" => BlockKind::Relational {
            op: rel_op(&p.text("Operator", "<="))?,
        },
        "Logic" => {
            let op = p.text("Operator", "AND");
            let (op, one) = match op.as_str() {
                "AND" => (LogicOp::And, false),
                "OR" => (LogicOp::Or, false),
                "XOR" => (LogicOp::Xor, false),
                "NAND" => (LogicOp::Nand, false),
                "NOR" => (LogicOp::Nor, false),
                "NOT" => (LogicOp::Not, true),
                other => return Err(format!("the logical operator \"{other}\" is not supported")),
            };
            let inputs = if one { 1 } else { p.count("Inputs", "2")? };
            if inputs == 0 || (!one && inputs < 2) {
                return Err("a logic block needs at least two inputs".into());
            }
            BlockKind::Logical { op, inputs }
        }
        "Switch" => {
            let c = p.text("Criteria", "u2 >= Threshold");
            let rule = match c.replace(' ', "").as_str() {
                "u2>=Threshold" => SwitchRule::Ge,
                "u2>Threshold" => SwitchRule::Gt,
                "u2~=0" => SwitchRule::NonZero,
                other => return Err(format!("the switch criterion \"{other}\" is not supported")),
            };
            BlockKind::Switch {
                rule,
                threshold: p.num("Threshold", "0")?,
            }
        }
        "Relay" => BlockKind::Relay {
            on_switch: p.num("OnSwitchValue", "eps")?,
            off_switch: p.num("OffSwitchValue", "eps")?,
            on_output: p.num("OnOutputValue", "1")?,
            off_output: p.num("OffOutputValue", "0")?,
        },
        "Quantizer" => BlockKind::Quantizer {
            interval: p.num("QuantizationInterval", "0.5")?,
        },
        "Lookup" => {
            let x = p.vec("InputValues", "[-5:5]")?;
            let y = p.vec("Output", "[-5:5]")?;
            check_table(&x, &y)?;
            BlockKind::Lookup1D { x, y }
        }
        "Lookup_n-D" => {
            let dims = p.count("NumberOfTableDimensions", "1")?;
            if dims != 1 {
                return Err(format!("a {dims}-D lookup table is not supported"));
            }
            let x = p.vec("BreakpointsForDimension1", "[1 2 3]")?;
            let y = p.vec("Table", "[1 2 3]")?;
            check_table(&x, &y)?;
            if p.text("InterpMethod", "Linear point-slope")
                .to_ascii_lowercase()
                .contains("nearest")
                || p.text("InterpMethod", "Linear")
                    .to_ascii_lowercase()
                    .contains("flat")
            {
                return Err("only linear interpolation is supported".into());
            }
            if p.text("ExtrapMethod", "Linear")
                .to_ascii_lowercase()
                .contains("linear")
            {
                p.warn("the lookup table is clipped at its ends instead of extrapolated");
            }
            BlockKind::Lookup1D { x, y }
        }
        "DotProduct" => BlockKind::DotProduct,
        "Mux" => BlockKind::Mux {
            inputs: p.count("Inputs", "4")?.max(1),
        },
        "Demux" => BlockKind::Demux {
            outputs: p.count("Outputs", "4")?.max(1),
        },
        "Selector" => {
            if let Some(o) = p.raw("IndexOptionArray") {
                if !o.to_ascii_lowercase().contains("dialog") {
                    return Err(
                        "a Selector that takes its indices from a port is not supported".into(),
                    );
                }
            }
            let text = match p.raw("Indices") {
                Some(t) => t.to_string(),
                None => p.text("IndexParamArray", "[1]"),
            };
            let text = strip_wrap(&text);
            let m = expr::eval(&text, p.ws).map_err(|e| format!("indices {text}: {e}"))?;
            let zero = p
                .text("IndexMode", "One-based")
                .to_ascii_lowercase()
                .starts_with("zero");
            let mut indices = Vec::new();
            for c in &m.data {
                let v = c.0 + if zero { 1.0 } else { 0.0 };
                if c.1 != 0.0 || v < 1.0 || v.fract() != 0.0 || v > 1e6 {
                    return Err(format!("the index {} is not valid", c.0));
                }
                indices.push(v as usize);
            }
            BlockKind::Selector { indices }
        }
        "Goto" => BlockKind::Goto {
            tag: p.text("GotoTag", "A"),
        },
        "From" => BlockKind::From {
            tag: p.text("GotoTag", "A"),
        },
        "Integrator" => {
            if !p.text("ExternalReset", "none").eq_ignore_ascii_case("none") {
                return Err("an Integrator with an external reset is not supported".into());
            }
            if !p
                .text("InitialConditionSource", "internal")
                .eq_ignore_ascii_case("internal")
            {
                return Err(
                    "an Integrator with an external initial condition is not supported".into(),
                );
            }
            if p.on("ShowStatePort", "off") {
                return Err("the state port of an Integrator is not supported".into());
            }
            let (mut lower, mut upper) = (None, None);
            if p.on("LimitOutput", "off") {
                let lo = p.num("LowerSaturationLimit", "-inf")?;
                let hi = p.num("UpperSaturationLimit", "inf")?;
                lower = lo.is_finite().then_some(lo);
                upper = hi.is_finite().then_some(hi);
            }
            BlockKind::Integrator {
                initial: p.vec("InitialCondition", "0")?,
                lower,
                upper,
            }
        }
        "Derivative" => {
            let n = p.num("Coefficient", "inf")?;
            if n.is_finite() && n > 0.0 {
                BlockKind::Derivative { n }
            } else {
                p.warn("a pure derivative cannot be simulated; it is approximated by a derivative filtered at 10000 rad/s");
                BlockKind::Derivative { n: 1e4 }
            }
        }
        "TransferFcn" => BlockKind::TransferFcn {
            num: siso(p, "Numerator", "[1]")?,
            den: siso(p, "Denominator", "[1 1]")?,
        },
        "ZeroPole" => BlockKind::ZeroPole {
            zeros: roots(p, "Zeros", "[]")?,
            poles: roots(p, "Poles", "[-1]")?,
            gain: p.num("Gain", "1")?,
        },
        "StateSpace" => {
            let (a, b, c, d) = (
                p.rows("A", "1")?,
                p.rows("B", "1")?,
                p.rows("C", "1")?,
                p.rows("D", "1")?,
            );
            BlockKind::StateSpace {
                x0: initial_state(p, a.len())?,
                a,
                b,
                c,
                d,
            }
        }
        "TransportDelay" => BlockKind::TransportDelay {
            delay: p.num("DelayTime", "1")?,
            initial: p.num("InitialOutput", "0")?,
        },
        "UnitDelay" => BlockKind::UnitDelay {
            initial: p.vec("InitialCondition", "0")?,
            sample_time: p.ts("SampleTime", "1")?,
        },
        "Delay" => {
            if !p
                .text("DelayLengthSource", "Dialog")
                .eq_ignore_ascii_case("dialog")
            {
                return Err("a Delay whose length comes from a port is not supported".into());
            }
            if !p.text("ExternalReset", "None").eq_ignore_ascii_case("none") {
                return Err("a Delay with an external reset is not supported".into());
            }
            BlockKind::Delay {
                length: p.count("DelayLength", "1")?,
                initial: p.num("InitialCondition", "0")?,
                sample_time: p.ts("SampleTime", "1")?,
            }
        }
        "ZeroOrderHold" => BlockKind::ZeroOrderHold {
            sample_time: p.ts("SampleTime", "1")?,
        },
        "DiscreteIntegrator" => {
            let m = p.text("IntegratorMethod", "Integration: Forward Euler");
            if m.starts_with("Accumulation") {
                return Err("accumulation (without the sample time) is not supported".into());
            }
            let rule = if m.contains("Forward") {
                DiscreteRule::ForwardEuler
            } else if m.contains("Backward") {
                DiscreteRule::BackwardEuler
            } else if m.contains("Trapezoidal") {
                DiscreteRule::Trapezoidal
            } else {
                return Err(format!("the integration method \"{m}\" is not supported"));
            };
            if p.on("LimitOutput", "off") {
                return Err(
                    "a Discrete-Time Integrator with limited output is not supported".into(),
                );
            }
            if !p.text("ExternalReset", "none").eq_ignore_ascii_case("none") {
                return Err(
                    "a Discrete-Time Integrator with an external reset is not supported".into(),
                );
            }
            BlockKind::DiscreteIntegrator {
                rule,
                gain: p.num("gainval", "1")?,
                initial: p.num("InitialCondition", "0")?,
                sample_time: p.ts("SampleTime", "1")?,
            }
        }
        "DiscreteTransferFcn" => BlockKind::DiscreteTransferFcn {
            num: siso(p, "Numerator", "[1]")?,
            den: siso(p, "Denominator", "[1 1]")?,
            sample_time: p.ts("SampleTime", "1")?,
        },
        "DiscreteStateSpace" => {
            let (a, b, c, d) = (
                p.rows("A", "1")?,
                p.rows("B", "1")?,
                p.rows("C", "1")?,
                p.rows("D", "1")?,
            );
            BlockKind::DiscreteStateSpace {
                x0: initial_state(p, a.len())?,
                a,
                b,
                c,
                d,
                sample_time: p.ts("SampleTime", "1")?,
            }
        }
        "Reference" => {
            return Err(
                "a link to a library block is not supported (open the library and copy the block)"
                    .into(),
            )
        }
        "ModelReference" => return Err("a model reference is not supported".into()),
        "" => return Err("the block has no type".into()),
        other => return Err(format!("the block type \"{other}\" is not supported")),
    })
}

fn check_table(x: &[f64], y: &[f64]) -> std::result::Result<(), String> {
    if x.len() != y.len() || x.is_empty() {
        return Err("the breakpoints and the table must have the same, non-zero, length".into());
    }
    if !x.windows(2).all(|w| w[0] < w[1]) {
        return Err("the breakpoints must be increasing".into());
    }
    Ok(())
}

fn siso(p: &P, k: &str, d: &str) -> std::result::Result<Vec<f64>, String> {
    let m = p.real(k, d)?;
    if m.rows > 1 {
        return Err(format!(
            "{k}: multiple-output transfer functions are not supported"
        ));
    }
    Ok(m.re())
}

fn roots(p: &P, k: &str, d: &str) -> std::result::Result<Vec<[f64; 2]>, String> {
    let m = p.mat(k, d)?;
    if m.rows > 1 {
        return Err(format!("{k}: multiple-output models are not supported"));
    }
    Ok(m.data.iter().map(|c| [c.0, c.1]).collect())
}

fn initial_state(p: &P, n: usize) -> std::result::Result<Vec<f64>, String> {
    let v = p.vec("X0", "0")?;
    if v.len() == 1 && n != 1 {
        Ok(vec![v[0]; n])
    } else {
        Ok(v)
    }
}

/// A name valid in Ábaco Blocks, unique among `used`.
fn clean_name(raw: &str, used: &mut BTreeSet<String>) -> String {
    let mut base: String = raw
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\n' | '\r') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if base.trim().is_empty() {
        base = "Block".into();
    }
    let mut name = base.clone();
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    used.insert(name.clone());
    name
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}/{name}")
    }
}

fn convert_system(cx: &mut Cx, sys: &SimSystem, path: &str) -> Model {
    let mut model = Model::new(sys.name.clone());
    let mut names: HashMap<&str, String> = HashMap::new();
    let mut used = BTreeSet::new();
    // Names first, so the first block of a name keeps it. A To Workspace block takes its
    // variable's name when that is free: results are recorded under the block's name.
    for b in sys.blocks.iter().filter(|b| b.block_type != "ToWorkspace") {
        names.insert(b.name.as_str(), clean_name(&b.name, &mut used));
    }
    for b in sys.blocks.iter().filter(|b| b.block_type == "ToWorkspace") {
        let wanted = b
            .param("VariableName")
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let name = match wanted {
            Some(v) if !used.contains(v) && !v.contains('/') => {
                used.insert(v.to_string());
                v.to_string()
            }
            _ => {
                if let Some(v) = wanted {
                    cx.warnings.push(format!(
                        "{}: the variable \"{v}\" is taken; the result is recorded under \"{}\"",
                        join(path, &b.name),
                        b.name
                    ));
                }
                clean_name(&b.name, &mut used)
            }
        };
        names.insert(b.name.as_str(), name);
    }

    let pow_folds = fold_powers(sys);
    let mut dropped: BTreeSet<&str> = BTreeSet::new();
    let mut folded_lines: BTreeSet<usize> = BTreeSet::new();
    let (mut next_in, mut next_out) = (0usize, 0usize);
    for (idx, b) in sys.blocks.iter().enumerate() {
        let here = join(path, &b.name);
        let Some(name) = names.get(b.name.as_str()).cloned() else {
            continue;
        };
        if let Some(f) = pow_folds.get(&idx) {
            folded_lines.insert(f.line);
            if f.constant_unused {
                dropped.insert(f.constant.as_str());
            }
        }
        if b.block_type == "Inport" {
            next_in += 1;
        }
        if b.block_type == "Outport" {
            next_out += 1;
        }
        if dropped.contains(b.name.as_str()) {
            continue;
        }
        let converted: std::result::Result<BlockKind, String> = if b.block_type == "SubSystem" {
            convert_subsystem(cx, b, &here)
        } else if let Some(f) = pow_folds.get(&idx) {
            Ok(BlockKind::Power {
                exponent: f.exponent,
            })
        } else {
            let mut p = P {
                b,
                ws: cx.ws,
                fixed: cx.fixed_step,
                warnings: Vec::new(),
            };
            let r = block_kind(&mut p);
            for w in p.warnings {
                cx.warnings.push(format!("{here}: {w}"));
            }
            r.map(|mut k| {
                // Unnumbered ports are numbered in order of appearance, as Simulink does.
                match &mut k {
                    BlockKind::Inport { index } if b.param("Port").is_none() => *index = next_in,
                    BlockKind::Outport { index } if b.param("Port").is_none() => *index = next_out,
                    _ => {}
                }
                k
            })
        };
        match converted {
            Ok(kind) => {
                let position = b.param("Position").and_then(|t| {
                    let m = expr::eval(t, cx.ws).ok()?;
                    let v = m.re();
                    (v.len() == 4 && v.iter().all(|x| x.is_finite())).then(|| (v[0], v[1]))
                });
                model.blocks.push(Block {
                    name,
                    kind,
                    position,
                });
            }
            Err(msg) => {
                dropped.insert(b.name.as_str());
                let what = format!("{here} ({}): {msg}", b.block_type);
                if cx.strict {
                    cx.problems.push(what);
                } else {
                    cx.warnings.push(format!("dropped {what}"));
                }
            }
        }
    }

    for (i, l) in sys.lines.iter().enumerate() {
        if folded_lines.contains(&i) {
            continue;
        }
        let (src, dst) = (l.src.0.as_str(), l.dst.0.as_str());
        if dropped.contains(src) || dropped.contains(dst) {
            if !cx.strict {
                cx.warnings.push(format!(
                    "{}: the line from \"{src}\" to \"{dst}\" is dropped with its block",
                    if path.is_empty() { "(model)" } else { path }
                ));
            }
            continue;
        }
        let (Some(from), Some(to)) = (names.get(src), names.get(dst)) else {
            let what = format!(
                "{}: a line joins \"{src}\" and \"{dst}\", and one of them does not exist",
                if path.is_empty() { "(model)" } else { path }
            );
            if cx.strict {
                cx.problems.push(what);
            } else {
                cx.warnings.push(what);
            }
            continue;
        };
        if l.src.1 == 0 || l.dst.1 == 0 {
            let what = format!(
                "{}: the line from \"{src}\" to \"{dst}\" uses a special port (trigger, enable, action or physical connection), which is not supported",
                if path.is_empty() { "(model)" } else { path }
            );
            if cx.strict {
                cx.problems.push(what);
            } else {
                cx.warnings.push(what);
            }
            continue;
        }
        model.lines.push(Line {
            from: (from.clone(), l.src.1 - 1),
            to: (to.clone(), l.dst.1 - 1),
        });
    }
    model
}

fn convert_subsystem(
    cx: &mut Cx,
    b: &SimBlock,
    here: &str,
) -> std::result::Result<BlockKind, String> {
    let Some(inner) = &b.system else {
        return Err("the subsystem has no diagram (a library link or a model reference?)".into());
    };
    if let Some(special) = inner.blocks.iter().find(|x| {
        matches!(
            x.block_type.as_str(),
            "TriggerPort" | "EnablePort" | "ActionPort" | "ResetPort"
        )
    }) {
        return Err(format!(
            "a triggered, enabled or conditionally executed subsystem ({}) is not supported",
            special.block_type
        ));
    }
    let mut m = convert_system(cx, inner, here);
    m.name = b.name.clone();
    Ok(BlockKind::Subsystem { model: Box::new(m) })
}

struct PowFold {
    exponent: f64,
    line: usize,
    constant: String,
    constant_unused: bool,
}

/// `Math Function: pow` whose second input is a scalar Constant used only there becomes a Power
/// block with that exponent (Ábaco Blocks' Power has the exponent as a parameter).
fn fold_powers(sys: &SimSystem) -> HashMap<usize, PowFold> {
    let mut out = HashMap::new();
    for (i, b) in sys.blocks.iter().enumerate() {
        if b.block_type != "Math" || b.param("Operator").map(str::trim) != Some("pow") {
            continue;
        }
        let Some((li, line)) = sys
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.dst == (b.name.clone(), 2))
        else {
            continue;
        };
        let Some(c) = sys
            .blocks
            .iter()
            .find(|x| x.name == line.src.0 && x.block_type == "Constant")
        else {
            continue;
        };
        let Some(v) = c
            .param("Value")
            .and_then(|t| expr::eval(t, &Workspace::new()).ok())
            .filter(|m| m.data.len() == 1 && m.is_real())
            .and_then(|m| m.data.first().map(|c| c.0))
        else {
            continue;
        };
        let uses = sys.lines.iter().filter(|l| l.src.0 == c.name).count();
        out.insert(
            i,
            PowFold {
                exponent: v,
                line: li,
                constant: c.name.clone(),
                constant_unused: uses == 1,
            },
        );
    }
    out
}

// ───────────────────────────── export ─────────────────────────────

fn f(x: f64) -> String {
    format!("{x}")
}

fn vecs(v: &[f64]) -> String {
    match v {
        [x] => f(*x),
        _ => format!(
            "[{}]",
            v.iter().map(|x| f(*x)).collect::<Vec<_>>().join(" ")
        ),
    }
}

fn mats(m: &[Vec<f64>]) -> String {
    format!(
        "[{}]",
        m.iter()
            .map(|r| r.iter().map(|x| f(*x)).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("; ")
    )
}

fn complex(r: [f64; 2]) -> String {
    if r[1] == 0.0 {
        f(r[0])
    } else if r[1] < 0.0 {
        format!("{}-{}i", f(r[0]), f(-r[1]))
    } else {
        format!("{}+{}i", f(r[0]), f(r[1]))
    }
}

fn roots_text(r: &[[f64; 2]]) -> String {
    format!(
        "[{}]",
        r.iter().map(|x| complex(*x)).collect::<Vec<_>>().join(" ")
    )
}

struct Emit {
    ty: &'static str,
    params: Vec<(String, String)>,
    /// A Constant that feeds the second input (blocks whose parameter is a second input).
    helper: Option<f64>,
}

impl Emit {
    fn new(ty: &'static str) -> Emit {
        Emit {
            ty,
            params: Vec::new(),
            helper: None,
        }
    }
    fn p(mut self, k: &str, v: impl Into<String>) -> Emit {
        self.params.push((k.to_string(), v.into()));
        self
    }
    fn n(self, k: &str, v: f64) -> Emit {
        self.p(k, f(v))
    }
}

fn trig(name: &'static str) -> Emit {
    Emit::new("Trigonometry").p("Operator", name)
}

fn mathfn(name: &'static str) -> Emit {
    Emit::new("Math").p("Operator", name)
}

fn rel_text(op: RelOp) -> &'static str {
    match op {
        RelOp::Eq => "==",
        RelOp::Ne => "~=",
        RelOp::Lt => "<",
        RelOp::Le => "<=",
        RelOp::Gt => ">",
        RelOp::Ge => ">=",
    }
}

fn emit_kind(path: &str, kind: &BlockKind, name: &str) -> Result<Emit> {
    let no = |what: &str| -> BlockError {
        BlockError::block(
            path,
            format!("{what} has no Simulink block to export it to"),
        )
    };
    Ok(match kind {
        BlockKind::Constant { value } => Emit::new("Constant").p("Value", vecs(value)),
        BlockKind::Step {
            time,
            before,
            after,
        } => Emit::new("Step")
            .n("Time", *time)
            .n("Before", *before)
            .n("After", *after)
            .p("SampleTime", "0"),
        BlockKind::Ramp {
            slope,
            start,
            initial,
        } => Emit::new("Ramp")
            .n("Slope", *slope)
            .n("Start", *start)
            .n("X0", *initial),
        BlockKind::Sine {
            amplitude,
            frequency,
            phase,
            bias,
        } => Emit::new("Sin")
            .p("SineType", "Time based")
            .n("Amplitude", *amplitude)
            .n("Bias", *bias)
            .n("Frequency", *frequency)
            .n("Phase", *phase)
            .p("SampleTime", "0"),
        BlockKind::Pulse {
            amplitude,
            period,
            duty,
            delay,
        } => Emit::new("DiscretePulseGenerator")
            .p("PulseType", "Time based")
            .n("Amplitude", *amplitude)
            .n("Period", *period)
            .n("PulseWidth", *duty)
            .n("PhaseDelay", *delay)
            .p("SampleTime", "0"),
        BlockKind::Wave {
            shape,
            amplitude,
            frequency,
        } => {
            let w = match shape {
                WaveShape::Sine => "sine",
                WaveShape::Square => "square",
                WaveShape::Sawtooth | WaveShape::Triangle => {
                    return Err(no("a sawtooth or triangle wave"))
                }
            };
            Emit::new("SignalGenerator")
                .p("WaveForm", w)
                .n("Amplitude", *amplitude)
                .n("Frequency", *frequency)
                .p("Units", "Hertz")
        }
        BlockKind::Chirp { .. } => return Err(no("a chirp signal")),
        BlockKind::Clock => Emit::new("Clock"),
        BlockKind::Ground => Emit::new("Ground"),
        BlockKind::Noise {
            mean,
            variance,
            seed,
            sample_time,
        } => Emit::new("RandomNumber")
            .n("Mean", *mean)
            .n("Variance", *variance)
            .n("Seed", *seed as f64)
            .n("SampleTime", *sample_time),
        BlockKind::Table { .. } => return Err(no("a time table")),
        BlockKind::Inport { index } => Emit::new("Inport").n("Port", *index as f64),
        BlockKind::Scope { inputs } => Emit::new("Scope").n("NumInputPorts", *inputs as f64),
        BlockKind::ToWorkspace => Emit::new("ToWorkspace")
            .p("VariableName", name)
            .p("MaxDataPoints", "inf")
            .p("SampleTime", "-1"),
        BlockKind::Display => Emit::new("Display"),
        BlockKind::Terminator => Emit::new("Terminator"),
        BlockKind::Stop => Emit::new("Stop"),
        BlockKind::Outport { index } => Emit::new("Outport").n("Port", *index as f64),
        BlockKind::Sum { signs } => Emit::new("Sum")
            .p("IconShape", "rectangular")
            .p("Inputs", signs.as_str()),
        BlockKind::SumElements => Emit::new("Sum")
            .p("IconShape", "rectangular")
            .p("Inputs", "+"),
        BlockKind::Gain { gain } => Emit::new("Gain")
            .n("Gain", *gain)
            .p("Multiplication", "Element-wise(K.*u)"),
        BlockKind::MatrixGain { matrix } => Emit::new("Gain")
            .p("Gain", mats(matrix))
            .p("Multiplication", "Matrix(K*u)"),
        BlockKind::Product { ops } => Emit::new("Product")
            .p("Inputs", ops.as_str())
            .p("Multiplication", "Element-wise(.*)"),
        BlockKind::Math { func } => match func {
            MathFunc::Exp => mathfn("exp"),
            MathFunc::Log => mathfn("log"),
            MathFunc::Log10 => mathfn("log10"),
            MathFunc::Sqrt => Emit::new("Sqrt").p("Operator", "sqrt"),
            MathFunc::Square => mathfn("square"),
            MathFunc::Reciprocal => mathfn("reciprocal"),
            MathFunc::Sin => trig("sin"),
            MathFunc::Cos => trig("cos"),
            MathFunc::Tan => trig("tan"),
            MathFunc::Asin => trig("asin"),
            MathFunc::Acos => trig("acos"),
            MathFunc::Atan => trig("atan"),
            MathFunc::Sinh => trig("sinh"),
            MathFunc::Cosh => trig("cosh"),
            MathFunc::Tanh => trig("tanh"),
            MathFunc::Pow10 => mathfn("10^u"),
            MathFunc::Abs => Emit::new("Abs"),
            MathFunc::Sign => Emit::new("Signum"),
        },
        BlockKind::Power { exponent } => {
            let mut e = mathfn("pow");
            e.helper = Some(*exponent);
            e
        }
        BlockKind::Atan2 => trig("atan2"),
        BlockKind::Mod => mathfn("mod"),
        BlockKind::Saturation { lower, upper } => Emit::new("Saturate")
            .n("UpperLimit", *upper)
            .n("LowerLimit", *lower),
        BlockKind::DeadZone { start, end } => Emit::new("DeadZone")
            .n("LowerValue", *start)
            .n("UpperValue", *end),
        BlockKind::Bias { bias } => Emit::new("Bias").n("Bias", *bias),
        BlockKind::Rounding { mode } => Emit::new("Rounding").p(
            "Operator",
            match mode {
                RoundMode::Floor => "floor",
                RoundMode::Ceil => "ceil",
                RoundMode::Round => "round",
                RoundMode::Fix => "fix",
            },
        ),
        BlockKind::MinMax { mode, inputs } => Emit::new("MinMax")
            .p(
                "Function",
                if *mode == Extremum::Min { "min" } else { "max" },
            )
            .n("Inputs", *inputs as f64),
        BlockKind::Relational { op } => {
            Emit::new("RelationalOperator").p("Operator", rel_text(*op))
        }
        BlockKind::CompareConst { op, value } => {
            let mut e = Emit::new("RelationalOperator").p("Operator", rel_text(*op));
            e.helper = Some(*value);
            e
        }
        BlockKind::Logical { op, inputs } => {
            let o = match op {
                LogicOp::And => "AND",
                LogicOp::Or => "OR",
                LogicOp::Xor => "XOR",
                LogicOp::Nand => "NAND",
                LogicOp::Nor => "NOR",
                LogicOp::Not => "NOT",
            };
            Emit::new("Logic")
                .p("Operator", o)
                .n("Inputs", *inputs as f64)
        }
        BlockKind::Switch { rule, threshold } => Emit::new("Switch")
            .p(
                "Criteria",
                match rule {
                    SwitchRule::Ge => "u2 >= Threshold",
                    SwitchRule::Gt => "u2 > Threshold",
                    SwitchRule::NonZero => "u2 ~= 0",
                },
            )
            .n("Threshold", *threshold),
        BlockKind::Relay {
            on_switch,
            off_switch,
            on_output,
            off_output,
        } => Emit::new("Relay")
            .n("OnSwitchValue", *on_switch)
            .n("OffSwitchValue", *off_switch)
            .n("OnOutputValue", *on_output)
            .n("OffOutputValue", *off_output),
        BlockKind::Quantizer { interval } => {
            Emit::new("Quantizer").n("QuantizationInterval", *interval)
        }
        BlockKind::Lookup1D { x, y } => Emit::new("Lookup_n-D")
            .p("NumberOfTableDimensions", "1")
            .p("BreakpointsForDimension1", format!("[{}]", join_nums(x)))
            .p("Table", format!("[{}]", join_nums(y)))
            .p("InterpMethod", "Linear point-slope")
            .p("ExtrapMethod", "Clip"),
        BlockKind::DotProduct => Emit::new("DotProduct"),
        BlockKind::Mux { inputs } => Emit::new("Mux").n("Inputs", *inputs as f64),
        BlockKind::Demux { outputs } => Emit::new("Demux").n("Outputs", *outputs as f64),
        BlockKind::Selector { indices } => Emit::new("Selector")
            .p("IndexMode", "One-based")
            .p(
                "Indices",
                format!(
                    "[{}]",
                    indices
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )
            .p("InputPortWidth", "-1"),
        BlockKind::Goto { tag } => Emit::new("Goto")
            .p("GotoTag", tag.as_str())
            .p("TagVisibility", "local"),
        BlockKind::From { tag } => Emit::new("From").p("GotoTag", tag.as_str()),
        BlockKind::Subsystem { .. } => Emit::new("SubSystem"),
        BlockKind::Integrator {
            initial,
            lower,
            upper,
        } => {
            let limited = lower.is_some() || upper.is_some();
            Emit::new("Integrator")
                .p("InitialCondition", vecs(initial))
                .p("LimitOutput", if limited { "on" } else { "off" })
                .p("UpperSaturationLimit", upper.map_or("inf".to_string(), f))
                .p("LowerSaturationLimit", lower.map_or("-inf".to_string(), f))
        }
        BlockKind::Derivative { n } => Emit::new("Derivative").n("Coefficient", *n),
        BlockKind::TransferFcn { num, den } => Emit::new("TransferFcn")
            .p("Numerator", format!("[{}]", join_nums(num)))
            .p("Denominator", format!("[{}]", join_nums(den))),
        BlockKind::ZeroPole { zeros, poles, gain } => Emit::new("ZeroPole")
            .p("Zeros", roots_text(zeros))
            .p("Poles", roots_text(poles))
            .n("Gain", *gain),
        BlockKind::StateSpace { a, b, c, d, x0 } => Emit::new("StateSpace")
            .p("A", mats(a))
            .p("B", mats(b))
            .p("C", mats(c))
            .p("D", mats(d))
            .p("X0", vecs_or_zero(x0)),
        BlockKind::Pid { .. } => return Err(no("a PID controller")),
        BlockKind::TransportDelay { delay, initial } => Emit::new("TransportDelay")
            .n("DelayTime", *delay)
            .n("InitialOutput", *initial),
        BlockKind::UnitDelay {
            initial,
            sample_time,
        } => Emit::new("UnitDelay")
            .p("InitialCondition", vecs(initial))
            .n("SampleTime", *sample_time),
        BlockKind::Delay {
            length,
            initial,
            sample_time,
        } => Emit::new("Delay")
            .p("DelayLengthSource", "Dialog")
            .n("DelayLength", *length as f64)
            .n("InitialCondition", *initial)
            .n("SampleTime", *sample_time),
        BlockKind::ZeroOrderHold { sample_time } => {
            Emit::new("ZeroOrderHold").n("SampleTime", *sample_time)
        }
        BlockKind::DiscreteIntegrator {
            rule,
            gain,
            initial,
            sample_time,
        } => Emit::new("DiscreteIntegrator")
            .p(
                "IntegratorMethod",
                match rule {
                    DiscreteRule::ForwardEuler => "Integration: Forward Euler",
                    DiscreteRule::BackwardEuler => "Integration: Backward Euler",
                    DiscreteRule::Trapezoidal => "Integration: Trapezoidal",
                },
            )
            .n("gainval", *gain)
            .n("InitialCondition", *initial)
            .n("SampleTime", *sample_time),
        BlockKind::DiscreteTransferFcn {
            num,
            den,
            sample_time,
        } => Emit::new("DiscreteTransferFcn")
            .p("Numerator", format!("[{}]", join_nums(num)))
            .p("Denominator", format!("[{}]", join_nums(den)))
            .n("SampleTime", *sample_time),
        BlockKind::DiscreteStateSpace {
            a,
            b,
            c,
            d,
            x0,
            sample_time,
        } => Emit::new("DiscreteStateSpace")
            .p("A", mats(a))
            .p("B", mats(b))
            .p("C", mats(c))
            .p("D", mats(d))
            .p("X0", vecs_or_zero(x0))
            .n("SampleTime", *sample_time),
        BlockKind::DiscreteDerivative { .. } => return Err(no("a discrete derivative")),
        BlockKind::Identity => return Err(no("an internal pass-through")),
    })
}

fn join_nums(v: &[f64]) -> String {
    v.iter().map(|x| f(*x)).collect::<Vec<_>>().join(" ")
}

fn vecs_or_zero(v: &[f64]) -> String {
    if v.is_empty() {
        "0".into()
    } else {
        vecs(v)
    }
}

/// The Simulink model for `model`; every parameter is written out.
pub fn to_simulink(model: &Model) -> Result<SimModel> {
    let mut m = SimModel {
        name: if model.name.is_empty() {
            "untitled".into()
        } else {
            model.name.clone()
        },
        ..SimModel::default()
    };
    let s = &model.solver;
    let (name, fixed) = match s.method {
        Method::Dopri5 => ("ode45", false),
        Method::Bs23 => ("ode23", false),
        Method::Rosenbrock23 => ("ode23s", false),
        Method::Euler => ("ode1", true),
        Method::Heun => ("ode2", true),
        Method::Bs3 => ("ode3", true),
        Method::Rk4 => ("ode4", true),
    };
    let auto = |v: Option<f64>| v.map_or("auto".to_string(), f);
    let mut put = |k: &str, v: String| m.params.push((k.to_string(), v));
    put("Solver", name.into());
    put(
        "SolverType",
        if fixed { "Fixed-step" } else { "Variable-step" }.into(),
    );
    put("StartTime", f(s.start_time));
    put("StopTime", f(s.stop_time));
    if fixed {
        let step = s.step.unwrap_or((s.stop_time - s.start_time) / 1000.0);
        put("FixedStep", f(step));
    } else {
        put("InitialStep", auto(s.step));
        put("MaxStep", auto(s.max_step));
    }
    put("RelTol", f(s.rtol));
    put("AbsTol", f(s.atol));
    m.system = emit_system(model, "")?;
    m.system.name = m.name.clone();
    Ok(m)
}

fn emit_system(model: &Model, path: &str) -> Result<SimSystem> {
    let mut sys = SimSystem {
        name: model.name.clone(),
        ..SimSystem::default()
    };
    let mut used: BTreeSet<String> = model.blocks.iter().map(|b| b.name.clone()).collect();
    let known: BTreeMap<&str, ()> = model.blocks.iter().map(|b| (b.name.as_str(), ())).collect();
    let mut helpers = Vec::new();
    for (i, b) in model.blocks.iter().enumerate() {
        let here = join(path, &b.name);
        let e = emit_kind(&here, &b.kind, &b.name)?;
        let (ins, outs) = b
            .kind
            .ports()
            .ok_or_else(|| BlockError::block(&here, "its parameters are not valid"))?;
        let mut blk = SimBlock {
            block_type: e.ty.to_string(),
            name: b.name.clone(),
            params: e.params,
            system: None,
        };
        let ports = if let BlockKind::Scope { inputs } = b.kind {
            format!("[{inputs}]")
        } else if matches!(b.kind, BlockKind::Inport { .. }) {
            "[0, 1]".to_string()
        } else if matches!(b.kind, BlockKind::Outport { .. }) {
            "[1]".to_string()
        } else {
            let ins = ins + usize::from(e.helper.is_some());
            format!("[{ins}, {outs}]")
        };
        blk.set("Ports", ports);
        let (x, y) = b
            .position
            .unwrap_or((50.0 + 150.0 * (i % 8) as f64, 50.0 + 90.0 * (i / 8) as f64));
        let h = 30.0f64.max(20.0 * ins.max(outs) as f64);
        blk.set(
            "Position",
            format!("[{}, {}, {}, {}]", f(x), f(y), f(x + 30.0), f(y + h)),
        );
        if let BlockKind::Subsystem { model: inner } = &b.kind {
            let mut s = emit_system(inner, &here)?;
            s.name = b.name.clone();
            blk.system = Some(s);
        }
        if let Some(v) = e.helper {
            let mut hn = format!("{}_const", b.name);
            while used.contains(&hn) {
                hn.push('_');
            }
            used.insert(hn.clone());
            let mut c = SimBlock {
                block_type: "Constant".into(),
                name: hn.clone(),
                params: vec![("Value".into(), f(v)), ("Ports".into(), "[0, 1]".into())],
                system: None,
            };
            c.set(
                "Position",
                format!(
                    "[{}, {}, {}, {}]",
                    f(x - 90.0),
                    f(y + 40.0),
                    f(x - 60.0),
                    f(y + 70.0)
                ),
            );
            helpers.push(SimLine {
                src: (hn, 1),
                dst: (b.name.clone(), 2),
            });
            sys.blocks.push(blk);
            sys.blocks.push(c);
        } else {
            sys.blocks.push(blk);
        }
    }
    for l in &model.lines {
        for end in [&l.from.0, &l.to.0] {
            if !known.contains_key(end.as_str()) {
                return Err(BlockError::Model(format!(
                    "{}: a line refers to the block \"{end}\", which does not exist",
                    if path.is_empty() { "(model)" } else { path }
                )));
            }
        }
        sys.lines.push(SimLine {
            src: (l.from.0.clone(), l.from.1 + 1),
            dst: (l.to.0.clone(), l.to.1 + 1),
        });
    }
    sys.lines.extend(helpers);
    Ok(sys)
}

// ───────────────────────────── tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulate;

    fn strip(m: &mut Model) {
        for b in &mut m.blocks {
            b.position = None;
            if let BlockKind::Subsystem { model } = &mut b.kind {
                strip(model);
            }
        }
    }

    fn sample() -> Model {
        let mut m = Model::new("demo");
        m.solver = SolverConfig {
            method: Method::Rk4,
            start_time: 0.0,
            stop_time: 5.0,
            step: Some(0.01),
            max_step: None,
            rtol: 1e-3,
            atol: 1e-6,
        };
        let kinds: Vec<(&str, BlockKind)> = vec![
            (
                "c",
                BlockKind::Constant {
                    value: vec![1.0, 2.0],
                },
            ),
            (
                "step",
                BlockKind::Step {
                    time: 1.0,
                    before: 0.0,
                    after: 2.5,
                },
            ),
            (
                "sine",
                BlockKind::Sine {
                    amplitude: 2.0,
                    frequency: 3.0,
                    phase: 0.5,
                    bias: 0.1,
                },
            ),
            (
                "sum",
                BlockKind::Sum {
                    signs: "+-+".into(),
                },
            ),
            ("gain", BlockKind::Gain { gain: -2.5 }),
            (
                "mg",
                BlockKind::MatrixGain {
                    matrix: vec![vec![1.0, 2.0], vec![3.0, 4.0]],
                },
            ),
            ("prod", BlockKind::Product { ops: "*/".into() }),
            (
                "sat",
                BlockKind::Saturation {
                    lower: -1.0,
                    upper: 2.0,
                },
            ),
            (
                "int",
                BlockKind::Integrator {
                    initial: vec![0.5],
                    lower: Some(-3.0),
                    upper: None,
                },
            ),
            (
                "tf",
                BlockKind::TransferFcn {
                    num: vec![1.0, 2.0],
                    den: vec![1.0, 3.0, 2.0],
                },
            ),
            (
                "zp",
                BlockKind::ZeroPole {
                    zeros: vec![[-1.0, 0.0]],
                    poles: vec![[-2.0, 1.0], [-2.0, -1.0]],
                    gain: 4.0,
                },
            ),
            (
                "ss",
                BlockKind::StateSpace {
                    a: vec![vec![0.0, 1.0], vec![-2.0, -3.0]],
                    b: vec![vec![0.0], vec![1.0]],
                    c: vec![vec![1.0, 0.0]],
                    d: vec![vec![0.0]],
                    x0: vec![1.0, 0.0],
                },
            ),
            (
                "ud",
                BlockKind::UnitDelay {
                    initial: vec![0.0],
                    sample_time: 0.1,
                },
            ),
            (
                "di",
                BlockKind::DiscreteIntegrator {
                    rule: DiscreteRule::Trapezoidal,
                    gain: 2.0,
                    initial: 1.0,
                    sample_time: 0.2,
                },
            ),
            ("rel", BlockKind::Relational { op: RelOp::Ge }),
            (
                "sw",
                BlockKind::Switch {
                    rule: SwitchRule::Gt,
                    threshold: 0.5,
                },
            ),
            (
                "lut",
                BlockKind::Lookup1D {
                    x: vec![0.0, 1.0, 2.0],
                    y: vec![0.0, 1.0, 4.0],
                },
            ),
            ("mux", BlockKind::Mux { inputs: 3 }),
            (
                "sel",
                BlockKind::Selector {
                    indices: vec![2, 3],
                },
            ),
            ("pw", BlockKind::Power { exponent: 3.0 }),
            (
                "sqrt",
                BlockKind::Math {
                    func: MathFunc::Sqrt,
                },
            ),
            (
                "tan",
                BlockKind::Math {
                    func: MathFunc::Tanh,
                },
            ),
            ("scope", BlockKind::Scope { inputs: 2 }),
            ("ws", BlockKind::ToWorkspace),
            ("stop", BlockKind::Stop),
        ];
        for (n, k) in kinds {
            m.add(n, k).unwrap();
        }
        let mut sub = Model::new("sub");
        sub.add("in", BlockKind::Inport { index: 1 }).unwrap();
        sub.add("g", BlockKind::Gain { gain: 3.0 }).unwrap();
        sub.add("out", BlockKind::Outport { index: 1 }).unwrap();
        sub.connect(("in", 0), ("g", 0));
        sub.connect(("g", 0), ("out", 0));
        m.add(
            "sub",
            BlockKind::Subsystem {
                model: Box::new(sub),
            },
        )
        .unwrap();
        m.connect(("step", 0), ("sum", 0));
        m.connect(("sine", 0), ("sum", 1));
        m.connect(("sum", 0), ("gain", 0));
        m.connect(("gain", 0), ("sub", 0));
        m.connect(("sub", 0), ("scope", 1));
        m.connect(("pw", 0), ("scope", 0));
        m
    }

    #[test]
    fn round_trips_through_mdl_text() {
        let m = sample();
        let text = write_mdl(&m).unwrap();
        let back = read(text.as_bytes(), &Workspace::new(), true).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        let (mut a, mut b) = (m, back.model);
        strip(&mut a);
        strip(&mut b);
        // The helper constant of the Power block folds back into the block.
        assert_eq!(a.blocks, b.blocks);
        assert_eq!(a.lines, b.lines);
        assert_eq!(a.solver, b.solver);
        assert_eq!(a.name, b.name);
    }

    #[test]
    fn round_trips_through_slx() {
        let m = sample();
        let bytes = write_slx(&m).unwrap();
        assert!(bytes.starts_with(b"PK"));
        let back = read(&bytes, &Workspace::new(), true).unwrap();
        let (mut a, mut b) = (m, back.model);
        strip(&mut a);
        strip(&mut b);
        assert_eq!(a.blocks, b.blocks);
        assert_eq!(a.lines, b.lines);
        assert_eq!(a.solver, b.solver);
    }

    const DECAY: &str = r#"
Model {
  Name "decay"
  Solver "ode45"
  StartTime "0.0"
  StopTime "2"
  RelTol "1e-6"
  System {
    Name "decay"
    Block { BlockType Integrator Name "I" InitialCondition "1" Position [100, 50, 130, 80] }
    Block { BlockType Gain Name "K" Gain "-a" }
    Block { BlockType Scope Name "Scope" NumInputPorts "1" }
    Line { SrcBlock "I" SrcPort 1 DstBlock "K" DstPort 1 }
    Line { SrcBlock "K" SrcPort 1 Points [0, 40; -60, 0] DstBlock "I" DstPort 1 }
    Line { SrcBlock "I" SrcPort 1 DstBlock "Scope" DstPort 1 }
  }
}
"#;

    #[test]
    fn an_imported_model_runs_and_uses_the_workspace() {
        let mut ws = Workspace::new();
        ws.insert("a".into(), vec![vec![1.0]]);
        let imp = read(DECAY.as_bytes(), &ws, true).unwrap();
        assert_eq!(imp.model.solver.method, Method::Dopri5);
        assert_eq!(imp.model.solver.stop_time, 2.0);
        assert_eq!(
            imp.model.block("I").and_then(|b| b.position),
            Some((100.0, 50.0))
        );
        let r = simulate(&imp.model).unwrap();
        let x = r.signal("Scope").unwrap().column(0);
        assert!((x[x.len() - 1] - (-2.0f64).exp()).abs() < 1e-4);
    }

    #[test]
    fn a_mat_file_supplies_the_variables() {
        let vars = vec![
            ("a".to_string(), MatValue::scalar(1.0)),
            ("t".to_string(), MatValue::text("skipped")),
        ];
        let ws = workspace_from_mat(&vars);
        assert_eq!(ws.get("a"), Some(&vec![vec![1.0]]));
        assert!(!ws.contains_key("t"));
        let imp = read(DECAY.as_bytes(), &ws, true).unwrap();
        assert!(imp.model.block("K").is_some());
    }

    #[test]
    fn a_missing_variable_is_an_error_that_names_it() {
        let e = read(DECAY.as_bytes(), &Workspace::new(), true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("Gain = -a"), "{e}");
        assert!(e.contains("a"), "{e}");
    }

    const UNSUPPORTED: &str = r#"
Model {
  Name "m"
  System {
    Name "m"
    Block { BlockType Constant Name "c" Value "2" }
    Block { BlockType PID Name "pid" }
    Block { BlockType Scope Name "s" }
    Line { SrcBlock "c" SrcPort 1 DstBlock "pid" DstPort 1 }
    Line { SrcBlock "pid" SrcPort 1 DstBlock "s" DstPort 1 }
    Line { SrcBlock "c" SrcPort 1 DstBlock "s" DstPort 1 }
  }
}
"#;

    #[test]
    fn strict_lists_unsupported_blocks_and_lenient_drops_them() {
        let e = read(UNSUPPORTED.as_bytes(), &Workspace::new(), true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("pid") && e.contains("PID"), "{e}");
        let imp = read(UNSUPPORTED.as_bytes(), &Workspace::new(), false).unwrap();
        assert!(imp.model.block("pid").is_none());
        assert_eq!(imp.model.lines.len(), 1);
        assert!(imp
            .warnings
            .iter()
            .any(|w| w.contains("dropped") && w.contains("pid")));
    }

    #[test]
    fn unexportable_blocks_are_errors() {
        let mut m = Model::new("m");
        m.add(
            "p",
            BlockKind::Pid {
                kp: 1.0,
                ki: 0.0,
                kd: 0.0,
                n: 100.0,
                initial: 0.0,
            },
        )
        .unwrap();
        let e = to_simulink(&m).unwrap_err().to_string();
        assert!(e.contains("\"p\"") && e.contains("PID"), "{e}");
    }

    #[test]
    fn solver_names_map_and_approximations_are_reported() {
        let mut sim = SimModel::default();
        sim.params.push(("Solver".into(), "ode15s".into()));
        sim.params.push(("StopTime".into(), "3".into()));
        let imp = from_simulink(&sim, &Workspace::new(), true).unwrap();
        assert_eq!(imp.model.solver.method, Method::Rosenbrock23);
        assert!(imp.warnings.iter().any(|w| w.contains("ode15s")));
    }

    #[test]
    fn inherited_sample_times_use_the_fixed_step() {
        let text = r#"Model { Name "d" Solver "ode4" FixedStep "0.05" System { Name "d"
            Block { BlockType UnitDelay Name "z" SampleTime "-1" } } }"#;
        let imp = read(text.as_bytes(), &Workspace::new(), true).unwrap();
        match imp.model.block("z").map(|b| &b.kind) {
            Some(BlockKind::UnitDelay { sample_time, .. }) => assert_eq!(*sample_time, 0.05),
            other => panic!("{other:?}"),
        }
        let text = text.replace(r#"FixedStep "0.05""#, r#"FixedStep "auto""#);
        assert!(read(text.as_bytes(), &Workspace::new(), true).is_err());
    }

    #[test]
    fn garbage_parameters_never_panic() {
        let junk = [
            "",
            "[",
            "]]",
            "1e999",
            "1:0",
            "[1 2; 3]",
            "zeros(1e9)",
            "((((",
            "a b",
            "-",
            "1e-",
        ];
        let types = [
            "Constant",
            "Gain",
            "Sum",
            "Product",
            "Integrator",
            "TransferFcn",
            "StateSpace",
            "Selector",
            "UnitDelay",
            "Lookup_n-D",
            "ZeroPole",
            "Scope",
            "Mux",
            "MinMax",
            "Relay",
            "Math",
        ];
        for t in types {
            for j in junk {
                let mut sys = SimSystem::default();
                let mut b = SimBlock {
                    block_type: t.into(),
                    name: "b".into(),
                    ..SimBlock::default()
                };
                for k in [
                    "Value",
                    "Gain",
                    "Inputs",
                    "InitialCondition",
                    "Numerator",
                    "Denominator",
                    "A",
                    "B",
                    "C",
                    "D",
                    "X0",
                    "Indices",
                    "SampleTime",
                    "BreakpointsForDimension1",
                    "Table",
                    "Zeros",
                    "Poles",
                    "Function",
                    "Operator",
                    "NumInputPorts",
                    "Position",
                ] {
                    b.params.push((k.into(), j.into()));
                }
                sys.blocks.push(b);
                let sim = SimModel {
                    system: sys,
                    ..SimModel::default()
                };
                let _ = from_simulink(&sim, &Workspace::new(), false);
                let _ = from_simulink(&sim, &Workspace::new(), true);
            }
        }
    }
}
