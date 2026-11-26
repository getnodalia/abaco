// SPDX-License-Identifier: MIT OR Apache-2.0
//! The block-diagram model: blocks, lines, solver settings and the JSON file format.
//!
//! A model is data. Every block is named (unique inside its diagram), carries its parameters in
//! a [`BlockKind`] and is wired to others by [`Line`]s that name a block and a zero-based port.
//! The file is plain JSON, so it diffs, merges and is easy to write from a script.

use serde::{Deserialize, Serialize};

use crate::error::BlockError;

/// Largest model file read, in bytes.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
/// Deepest nesting of subsystems.
pub const MAX_DEPTH: usize = 16;
/// Most blocks in a compiled (flattened) model.
pub const MAX_BLOCKS: usize = 20_000;

fn one() -> f64 {
    1.0
}
fn one_vec() -> Vec<f64> {
    vec![1.0]
}
fn zero_vec() -> Vec<f64> {
    vec![0.0]
}
fn two() -> usize {
    2
}
fn one_usize() -> usize {
    1
}
fn hundred() -> f64 {
    100.0
}
fn half() -> f64 {
    0.5
}
fn fifty() -> f64 {
    50.0
}
fn tenth() -> f64 {
    0.1
}
fn ten() -> f64 {
    10.0
}
fn neg_one() -> f64 {
    -1.0
}
fn signs() -> String {
    "++".to_owned()
}
fn stars() -> String {
    "**".to_owned()
}
fn neg_half() -> f64 {
    -0.5
}

/// A number or a list of numbers in JSON; serialised as a bare number when it has one entry.
mod sv {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum One {
        Num(f64),
        List(Vec<f64>),
    }

    pub fn serialize<S: Serializer>(v: &[f64], s: S) -> Result<S::Ok, S::Error> {
        match v {
            [x] => x.serialize(s),
            _ => v.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
        Ok(match One::deserialize(d)? {
            One::Num(x) => vec![x],
            One::List(v) => v,
        })
    }
}

/// The shapes of the periodic signal generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveShape {
    /// `sin`.
    Sine,
    /// +1 for the first half period, −1 for the second.
    Square,
    /// Rises from −1 to 1 over a period, then jumps back.
    Sawtooth,
    /// Rises from −1 to 1 over half a period and falls back.
    Triangle,
}

/// One-input mathematical functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathFunc {
    /// `eˣ`.
    Exp,
    /// Natural logarithm.
    Log,
    /// Base-10 logarithm.
    Log10,
    /// Square root.
    Sqrt,
    /// `x²`.
    Square,
    /// `1/x`.
    Reciprocal,
    /// Sine.
    Sin,
    /// Cosine.
    Cos,
    /// Tangent.
    Tan,
    /// Inverse sine.
    Asin,
    /// Inverse cosine.
    Acos,
    /// Inverse tangent.
    Atan,
    /// Hyperbolic sine.
    Sinh,
    /// Hyperbolic cosine.
    Cosh,
    /// Hyperbolic tangent.
    Tanh,
    /// `10ˣ`.
    Pow10,
    /// Absolute value.
    Abs,
    /// −1, 0 or 1.
    Sign,
}

/// How the Rounding block rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundMode {
    /// Towards −∞.
    Floor,
    /// Towards +∞.
    Ceil,
    /// To nearest, halves away from zero.
    Round,
    /// Towards zero.
    Fix,
}

/// Minimum or maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extremum {
    /// The smallest input.
    Min,
    /// The largest input.
    Max,
}

/// Comparison operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelOp {
    /// `==`
    #[serde(rename = "==")]
    Eq,
    /// `~=`
    #[serde(rename = "~=")]
    Ne,
    /// `<`
    #[serde(rename = "<")]
    Lt,
    /// `<=`
    #[serde(rename = "<=")]
    Le,
    /// `>`
    #[serde(rename = ">")]
    Gt,
    /// `>=`
    #[serde(rename = ">=")]
    Ge,
}

/// Logical operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicOp {
    /// All inputs non-zero.
    And,
    /// Any input non-zero.
    Or,
    /// Exactly one input non-zero (odd count for more than two).
    Xor,
    /// `not and`.
    Nand,
    /// `not or`.
    Nor,
    /// Negation of the single input.
    Not,
}

/// When the Switch passes its first input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwitchRule {
    /// Control input ≥ threshold.
    #[serde(rename = ">=")]
    Ge,
    /// Control input > threshold.
    #[serde(rename = ">")]
    Gt,
    /// Control input ≠ 0.
    #[serde(rename = "~=0")]
    NonZero,
}

/// The discrete-time integration rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscreteRule {
    /// `y[n] = x[n]`, `x[n+1] = x[n] + K·Ts·u[n]`.
    ForwardEuler,
    /// `y[n] = x[n] + K·Ts·u[n]`.
    BackwardEuler,
    /// Trapezoidal rule.
    Trapezoidal,
}

/// Every kind of block and its parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BlockKind {
    // ───────────── sources ─────────────
    /// A constant (scalar or vector).
    Constant {
        /// The value.
        #[serde(with = "sv", default = "one_vec")]
        value: Vec<f64>,
    },
    /// Jumps from `before` to `after` at `time`.
    Step {
        /// Jump time.
        #[serde(default = "one")]
        time: f64,
        /// Value before.
        #[serde(default)]
        before: f64,
        /// Value from `time` on.
        #[serde(default = "one")]
        after: f64,
    },
    /// `initial` until `start`, then rises with `slope`.
    Ramp {
        /// Rate of rise.
        #[serde(default = "one")]
        slope: f64,
        /// Start time.
        #[serde(default)]
        start: f64,
        /// Value before and at `start`.
        #[serde(default)]
        initial: f64,
    },
    /// `amplitude·sin(frequency·t + phase) + bias`.
    Sine {
        /// Peak value.
        #[serde(default = "one")]
        amplitude: f64,
        /// Angular frequency in rad/s.
        #[serde(default = "one")]
        frequency: f64,
        /// Phase in radians.
        #[serde(default)]
        phase: f64,
        /// Offset.
        #[serde(default)]
        bias: f64,
    },
    /// A periodic pulse train.
    Pulse {
        /// Height of the pulse.
        #[serde(default = "one")]
        amplitude: f64,
        /// Period in seconds.
        #[serde(default = "one")]
        period: f64,
        /// Width as a percentage of the period.
        #[serde(default = "fifty")]
        duty: f64,
        /// Delay before the first period.
        #[serde(default)]
        delay: f64,
    },
    /// A sine, square, sawtooth or triangle wave.
    Wave {
        /// The shape.
        shape: WaveShape,
        /// Peak value.
        #[serde(default = "one")]
        amplitude: f64,
        /// Frequency in Hz.
        #[serde(default = "one")]
        frequency: f64,
    },
    /// A linear frequency sweep.
    Chirp {
        /// Peak value.
        #[serde(default = "one")]
        amplitude: f64,
        /// Start frequency in Hz.
        #[serde(default = "tenth")]
        f0: f64,
        /// Time at which `f1` is reached.
        #[serde(default = "ten")]
        target_time: f64,
        /// Frequency at `target_time`, in Hz.
        #[serde(default = "one")]
        f1: f64,
    },
    /// The simulation time.
    Clock,
    /// Always zero (ties off an unused input).
    Ground,
    /// Gaussian noise held for each sample time (seeded, so runs repeat).
    Noise {
        /// Mean.
        #[serde(default)]
        mean: f64,
        /// Variance.
        #[serde(default = "one")]
        variance: f64,
        /// Seed of the generator.
        #[serde(default = "one_usize")]
        seed: usize,
        /// Sample time.
        #[serde(default = "tenth")]
        sample_time: f64,
    },
    /// A table of `(time, value)` points, linearly interpolated or held.
    Table {
        /// Increasing times.
        times: Vec<f64>,
        /// Values at those times.
        values: Vec<f64>,
        /// Hold each value until the next time instead of interpolating.
        #[serde(default)]
        hold: bool,
    },
    /// An input of the diagram (or of a subsystem).
    Inport {
        /// One-based position.
        #[serde(default = "one_usize")]
        index: usize,
    },
    // ───────────── sinks ─────────────
    /// Records its inputs for plotting.
    Scope {
        /// Number of inputs.
        #[serde(default = "one_usize")]
        inputs: usize,
    },
    /// Records its input under the block's name.
    ToWorkspace,
    /// Shows the last value.
    Display,
    /// Swallows an unused output.
    Terminator,
    /// Stops the run when its input is non-zero.
    Stop,
    /// An output of the diagram (or of a subsystem).
    Outport {
        /// One-based position.
        #[serde(default = "one_usize")]
        index: usize,
    },
    // ───────────── math ─────────────
    /// Adds and subtracts: `signs` is one of `+` or `-` per input.
    Sum {
        /// For example `"+-"`.
        #[serde(default = "signs")]
        signs: String,
    },
    /// Multiplies the signal by a constant.
    Gain {
        /// The factor.
        #[serde(default = "one")]
        gain: f64,
    },
    /// Multiplies a vector by a matrix.
    MatrixGain {
        /// Row-major matrix.
        matrix: Vec<Vec<f64>>,
    },
    /// Multiplies and divides: `ops` is one of `*` or `/` per input.
    Product {
        /// For example `"*/"`.
        #[serde(default = "stars")]
        ops: String,
    },
    /// A one-input function.
    Math {
        /// Which one.
        func: MathFunc,
    },
    /// `uᵖ`.
    Power {
        /// The exponent.
        #[serde(default = "two_f")]
        exponent: f64,
    },
    /// `atan2(y, x)` of two inputs.
    Atan2,
    /// The remainder of the first input divided by the second.
    Mod,
    /// Clips the signal.
    Saturation {
        /// Lower limit.
        #[serde(default = "neg_one")]
        lower: f64,
        /// Upper limit.
        #[serde(default = "one")]
        upper: f64,
    },
    /// Zero inside `[start, end]`, shifted outside.
    DeadZone {
        /// Start of the dead zone.
        #[serde(default = "neg_half")]
        start: f64,
        /// End of the dead zone.
        #[serde(default = "half")]
        end: f64,
    },
    /// Adds a constant.
    Bias {
        /// The constant.
        #[serde(default)]
        bias: f64,
    },
    /// Rounds.
    Rounding {
        /// The rule.
        mode: RoundMode,
    },
    /// The smallest or largest of its inputs.
    MinMax {
        /// Which.
        mode: Extremum,
        /// Number of inputs.
        #[serde(default = "two")]
        inputs: usize,
    },
    /// Compares two inputs: 1 or 0.
    Relational {
        /// The operator.
        op: RelOp,
    },
    /// Compares the input with a constant: 1 or 0.
    CompareConst {
        /// The operator.
        op: RelOp,
        /// The constant.
        #[serde(default)]
        value: f64,
    },
    /// Boolean logic on non-zero values: 1 or 0.
    Logical {
        /// The operator.
        op: LogicOp,
        /// Number of inputs.
        #[serde(default = "two")]
        inputs: usize,
    },
    /// Passes the first or the third input depending on the second.
    Switch {
        /// The rule for the control input.
        rule: SwitchRule,
        /// The threshold.
        #[serde(default)]
        threshold: f64,
    },
    /// A relay with hysteresis.
    Relay {
        /// Switch on when the input rises to this.
        #[serde(default = "half")]
        on_switch: f64,
        /// Switch off when the input falls to this.
        #[serde(default = "neg_half")]
        off_switch: f64,
        /// Output while on.
        #[serde(default = "one")]
        on_output: f64,
        /// Output while off.
        #[serde(default)]
        off_output: f64,
    },
    /// Rounds to a multiple of `interval`.
    Quantizer {
        /// The step.
        #[serde(default = "tenth")]
        interval: f64,
    },
    /// A one-dimensional table, linearly interpolated and clipped at the ends.
    Lookup1D {
        /// Increasing inputs.
        x: Vec<f64>,
        /// Outputs.
        y: Vec<f64>,
    },
    /// The dot product of two vectors.
    DotProduct,
    /// The sum of the elements of a vector.
    SumElements,
    // ───────────── routing ─────────────
    /// Joins scalars or vectors into one vector.
    Mux {
        /// Number of inputs.
        #[serde(default = "two")]
        inputs: usize,
    },
    /// Splits a vector into equal parts.
    Demux {
        /// Number of outputs.
        #[serde(default = "two")]
        outputs: usize,
    },
    /// Picks elements of a vector, **one-based**.
    Selector {
        /// The elements.
        indices: Vec<usize>,
    },
    /// Sends its input to every `From` block with the same tag.
    Goto {
        /// The tag.
        tag: String,
    },
    /// Receives what the `Goto` with the same tag sent.
    From {
        /// The tag.
        tag: String,
    },
    /// A nested diagram, with `Inport` and `Outport` blocks as its connections.
    Subsystem {
        /// The diagram inside.
        model: Box<Model>,
    },
    // ───────────── continuous ─────────────
    /// `∫u dt`, optionally limited.
    Integrator {
        /// Initial value (scalar or vector).
        #[serde(with = "sv", default = "zero_vec")]
        initial: Vec<f64>,
        /// Lower limit of the state.
        #[serde(default)]
        lower: Option<f64>,
        /// Upper limit of the state.
        #[serde(default)]
        upper: Option<f64>,
    },
    /// A derivative filtered by a pole at `-n`: `n·s/(s+n)`.
    Derivative {
        /// Filter coefficient (rad/s).
        #[serde(default = "hundred")]
        n: f64,
    },
    /// A transfer function in `s`, coefficients highest power first.
    TransferFcn {
        /// Numerator.
        num: Vec<f64>,
        /// Denominator.
        den: Vec<f64>,
    },
    /// Zeros, poles and gain, each root as `[re, im]` (complex roots in conjugate pairs).
    ZeroPole {
        /// Zeros.
        #[serde(default)]
        zeros: Vec<[f64; 2]>,
        /// Poles.
        #[serde(default)]
        poles: Vec<[f64; 2]>,
        /// Gain.
        #[serde(default = "one")]
        gain: f64,
    },
    /// A continuous state-space model.
    StateSpace {
        /// `n × n`.
        a: Vec<Vec<f64>>,
        /// `n × m`.
        b: Vec<Vec<f64>>,
        /// `p × n`.
        c: Vec<Vec<f64>>,
        /// `p × m`.
        d: Vec<Vec<f64>>,
        /// Initial state (zero when empty).
        #[serde(default)]
        x0: Vec<f64>,
    },
    /// A parallel PID controller with a filtered derivative.
    Pid {
        /// Proportional gain.
        #[serde(default = "one")]
        kp: f64,
        /// Integral gain.
        #[serde(default)]
        ki: f64,
        /// Derivative gain.
        #[serde(default)]
        kd: f64,
        /// Derivative filter coefficient (rad/s).
        #[serde(default = "hundred")]
        n: f64,
        /// Initial value of the integrator.
        #[serde(default)]
        initial: f64,
    },
    /// A fixed delay.
    TransportDelay {
        /// The delay in seconds.
        #[serde(default = "one")]
        delay: f64,
        /// Output before the delay has elapsed.
        #[serde(default)]
        initial: f64,
    },
    // ───────────── discrete ─────────────
    /// One-sample delay.
    UnitDelay {
        /// Initial output.
        #[serde(with = "sv", default = "zero_vec")]
        initial: Vec<f64>,
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// An N-sample delay.
    Delay {
        /// Number of samples.
        #[serde(default = "one_usize")]
        length: usize,
        /// Initial output.
        #[serde(default)]
        initial: f64,
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// Samples and holds.
    ZeroOrderHold {
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// A discrete integrator.
    DiscreteIntegrator {
        /// The rule.
        rule: DiscreteRule,
        /// Gain `K`.
        #[serde(default = "one")]
        gain: f64,
        /// Initial state.
        #[serde(default)]
        initial: f64,
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// A transfer function in `z`, coefficients highest power first.
    DiscreteTransferFcn {
        /// Numerator.
        num: Vec<f64>,
        /// Denominator.
        den: Vec<f64>,
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// A discrete state-space model.
    DiscreteStateSpace {
        /// `n × n`.
        a: Vec<Vec<f64>>,
        /// `n × m`.
        b: Vec<Vec<f64>>,
        /// `p × n`.
        c: Vec<Vec<f64>>,
        /// `p × m`.
        d: Vec<Vec<f64>>,
        /// Initial state (zero when empty).
        #[serde(default)]
        x0: Vec<f64>,
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// `(u[n] − u[n−1]) / Ts`.
    DiscreteDerivative {
        /// Sample time.
        #[serde(default = "one")]
        sample_time: f64,
    },
    /// A pass-through, used inside the compiler for subsystem ports and `From` blocks.
    #[serde(skip)]
    Identity,
}

fn two_f() -> f64 {
    2.0
}

/// A block: a name, its parameters and where it sits in the editor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Unique inside its diagram.
    pub name: String,
    /// The kind and its parameters.
    #[serde(flatten)]
    pub kind: BlockKind,
    /// Position in the editor (grid units); not used by the solver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(f64, f64)>,
}

/// A wire from an output port to an input port; ports are zero-based.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// Source block and output port.
    pub from: (String, usize),
    /// Destination block and input port.
    pub to: (String, usize),
}

/// The integration methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Fixed step, first order (Euler).
    Euler,
    /// Fixed step, second order (Heun).
    Heun,
    /// Fixed step, third order (Bogacki–Shampine).
    Bs3,
    /// Fixed step, fourth order (classical Runge–Kutta).
    Rk4,
    /// Variable step, Dormand–Prince 5(4) (`ode45`).
    Dopri5,
    /// Variable step, Bogacki–Shampine 3(2) (`ode23`).
    Bs23,
    /// Variable step, Rosenbrock 2(3) for stiff problems (`ode23s`).
    Rosenbrock23,
}

impl Method {
    /// `true` for the fixed-step methods.
    pub fn is_fixed(self) -> bool {
        matches!(
            self,
            Method::Euler | Method::Heun | Method::Bs3 | Method::Rk4
        )
    }
}

fn default_method() -> Method {
    Method::Dopri5
}
fn default_stop() -> f64 {
    10.0
}
fn default_rtol() -> f64 {
    1e-3
}
fn default_atol() -> f64 {
    1e-6
}

/// How the model is integrated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolverConfig {
    /// The method.
    #[serde(default = "default_method")]
    pub method: Method,
    /// Start time.
    #[serde(default)]
    pub start_time: f64,
    /// Stop time.
    #[serde(default = "default_stop")]
    pub stop_time: f64,
    /// The step of a fixed-step method (default `(stop − start)/1000`); the first step of a
    /// variable-step one.
    #[serde(default)]
    pub step: Option<f64>,
    /// The largest step of a variable-step method.
    #[serde(default)]
    pub max_step: Option<f64>,
    /// Relative tolerance.
    #[serde(default = "default_rtol")]
    pub rtol: f64,
    /// Absolute tolerance.
    #[serde(default = "default_atol")]
    pub atol: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        SolverConfig {
            method: default_method(),
            start_time: 0.0,
            stop_time: default_stop(),
            step: None,
            max_step: None,
            rtol: default_rtol(),
            atol: default_atol(),
        }
    }
}

/// A block diagram.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// A title.
    #[serde(default)]
    pub name: String,
    /// Solver settings (ignored inside a subsystem).
    #[serde(default)]
    pub solver: SolverConfig,
    /// The blocks.
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// The wires.
    #[serde(default)]
    pub lines: Vec<Line>,
}

impl Model {
    /// An empty diagram.
    pub fn new(name: impl Into<String>) -> Model {
        Model {
            name: name.into(),
            ..Model::default()
        }
    }

    /// Adds a block; the name must not be taken.
    pub fn add(&mut self, name: impl Into<String>, kind: BlockKind) -> Result<(), BlockError> {
        let name = name.into();
        if name.is_empty() || name.contains('/') {
            return Err(BlockError::Model(format!(
                "\"{name}\" is not a valid block name (it is empty or contains \"/\")"
            )));
        }
        if self.block(&name).is_some() {
            return Err(BlockError::Model(format!(
                "there is already a block named \"{name}\""
            )));
        }
        self.blocks.push(Block {
            name,
            kind,
            position: None,
        });
        Ok(())
    }

    /// Wires `from` (output port, zero-based) to `to` (input port).
    pub fn connect(&mut self, from: (&str, usize), to: (&str, usize)) {
        self.lines.push(Line {
            from: (from.0.to_owned(), from.1),
            to: (to.0.to_owned(), to.1),
        });
    }

    /// The block with this name.
    pub fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.name == name)
    }

    /// Reads a model from JSON text.
    pub fn from_json(text: &str) -> Result<Model, BlockError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(BlockError::Model(format!(
                "the model file is larger than {} MB",
                MAX_FILE_BYTES / (1024 * 1024)
            )));
        }
        if !text.trim_start().starts_with('{') {
            return Err(BlockError::Model(
                "invalid model file: a model is a JSON object".into(),
            ));
        }
        serde_json::from_str(text).map_err(|e| {
            BlockError::Model(format!(
                "invalid model file: {e} (line {}, column {})",
                e.line(),
                e.column()
            ))
        })
    }

    /// The model as pretty JSON.
    pub fn to_json(&self) -> Result<String, BlockError> {
        serde_json::to_string_pretty(self).map_err(|e| BlockError::Model(e.to_string()))
    }
}

impl BlockKind {
    /// Number of input and output ports, as drawn in an editor; `None` when the parameters
    /// are invalid (for example a sign string with a character other than `+` or `-`).
    pub fn ports(&self) -> Option<(usize, usize)> {
        if let BlockKind::Subsystem { model } = self {
            let (mut i, mut o) = (0, 0);
            for b in &model.blocks {
                match b.kind {
                    BlockKind::Inport { .. } => i += 1,
                    BlockKind::Outport { .. } => o += 1,
                    _ => {}
                }
            }
            return Some((i, o));
        }
        crate::compile::port_counts("", self).ok()
    }

    /// A short human name for the kind, for messages.
    pub fn label(&self) -> String {
        let dbg = format!("{self:?}");
        let end = dbg.find([' ', '{', '(']).unwrap_or(dbg.len());
        dbg.get(..end).unwrap_or("Block").to_owned()
    }
}
