// SPDX-License-Identifier: MIT OR Apache-2.0
//! Physical constants, CODATA 2018 recommended values.
//!
//! Source: E. Tiesinga, P. J. Mohr, D. B. Newell and B. N. Taylor, "CODATA recommended
//! values of the fundamental physical constants: 2018", Rev. Mod. Phys. 93, 025010
//! (2021); NIST SP 961. Since the 2019 redefinition of the SI, `c`, `h`, `e`, `k_B` and
//! `N_A` are exact, and so are the constants derived from them only (`hbar`, `R`,
//! `sigma_SB`).

use crate::error::UnitError;
use crate::quantity::Quantity;
use crate::unit::Unit;
use std::f64::consts::PI;

/// A named physical constant.
#[derive(Clone, Copy, Debug)]
pub struct Constant {
    /// The name used in expressions (`c`, `k_B`).
    pub name: &'static str,
    /// Other spellings (`ħ` for `hbar`).
    pub aliases: &'static [&'static str],
    /// What it is.
    pub description: &'static str,
    /// Value in `unit`.
    pub value: f64,
    /// Unit expression of the value.
    pub unit: &'static str,
    /// Relative standard uncertainty (0 for exact constants).
    pub rel_uncertainty: f64,
}

impl Constant {
    /// The constant as a quantity, displayed in its natural unit.
    pub fn quantity(&self) -> Result<Quantity, UnitError> {
        Ok(Quantity::new(self.value, &Unit::parse(self.unit)?))
    }
}

const H: f64 = 6.626_070_15e-34;
const KB: f64 = 1.380_649e-23;
const NA: f64 = 6.022_140_76e23;

/// Every constant, CODATA 2018.
pub static CONSTANTS: &[Constant] = &[
    Constant {
        name: "c",
        aliases: &[],
        description: "speed of light in vacuum",
        value: 299_792_458.0,
        unit: "m/s",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "h_planck",
        aliases: &["h_P"],
        description: "Planck constant",
        value: H,
        unit: "J*s",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "hbar",
        aliases: &["ħ"],
        description: "reduced Planck constant h/(2 pi)",
        value: H / (2.0 * PI),
        unit: "J*s",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "q_e",
        aliases: &["e_charge"],
        description: "elementary charge",
        value: 1.602_176_634e-19,
        unit: "C",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "k_B",
        aliases: &["kB"],
        description: "Boltzmann constant",
        value: KB,
        unit: "J/K",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "N_A",
        aliases: &["NA"],
        description: "Avogadro constant",
        value: NA,
        unit: "1/mol",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "G",
        aliases: &["G_N"],
        description: "Newtonian constant of gravitation",
        value: 6.674_30e-11,
        unit: "m^3/(kg*s^2)",
        rel_uncertainty: 2.2e-5,
    },
    Constant {
        name: "g0",
        aliases: &["g_n"],
        description: "standard acceleration of gravity",
        value: 9.806_65,
        unit: "m/s^2",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "R",
        aliases: &["R_gas"],
        description: "molar gas constant N_A k_B",
        value: NA * KB,
        unit: "J/(mol*K)",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "eps0",
        aliases: &["ε0"],
        description: "vacuum electric permittivity",
        value: 8.854_187_812_8e-12,
        unit: "F/m",
        rel_uncertainty: 1.5e-10,
    },
    Constant {
        name: "mu0",
        aliases: &["μ0", "µ0"],
        description: "vacuum magnetic permeability (CODATA 2018 value)",
        value: 1.256_637_062_12e-6,
        unit: "N/A^2",
        rel_uncertainty: 1.5e-10,
    },
    Constant {
        name: "sigma_SB",
        aliases: &["σ"],
        description: "Stefan-Boltzmann constant 2 pi^5 k^4 / (15 h^3 c^2)",
        value: 5.670_374_419_184_429e-8,
        unit: "W/(m^2*K^4)",
        rel_uncertainty: 0.0,
    },
    Constant {
        name: "m_e",
        aliases: &[],
        description: "electron mass",
        value: 9.109_383_701_5e-31,
        unit: "kg",
        rel_uncertainty: 3.0e-10,
    },
    Constant {
        name: "m_p",
        aliases: &[],
        description: "proton mass",
        value: 1.672_621_923_69e-27,
        unit: "kg",
        rel_uncertainty: 3.1e-10,
    },
    Constant {
        name: "m_n",
        aliases: &[],
        description: "neutron mass",
        value: 1.674_927_498_04e-27,
        unit: "kg",
        rel_uncertainty: 5.7e-10,
    },
    Constant {
        name: "alpha",
        aliases: &["α"],
        description: "fine-structure constant",
        value: 7.297_352_569_3e-3,
        unit: "1",
        rel_uncertainty: 1.5e-10,
    },
    Constant {
        name: "atm_std",
        aliases: &[],
        description: "standard atmosphere",
        value: 101_325.0,
        unit: "Pa",
        rel_uncertainty: 0.0,
    },
];

/// Finds a constant by name or alias.
pub fn lookup(name: &str) -> Option<&'static Constant> {
    CONSTANTS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}
