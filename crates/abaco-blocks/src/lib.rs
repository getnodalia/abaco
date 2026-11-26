// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![allow(clippy::needless_range_loop)]
//! Ábaco Blocks: block diagrams that run, in the spirit of Simulink, with solvers of their own.
//!
//! A [`Model`] is a set of named blocks wired together by lines. [`compile`] checks it (every
//! input wired, signal widths consistent, no algebraic loop), inlines subsystems and sorts the
//! blocks; [`Compiled::simulate`] then integrates it with a fixed-step (Euler, Heun, Bogacki–
//! Shampine 3, Runge–Kutta 4) or variable-step (Dormand–Prince 5(4), Bogacki–Shampine 3(2), the
//! stiff Rosenbrock 2(3)) method. Discrete blocks sample exactly on their sample times, sources
//! switch exactly on their switching times, and saturations, switches, relays and comparisons
//! are located to machine precision by zero-crossing detection.
//!
//! ```
//! use abaco_blocks::{simulate, BlockKind, Model};
//!
//! // x' = -x, x(0) = 1, so x(t) = e^-t.
//! let mut m = Model::new("decay");
//! m.solver.stop_time = 2.0;
//! m.add("int", BlockKind::Integrator { initial: vec![1.0], lower: None, upper: None }).unwrap();
//! m.add("neg", BlockKind::Gain { gain: -1.0 }).unwrap();
//! m.add("scope", BlockKind::Scope { inputs: 1 }).unwrap();
//! m.connect(("int", 0), ("neg", 0));
//! m.connect(("neg", 0), ("int", 0));
//! m.connect(("int", 0), ("scope", 0));
//! let r = simulate(&m).unwrap();
//! let x = r.signal("scope").unwrap().column(0);
//! assert!((x[x.len() - 1] - (-2.0f64).exp()).abs() < 1e-3);
//! ```

mod blocks;
pub mod compile;
pub mod error;
pub mod lti;
pub mod model;
pub mod sim;

pub use compile::{compile, Compiled};
pub use error::{BlockError, Result};
pub use lti::linearize;
pub use model::{
    Block, BlockKind, DiscreteRule, Extremum, Line, LogicOp, MathFunc, Method, Model, RelOp,
    RoundMode, SolverConfig, SwitchRule, WaveShape,
};
pub use sim::{simulate, Signal, SimOptions, SimResult, Stats};
