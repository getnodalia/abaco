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
//! Physical dimensions, units, quantities and CODATA constants for Nodalia apps.
//!
//! * [`Dimension`]: exponents of the seven SI base dimensions as exact rationals.
//! * [`Unit`]: a parsed unit expression (`kg*m/s^2`, `km/h`, `µm`) with its SI scale.
//! * [`Quantity`]: a value with a dimension, stored in SI, with checked arithmetic.
//! * [`constants`]: CODATA 2018 constants as quantities.
//! * [`registry`]: the unit and prefix tables.
//!
//! ```
//! use nodalia_units::{Quantity, Unit};
//! let force = Quantity::parse("10 kg")?.mul(&Quantity::parse("9.81 m/s^2")?)?;
//! let n = force.to(&Unit::parse("N")?)?;
//! assert_eq!(n.format(6), "98.1 N");
//! # Ok::<(), nodalia_units::UnitError>(())
//! ```

pub mod constants;
mod dimension;
mod error;
mod fmt;
mod quantity;
mod ratio;
pub mod registry;
mod unit;

pub use dimension::{Dimension, BASE_SYMBOLS};
pub use error::UnitError;
pub use fmt::format_sig;
pub use quantity::Quantity;
pub use ratio::Ratio;
pub use registry::{is_unit_name, PrefixKind, UnitDef};
pub use unit::{suggest, Unit, UnitFactor, MAX_UNIT_LEN};
