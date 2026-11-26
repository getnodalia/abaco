// SPDX-License-Identifier: MIT OR Apache-2.0
//! The static tables of prefixes and units.
//!
//! Every conversion factor is exact by definition (or conventional definition, noted
//! per row) and is expressed in coherent SI units.

use crate::dimension::{d, Dimension};
use std::f64::consts::PI;

/// Which prefixes a unit accepts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrefixKind {
    /// No prefix (`ft`, `psi`, `h`).
    None,
    /// SI decimal prefixes `yocto` .. `yotta`.
    Si,
    /// SI decimal and binary (`Ki`, `Mi`, ...) prefixes: `bit`, `B`.
    SiAndBinary,
}

/// An SI or binary prefix.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Prefix {
    /// Canonical symbol, e.g. `k`, `µ`, `Ki`.
    pub symbol: &'static str,
    /// Name, e.g. `kilo`.
    pub name: &'static str,
    /// Multiplier.
    pub factor: f64,
    /// `true` for the binary prefixes `Ki` .. `Ei`.
    pub binary: bool,
}

const fn p(symbol: &'static str, name: &'static str, factor: f64) -> Prefix {
    Prefix {
        symbol,
        name,
        factor,
        binary: false,
    }
}

const fn pb(symbol: &'static str, name: &'static str, factor: f64) -> Prefix {
    Prefix {
        symbol,
        name,
        factor,
        binary: true,
    }
}

/// All prefixes. Alternative spellings of micro (`u`, `μ` U+03BC) are accepted by
/// [`prefix_lookup`] but only `µ` (U+00B5) is listed here.
pub static PREFIXES: &[Prefix] = &[
    p("y", "yocto", 1e-24),
    p("z", "zepto", 1e-21),
    p("a", "atto", 1e-18),
    p("f", "femto", 1e-15),
    p("p", "pico", 1e-12),
    p("n", "nano", 1e-9),
    p("µ", "micro", 1e-6),
    p("m", "milli", 1e-3),
    p("c", "centi", 1e-2),
    p("d", "deci", 1e-1),
    p("da", "deca", 1e1),
    p("h", "hecto", 1e2),
    p("k", "kilo", 1e3),
    p("M", "mega", 1e6),
    p("G", "giga", 1e9),
    p("T", "tera", 1e12),
    p("P", "peta", 1e15),
    p("E", "exa", 1e18),
    p("Z", "zetta", 1e21),
    p("Y", "yotta", 1e24),
    pb("Ki", "kibi", 1024.0),
    pb("Mi", "mebi", 1_048_576.0),
    pb("Gi", "gibi", 1_073_741_824.0),
    pb("Ti", "tebi", 1_099_511_627_776.0),
    pb("Pi", "pebi", 1_125_899_906_842_624.0),
    pb("Ei", "exbi", 1_152_921_504_606_846_976.0),
];

/// Finds a prefix by symbol. Accepts `u` and `μ` (Greek mu) as spellings of `µ`.
pub fn prefix_lookup(symbol: &str) -> Option<&'static Prefix> {
    let symbol = match symbol {
        "u" | "μ" => "µ",
        s => s,
    };
    PREFIXES.iter().find(|p| p.symbol == symbol)
}

/// A unit of measure: a name, a dimension and the exact factor to coherent SI.
///
/// A value `x` in this unit is `(x + zero) * factor` in SI. `zero` is non-zero only for
/// the offset temperature scales `degC` and `degF`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UnitDef {
    /// Canonical symbol (`m`, `Ω`, `degC`).
    pub symbol: &'static str,
    /// English name.
    pub name: &'static str,
    /// Other accepted spellings.
    pub aliases: &'static [&'static str],
    /// Dimension.
    pub dim: Dimension,
    /// SI value of one unit (of one degree interval for offset scales).
    pub factor: f64,
    /// Offset in this unit's own scale; non-zero only for `degC`, `degF`.
    pub zero: f64,
    /// Accepted prefixes.
    pub prefix: PrefixKind,
    /// `true` for temperature-difference units (`delta_degC`).
    pub delta: bool,
}

impl UnitDef {
    /// `true` for offset scales (`degC`, `degF`), which cannot appear in compound units.
    pub fn is_affine(&self) -> bool {
        self.zero != 0.0
    }
}

const fn u(
    symbol: &'static str,
    name: &'static str,
    dim: Dimension,
    factor: f64,
    prefix: PrefixKind,
) -> UnitDef {
    UnitDef {
        symbol,
        name,
        aliases: &[],
        dim,
        factor,
        zero: 0.0,
        prefix,
        delta: false,
    }
}

const fn ua(
    symbol: &'static str,
    name: &'static str,
    aliases: &'static [&'static str],
    dim: Dimension,
    factor: f64,
    prefix: PrefixKind,
) -> UnitDef {
    UnitDef {
        symbol,
        name,
        aliases,
        dim,
        factor,
        zero: 0.0,
        prefix,
        delta: false,
    }
}

use PrefixKind::{None as NoPre, Si, SiAndBinary};

const NONE: Dimension = Dimension::NONE;
const LEN: Dimension = Dimension::LENGTH;
const AREA: Dimension = d(2, 0, 0, 0, 0, 0, 0);
const VOL: Dimension = d(3, 0, 0, 0, 0, 0, 0);
const MASS: Dimension = Dimension::MASS;
const TIME: Dimension = Dimension::TIME;
const FORCE: Dimension = d(1, 1, -2, 0, 0, 0, 0);
const PRESSURE: Dimension = d(-1, 1, -2, 0, 0, 0, 0);
const ENERGY: Dimension = d(2, 1, -2, 0, 0, 0, 0);
const POWER: Dimension = d(2, 1, -3, 0, 0, 0, 0);
const VELOCITY: Dimension = d(1, 0, -1, 0, 0, 0, 0);
const FREQ: Dimension = d(0, 0, -1, 0, 0, 0, 0);

// Exact definitions the tables below are built from.
const G0: f64 = 9.80665; // standard gravity, m/s^2 (CGPM 1901, exact)
const FT: f64 = 0.3048; // international foot (1959, exact)
const IN: f64 = 0.0254; // international inch (1959, exact)
const LB: f64 = 0.453_592_37; // avoirdupois pound (1959, exact)
const LBF: f64 = LB * G0; // pound-force, N
const HOUR: f64 = 3600.0;
const CAL: f64 = 4.184; // thermochemical calorie, J (exact by definition)
const EV: f64 = 1.602_176_634e-19; // electronvolt, J (exact since 2019)

/// All units, coherent-SI factors in the fourth column.
pub static UNITS: &[UnitDef] = &[
    // SI base units. The gram is the prefixable unit so that `kg` is `k` + `g`.
    u("m", "metre", LEN, 1.0, Si),
    ua("g", "gram", &["gram"], MASS, 1e-3, Si),
    u("s", "second", TIME, 1.0, Si),
    u("A", "ampere", Dimension::CURRENT, 1.0, Si),
    u("K", "kelvin", Dimension::TEMPERATURE, 1.0, Si),
    u("mol", "mole", Dimension::AMOUNT, 1.0, Si),
    u("cd", "candela", Dimension::LUMINOUS, 1.0, Si),
    // SI derived units.
    u("N", "newton", FORCE, 1.0, Si),
    u("Pa", "pascal", PRESSURE, 1.0, Si),
    u("J", "joule", ENERGY, 1.0, Si),
    u("W", "watt", POWER, 1.0, Si),
    u("C", "coulomb", d(0, 0, 1, 1, 0, 0, 0), 1.0, Si),
    u("V", "volt", d(2, 1, -3, -1, 0, 0, 0), 1.0, Si),
    u("F", "farad", d(-2, -1, 4, 2, 0, 0, 0), 1.0, Si),
    ua(
        "Ω",
        "ohm",
        &["ohm", "\u{2126}"],
        d(2, 1, -3, -2, 0, 0, 0),
        1.0,
        Si,
    ),
    u("S", "siemens", d(-2, -1, 3, 2, 0, 0, 0), 1.0, Si),
    u("Wb", "weber", d(2, 1, -2, -1, 0, 0, 0), 1.0, Si),
    u("T", "tesla", d(0, 1, -2, -1, 0, 0, 0), 1.0, Si),
    u("H", "henry", d(2, 1, -2, -2, 0, 0, 0), 1.0, Si),
    u("Hz", "hertz", FREQ, 1.0, Si),
    u("lm", "lumen", Dimension::LUMINOUS, 1.0, Si),
    u("lx", "lux", d(-2, 0, 0, 0, 0, 0, 1), 1.0, Si),
    u("Bq", "becquerel", FREQ, 1.0, Si),
    u("Gy", "gray", d(2, 0, -2, 0, 0, 0, 0), 1.0, Si),
    u("Sv", "sievert", d(2, 0, -2, 0, 0, 0, 0), 1.0, Si),
    u("kat", "katal", d(0, 0, -1, 0, 0, 1, 0), 1.0, Si),
    u("rad", "radian", NONE, 1.0, Si),
    u("sr", "steradian", NONE, 1.0, Si),
    // Time.
    ua("min", "minute", &["minute"], TIME, 60.0, NoPre),
    ua("h", "hour", &["hr"], TIME, HOUR, NoPre),
    ua("d", "day", &["day"], TIME, 86_400.0, NoPre),
    // Volume, mass, pressure, area.
    ua("L", "litre", &["l", "liter", "litre"], VOL, 1e-3, Si),
    ua("t", "tonne", &["tonne"], MASS, 1e3, Si),
    u("bar", "bar", PRESSURE, 1e5, Si),
    u("atm", "standard atmosphere", PRESSURE, 101_325.0, NoPre),
    u(
        "psi",
        "pound-force per square inch",
        PRESSURE,
        LBF / (IN * IN),
        NoPre,
    ),
    // Conventional mercury: 13595.1 kg/m^3 * g0 * 1 mm (exact by convention).
    u(
        "mmHg",
        "millimetre of mercury",
        PRESSURE,
        13_595.1 * G0 * 1e-3,
        NoPre,
    ),
    u("torr", "torr", PRESSURE, 101_325.0 / 760.0, NoPre),
    u("ha", "hectare", AREA, 1e4, NoPre),
    // Length.
    u("ft", "foot", LEN, FT, NoPre),
    u("in", "inch", LEN, IN, NoPre),
    u("yd", "yard", LEN, 3.0 * FT, NoPre),
    u("mi", "mile", LEN, 5280.0 * FT, NoPre),
    u("nmi", "nautical mile", LEN, 1852.0, NoPre),
    ua(
        "Å",
        "angstrom",
        &["angstrom", "\u{212B}"],
        LEN,
        1e-10,
        NoPre,
    ),
    // Mass and force.
    ua("lb", "pound", &["lbm"], MASS, LB, NoPre),
    u("oz", "ounce", MASS, LB / 16.0, NoPre),
    u("lbf", "pound-force", FORCE, LBF, NoPre),
    u("kgf", "kilogram-force", FORCE, G0, NoPre),
    u("dyn", "dyne", FORCE, 1e-5, Si),
    // Angle and ratios (all dimensionless).
    ua("deg", "degree", &["degree", "°"], NONE, PI / 180.0, NoPre),
    ua("arcmin", "arcminute", &["′"], NONE, PI / 10_800.0, NoPre),
    ua("arcsec", "arcsecond", &["″"], NONE, PI / 648_000.0, NoPre),
    u("rev", "revolution", NONE, 2.0 * PI, NoPre),
    u("ppm", "parts per million", NONE, 1e-6, NoPre),
    u("percent", "per cent", NONE, 1e-2, NoPre),
    // Rotation: 2*pi/60 rad/s, with the angle dimensionless (see README).
    u("rpm", "revolution per minute", FREQ, 2.0 * PI / 60.0, NoPre),
    // Energy and power.
    u("eV", "electronvolt", ENERGY, EV, Si),
    u("cal", "calorie (thermochemical)", ENERGY, CAL, Si),
    u(
        "BTU",
        "British thermal unit (IT)",
        ENERGY,
        1_055.055_852_62,
        NoPre,
    ),
    u("Wh", "watt-hour", ENERGY, HOUR, Si),
    u("erg", "erg", ENERGY, 1e-7, Si),
    // Mechanical horsepower: 550 ft·lbf/s.
    u(
        "hp",
        "horsepower (mechanical)",
        POWER,
        550.0 * FT * LBF,
        NoPre,
    ),
    // Speed.
    u("kn", "knot", VELOCITY, 1852.0 / HOUR, NoPre),
    u("knot", "knot", VELOCITY, 1852.0 / HOUR, NoPre),
    u("mph", "mile per hour", VELOCITY, 5280.0 * FT / HOUR, NoPre),
    // US liquid gallon: 231 cubic inches.
    u("gal", "US gallon", VOL, 231.0 * IN * IN * IN, NoPre),
    // Information.
    u("bit", "bit", NONE, 1.0, SiAndBinary),
    ua("B", "byte", &["byte"], NONE, 8.0, SiAndBinary),
    // Temperature. Offset scales are listed separately below.
    UnitDef {
        symbol: "degC",
        name: "degree Celsius",
        aliases: &["°C", "\u{2103}", "celsius"],
        dim: Dimension::TEMPERATURE,
        factor: 1.0,
        zero: 273.15,
        prefix: NoPre,
        delta: false,
    },
    UnitDef {
        symbol: "degF",
        name: "degree Fahrenheit",
        aliases: &["°F", "\u{2109}", "fahrenheit"],
        dim: Dimension::TEMPERATURE,
        factor: 5.0 / 9.0,
        zero: 459.67,
        prefix: NoPre,
        delta: false,
    },
    UnitDef {
        symbol: "degR",
        name: "degree Rankine",
        aliases: &["°R"],
        dim: Dimension::TEMPERATURE,
        factor: 5.0 / 9.0,
        zero: 0.0,
        prefix: NoPre,
        delta: false,
    },
    UnitDef {
        symbol: "delta_degC",
        name: "temperature difference in degrees Celsius",
        aliases: &["Δ°C", "ΔdegC"],
        dim: Dimension::TEMPERATURE,
        factor: 1.0,
        zero: 0.0,
        prefix: NoPre,
        delta: true,
    },
    UnitDef {
        symbol: "delta_degF",
        name: "temperature difference in degrees Fahrenheit",
        aliases: &["Δ°F", "ΔdegF"],
        dim: Dimension::TEMPERATURE,
        factor: 5.0 / 9.0,
        zero: 0.0,
        prefix: NoPre,
        delta: true,
    },
];

/// Finds a unit by exact symbol or alias (no prefix).
pub fn unit_lookup_exact(name: &str) -> Option<&'static UnitDef> {
    UNITS
        .iter()
        .find(|u| u.symbol == name || u.aliases.contains(&name))
}

/// Resolves a name to an optional prefix and a unit.
///
/// An exact unit symbol always wins over a prefix reading (`min` is the minute, not
/// milli-inch; `Pa` is the pascal). Otherwise the longest prefix whose remainder is a
/// unit that accepts it is used (`kg`, `mm`, `MiB`).
pub fn resolve(name: &str) -> Option<(Option<&'static Prefix>, &'static UnitDef)> {
    if let Some(def) = unit_lookup_exact(name) {
        return Some((None, def));
    }
    // Longest prefix first: two-character prefixes, then one-character ones.
    for take in [2usize, 1] {
        let mut chars = name.char_indices();
        let mut end = 0;
        let mut count = 0;
        for (i, c) in chars.by_ref() {
            count += 1;
            end = i + c.len_utf8();
            if count == take {
                break;
            }
        }
        if count < take || end >= name.len() {
            continue;
        }
        let (pre, rest) = name.split_at(end);
        if let (Some(prefix), Some(def)) = (prefix_lookup(pre), unit_lookup_exact(rest)) {
            let ok = match def.prefix {
                PrefixKind::None => false,
                PrefixKind::Si => !prefix.binary,
                PrefixKind::SiAndBinary => true,
            };
            if ok && !def.is_affine() {
                return Some((Some(prefix), def));
            }
        }
    }
    None
}

/// `true` when `name` is a unit symbol, alias or prefixed unit.
pub fn is_unit_name(name: &str) -> bool {
    resolve(name).is_some()
}

/// All known unit symbols and aliases, for suggestions.
pub(crate) fn all_names() -> impl Iterator<Item = &'static str> {
    UNITS
        .iter()
        .flat_map(|u| std::iter::once(u.symbol).chain(u.aliases.iter().copied()))
}
