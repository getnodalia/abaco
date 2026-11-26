// SPDX-License-Identifier: MIT OR Apache-2.0
//! The blocks an editor offers: each with a category, a name and sensible default parameters.

use crate::model::BlockKind;

/// One entry of the block library.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteEntry {
    /// The group it is listed under (`Sources`, `Sinks`, `Math`, `Routing`, `Continuous`,
    /// `Discrete`).
    pub category: &'static str,
    /// The name of the block, also the file's `type`.
    pub name: &'static str,
    /// A new block with its default parameters.
    pub kind: BlockKind,
}

/// The categories in the order an editor lists them.
pub const CATEGORIES: [&str; 6] = [
    "Sources",
    "Sinks",
    "Math",
    "Routing",
    "Continuous",
    "Discrete",
];

/// Block type name, category and the JSON of a new block (the `type` is added from the name).
const ENTRIES: &[(&str, &str, &str)] = &[
    ("Sources", "Constant", r#"{"value": [1.0]}"#),
    (
        "Sources",
        "Step",
        r#"{"time": 1.0, "before": 0.0, "after": 1.0}"#,
    ),
    (
        "Sources",
        "Ramp",
        r#"{"slope": 1.0, "start": 0.0, "initial": 0.0}"#,
    ),
    (
        "Sources",
        "Sine",
        r#"{"amplitude": 1.0, "frequency": 1.0, "phase": 0.0, "bias": 0.0}"#,
    ),
    (
        "Sources",
        "Pulse",
        r#"{"amplitude": 1.0, "period": 1.0, "duty": 0.5, "delay": 0.0}"#,
    ),
    (
        "Sources",
        "Wave",
        r#"{"shape": "square", "amplitude": 1.0, "frequency": 1.0}"#,
    ),
    (
        "Sources",
        "Chirp",
        r#"{"amplitude": 1.0, "f0": 0.1, "target_time": 10.0, "f1": 2.0}"#,
    ),
    ("Sources", "Clock", "{}"),
    ("Sources", "Ground", "{}"),
    (
        "Sources",
        "Noise",
        r#"{"mean": 0.0, "variance": 1.0, "seed": 1, "sample_time": 0.1}"#,
    ),
    (
        "Sources",
        "Table",
        r#"{"times": [0.0, 1.0, 2.0], "values": [0.0, 1.0, 0.0], "hold": false}"#,
    ),
    ("Sources", "Inport", r#"{"index": 1}"#),
    ("Sinks", "Scope", r#"{"inputs": 1}"#),
    ("Sinks", "ToWorkspace", "{}"),
    ("Sinks", "Display", "{}"),
    ("Sinks", "Terminator", "{}"),
    ("Sinks", "Stop", "{}"),
    ("Sinks", "Outport", r#"{"index": 1}"#),
    ("Math", "Sum", r#"{"signs": "+-"}"#),
    ("Math", "Gain", r#"{"gain": 1.0}"#),
    ("Math", "MatrixGain", r#"{"matrix": [[1.0]]}"#),
    ("Math", "Product", r#"{"ops": "**"}"#),
    ("Math", "Math", r#"{"func": "abs"}"#),
    ("Math", "Power", r#"{"exponent": 2.0}"#),
    ("Math", "Atan2", "{}"),
    ("Math", "Mod", "{}"),
    ("Math", "Saturation", r#"{"lower": -1.0, "upper": 1.0}"#),
    ("Math", "DeadZone", r#"{"start": -0.5, "end": 0.5}"#),
    ("Math", "Bias", r#"{"bias": 1.0}"#),
    ("Math", "Rounding", r#"{"mode": "round"}"#),
    ("Math", "MinMax", r#"{"mode": "min", "inputs": 2}"#),
    ("Math", "Relational", r#"{"op": ">"}"#),
    ("Math", "CompareConst", r#"{"op": ">", "value": 0.0}"#),
    ("Math", "Logical", r#"{"op": "and", "inputs": 2}"#),
    ("Math", "Switch", r#"{"rule": ">=", "threshold": 0.0}"#),
    (
        "Math",
        "Relay",
        r#"{"on_switch": 0.5, "off_switch": -0.5, "on_output": 1.0, "off_output": 0.0}"#,
    ),
    ("Math", "Quantizer", r#"{"interval": 0.5}"#),
    (
        "Math",
        "Lookup1D",
        r#"{"x": [0.0, 1.0, 2.0], "y": [0.0, 1.0, 4.0]}"#,
    ),
    ("Math", "DotProduct", "{}"),
    ("Math", "SumElements", "{}"),
    ("Routing", "Mux", r#"{"inputs": 2}"#),
    ("Routing", "Demux", r#"{"outputs": 2}"#),
    ("Routing", "Selector", r#"{"indices": [1]}"#),
    ("Routing", "Goto", r#"{"tag": "A"}"#),
    ("Routing", "From", r#"{"tag": "A"}"#),
    (
        "Routing",
        "Subsystem",
        r#"{"model": {"name": "Subsystem", "blocks": [{"name": "In1", "type": "Inport", "index": 1}, {"name": "Out1", "type": "Outport", "index": 1}], "lines": [{"from": ["In1", 0], "to": ["Out1", 0]}]}}"#,
    ),
    ("Continuous", "Integrator", r#"{"initial": [0.0]}"#),
    ("Continuous", "Derivative", r#"{"n": 100.0}"#),
    (
        "Continuous",
        "TransferFcn",
        r#"{"num": [1.0], "den": [1.0, 1.0]}"#,
    ),
    (
        "Continuous",
        "ZeroPole",
        r#"{"zeros": [], "poles": [[-1.0, 0.0]], "gain": 1.0}"#,
    ),
    (
        "Continuous",
        "StateSpace",
        r#"{"a": [[-1.0]], "b": [[1.0]], "c": [[1.0]], "d": [[0.0]], "x0": [0.0]}"#,
    ),
    (
        "Continuous",
        "Pid",
        r#"{"kp": 1.0, "ki": 0.0, "kd": 0.0, "n": 100.0, "initial": 0.0}"#,
    ),
    (
        "Continuous",
        "TransportDelay",
        r#"{"delay": 1.0, "initial": 0.0}"#,
    ),
    (
        "Discrete",
        "UnitDelay",
        r#"{"initial": [0.0], "sample_time": 1.0}"#,
    ),
    (
        "Discrete",
        "Delay",
        r#"{"length": 2, "initial": 0.0, "sample_time": 1.0}"#,
    ),
    ("Discrete", "ZeroOrderHold", r#"{"sample_time": 1.0}"#),
    (
        "Discrete",
        "DiscreteIntegrator",
        r#"{"rule": "forward_euler", "gain": 1.0, "initial": 0.0, "sample_time": 1.0}"#,
    ),
    (
        "Discrete",
        "DiscreteTransferFcn",
        r#"{"num": [1.0], "den": [1.0, -0.5], "sample_time": 1.0}"#,
    ),
    (
        "Discrete",
        "DiscreteStateSpace",
        r#"{"a": [[0.5]], "b": [[1.0]], "c": [[1.0]], "d": [[0.0]], "x0": [0.0], "sample_time": 1.0}"#,
    ),
    ("Discrete", "DiscreteDerivative", r#"{"sample_time": 1.0}"#),
];

/// Every block an editor offers, in listing order.
pub fn palette() -> Vec<PaletteEntry> {
    ENTRIES
        .iter()
        .filter_map(|(category, name, params)| {
            let mut v: serde_json::Value = serde_json::from_str(params).ok()?;
            v.as_object_mut()?
                .insert("type".into(), serde_json::Value::String((*name).into()));
            let kind = serde_json::from_value(v).ok()?;
            Some(PaletteEntry {
                category,
                name,
                kind,
            })
        })
        .collect()
}

impl BlockKind {
    /// The kind's name as in a model file's `type`.
    pub fn type_name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str().map(str::to_owned)))
            .unwrap_or_else(|| self.label())
    }
}
