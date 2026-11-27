// SPDX-License-Identifier: MIT OR Apache-2.0
//! The structure shared by Simulink's two file formats: a model is a tree of systems holding
//! blocks (a type, a name and string parameters) and lines from one block port to another.
//! [`crate::mdl`] and [`crate::slx`] read and write it; nothing here knows what a block does.
//!
//! Ports are numbered from 1, as Simulink numbers them, and blocks are referred to by name.

/// The deepest nesting of subsystems accepted.
pub const MAX_DEPTH: usize = 64;
/// The most blocks one model may hold.
pub const MAX_BLOCKS: usize = 200_000;

/// A block.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SimBlock {
    /// `Gain`, `Sum`, `SubSystem`, …
    pub block_type: String,
    /// The name, unique inside its system.
    pub name: String,
    /// Parameters in file order. Values are the text Simulink stores (`"2"`, `"[1 2 3]"`, `"on"`).
    pub params: Vec<(String, String)>,
    /// The diagram inside a subsystem.
    pub system: Option<SimSystem>,
}

impl SimBlock {
    /// A parameter's value.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Sets a parameter, replacing an earlier value.
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self.params.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = value,
            None => self.params.push((name.to_string(), value)),
        }
    }
}

/// A line from an output port to an input port. A branching line in a file is one `SimLine`
/// per destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimLine {
    /// Source block and output port (from 1).
    pub src: (String, usize),
    /// Destination block and input port (from 1).
    pub dst: (String, usize),
}

/// A diagram.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SimSystem {
    /// The system's name.
    pub name: String,
    /// The blocks.
    pub blocks: Vec<SimBlock>,
    /// The lines.
    pub lines: Vec<SimLine>,
}

/// A whole model.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SimModel {
    /// The model's name.
    pub name: String,
    /// Model-level settings (`StartTime`, `StopTime`, `SolverName`, `FixedStep`, `RelTol`, …).
    pub params: Vec<(String, String)>,
    /// The top-level diagram.
    pub system: SimSystem,
}

impl SimModel {
    /// A model-level setting.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

impl SimSystem {
    /// The number of blocks in this system and every system inside it.
    pub fn count_blocks(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| 1 + b.system.as_ref().map_or(0, SimSystem::count_blocks))
            .sum()
    }
}

/// Reads a Simulink model of either format: an `.slx` archive or `.mdl` text, told apart by
/// the first bytes.
pub fn read(bytes: &[u8]) -> crate::error::Result<SimModel> {
    if bytes.starts_with(b"PK") {
        crate::slx::read(bytes)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(t) => crate::mdl::read(t),
            Err(_) => {
                crate::error::corrupt("the model file is neither a ZIP archive nor UTF-8 text")
            }
        }
    }
}
