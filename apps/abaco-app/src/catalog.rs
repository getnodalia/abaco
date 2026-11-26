// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the units panel lists and the conversion it offers: every unit the engine knows grouped
//! by what it measures, the physical constants with their values, and "5 km in other units".

use nodalia_units::registry::UNITS;
use nodalia_units::{constants, Quantity, Unit};

/// Most conversions the panel lists for one input.
pub const MAX_CONVERSIONS: usize = 40;
/// Longest text the converter reads.
pub const MAX_INPUT: usize = 200;

/// One unit of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitRow {
    pub symbol: &'static str,
    /// English name; the window translates it.
    pub name: &'static str,
    /// English name of the dimension (`length`, `force`...), or its formula when it has none.
    pub category: String,
}

/// One physical constant.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstantRow {
    /// What an expression calls it (`c`, `k_B`).
    pub name: &'static str,
    /// English description; the window translates it.
    pub description: &'static str,
    /// Its value with its unit at `digits` significant digits.
    pub value: String,
    /// Exact by definition of the SI.
    pub exact: bool,
}

/// Every unit, grouped by category in the order the categories first appear.
pub fn units() -> Vec<UnitRow> {
    let mut rows: Vec<UnitRow> = UNITS
        .iter()
        .filter(|u| !u.delta)
        .map(|u| UnitRow {
            symbol: u.symbol,
            name: u.name,
            category: u.dim.describe(),
        })
        .collect();
    // A stable sort keeps the registry's order inside a category and groups the categories.
    let mut order: Vec<String> = Vec::new();
    for r in &rows {
        if !order.contains(&r.category) {
            order.push(r.category.clone());
        }
    }
    rows.sort_by_key(|r| order.iter().position(|c| *c == r.category));
    rows
}

/// The distinct categories, in listing order.
pub fn categories() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in units() {
        if !out.contains(&r.category) {
            out.push(r.category);
        }
    }
    out
}

/// Every constant with its value at `digits` significant digits.
pub fn constants_table(digits: usize) -> Vec<ConstantRow> {
    constants::CONSTANTS
        .iter()
        .map(|c| ConstantRow {
            name: c.name,
            description: c.description,
            value: c
                .quantity()
                .map(|q| q.format(digits))
                .unwrap_or_else(|_| format!("{} {}", c.value, c.unit)),
            exact: c.rel_uncertainty == 0.0,
        })
        .collect()
}

/// Whether `text` matches the query: every word of the query is part of the text, ignoring case.
pub fn matches(query: &str, texts: &[&str]) -> bool {
    let hay = texts.join(" ").to_lowercase();
    query
        .split_whitespace()
        .all(|word| hay.contains(&word.to_lowercase()))
}

/// What the converter could not do, in words a person can act on (the engine's, in English).
pub type ConvertError = String;

/// A quantity written as `5 km` (a number and a unit), or an error.
pub fn parse_quantity(input: &str) -> Result<Quantity, ConvertError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(String::new());
    }
    if input.len() > MAX_INPUT {
        return Err("too long".to_owned());
    }
    Quantity::parse(input).map_err(|e| e.to_string())
}

/// `input` in the unit named `to`, for example `5 km` to `mi` is `3.10686 mi`.
pub fn convert(input: &str, to: &str, digits: usize) -> Result<String, ConvertError> {
    let q = parse_quantity(input)?;
    let unit = Unit::parse(to.trim()).map_err(|e| e.to_string())?;
    let out = q.to(&unit).map_err(|e| e.to_string())?;
    Ok(out.format(digits))
}

/// `input` in every unit of the same dimension, as `(symbol, text)` pairs.
pub fn convert_all(
    input: &str,
    digits: usize,
) -> Result<Vec<(&'static str, String)>, ConvertError> {
    let q = parse_quantity(input)?;
    let mut out = Vec::new();
    for def in UNITS.iter().filter(|u| !u.delta && u.dim == *q.dimension()) {
        let Ok(unit) = Unit::parse(def.symbol) else {
            continue;
        };
        if let Ok(v) = q.to(&unit) {
            out.push((def.symbol, v.format(digits)));
        }
        if out.len() >= MAX_CONVERSIONS {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_grouped_by_category() {
        let rows = units();
        assert!(rows.len() > 60);
        let mut seen: Vec<&str> = Vec::new();
        for r in &rows {
            if seen.last() != Some(&r.category.as_str()) {
                assert!(
                    !seen.contains(&r.category.as_str()),
                    "{} appears in two places",
                    r.category
                );
                seen.push(&r.category);
            }
        }
        assert!(rows
            .iter()
            .any(|r| r.symbol == "N" && r.category == "force"));
        assert!(rows.iter().all(|r| !r.symbol.starts_with("delta_")));
    }

    #[test]
    fn constants_carry_their_values() {
        let table = constants_table(6);
        let c = table.iter().find(|c| c.name == "c").unwrap();
        assert_eq!(c.value, "2.99792e8 m/s");
        assert!(c.exact);
        let g = table.iter().find(|c| c.name == "g0").unwrap();
        assert!(g.value.starts_with("9.80665"), "{}", g.value);
        assert!(table.iter().any(|c| !c.exact));
    }

    #[test]
    fn conversions_follow_the_units_engine() {
        assert_eq!(convert("1 mi", "km", 6).unwrap(), "1.60934 km");
        assert_eq!(convert("100 degC", "degF", 6).unwrap(), "212 degF");
        assert_eq!(convert("10 kg", "lb", 4).unwrap(), "22.05 lb");
    }

    #[test]
    fn a_mismatch_or_nonsense_is_an_error_not_a_panic() {
        assert!(convert("5 m", "s", 6).unwrap_err().contains("length"));
        assert!(convert("5 qqq", "m", 6).is_err());
        assert!(convert("5 m", "qqq", 6).is_err());
        assert_eq!(convert("", "m", 6).unwrap_err(), "");
        assert!(convert(&"9".repeat(500), "m", 6).is_err());
        assert!(convert_all("\u{0}", 6).is_err());
    }

    #[test]
    fn convert_all_lists_the_units_of_the_same_dimension() {
        let all = convert_all("1 m", 6).unwrap();
        assert!(all.iter().any(|(s, t)| *s == "ft" && t == "3.28084 ft"));
        assert!(all.iter().all(|(s, _)| *s != "s"));
        assert!(all.len() <= MAX_CONVERSIONS);
        // A bare number has the dimension of angles and ratios.
        assert!(convert_all("2", 6).is_ok());
    }

    #[test]
    fn filter_matches_every_word_ignoring_case() {
        assert!(matches("NEWTON force", &["N", "newton", "force"]));
        assert!(!matches("newton volt", &["N", "newton", "force"]));
        assert!(matches("", &["x"]));
    }
}
