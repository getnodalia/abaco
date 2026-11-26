// SPDX-License-Identifier: MIT OR Apache-2.0
//! Why a model could not be built or run.

use thiserror::Error;

/// A model, compile or simulation failure; the message names the block and what to change.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum BlockError {
    /// The model file or its structure is wrong.
    #[error("{0}")]
    Model(String),
    /// A block cannot be built from its parameters or connections.
    #[error("block \"{block}\": {message}")]
    Block {
        /// Path of the block.
        block: String,
        /// What is wrong.
        message: String,
    },
    /// Blocks with direct feedthrough form a loop.
    #[error("algebraic loop through {0}; break it with a Unit Delay, an Integrator or a Transfer Fcn with more poles than zeros")]
    AlgebraicLoop(String),
    /// A signal became NaN or infinite.
    #[error("block \"{block}\" produced a non-finite value at t = {t:e}")]
    NonFinite {
        /// Path of the block.
        block: String,
        /// Simulation time.
        t: f64,
    },
    /// The solver could not continue.
    #[error("{0}")]
    Solver(String),
    /// The run was stopped by the caller.
    #[error("the simulation was stopped")]
    Cancelled,
}

impl BlockError {
    pub(crate) fn block(block: &str, message: impl Into<String>) -> BlockError {
        BlockError::Block {
            block: block.to_owned(),
            message: message.into(),
        }
    }
}

/// Shorthand for results of this crate.
pub type Result<T> = std::result::Result<T, BlockError>;
