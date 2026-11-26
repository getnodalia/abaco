// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::inconsistent_digit_grouping,
    clippy::excessive_precision
)]

use nodalia_units::constants::{self, CONSTANTS};
use nodalia_units::{format_sig, registry, Dimension, Quantity, Ratio, Unit, UnitError};
use proptest::prelude::*;

fn q(s: &str) -> Quantity {
    Quantity::parse(s).unwrap_or_else(|e| panic!("parse {s}: {e}"))
}

fn u(s: &str) -> Unit {
    Unit::parse(s).unwrap_or_else(|e| panic!("unit {s}: {e}"))
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / a.abs().max(b.abs()).max(1e-300)
}

fn si(s: &str) -> f64 {
    q(s).si_value()
}

#[track_caller]
fn close(got: f64, want: f64) {
    assert!(rel(got, want) < 1e-12, "got {got}, want {want}");
}

// ---------- exact definitions ----------

#[test]
fn exact_length_mass_time_definitions() {
    close(si("1 ft"), 0.3048);
    close(si("1 in"), 0.0254);
    close(si("1 yd"), 0.9144);
    close(si("1 mi"), 1609.344);
    close(si("1 nmi"), 1852.0);
    close(si("1 lb"), 0.453_592_37);
    close(si("1 oz"), 0.453_592_37 / 16.0);
    close(si("1 min"), 60.0);
    close(si("1 h"), 3600.0);
    close(si("1 d"), 86_400.0);
    close(si("1 t"), 1000.0);
    close(si("1 kg"), 1.0);
    close(si("1 g"), 1e-3);
    close(si("1 mg"), 1e-6);
}

#[test]
fn exact_derived_definitions() {
    close(si("1 psi"), 6894.757_293_168_361);
    // The value quoted in tables, to the digits they give.
    assert!((si("1 psi") - 6894.757_293_168).abs() < 1e-8);
    close(si("1 kWh"), 3.6e6);
    close(si("1 hp"), 745.699_871_582_270_22);
    close(si("1 atm"), 101_325.0);
    close(si("1 L"), 1e-3);
    close(si("1 l"), 1e-3);
    close(si("1 mL"), 1e-6);
    close(si("1 bar"), 1e5);
    close(si("1 mmHg"), 133.322_387_415);
    close(si("1 torr"), 101_325.0 / 760.0);
    close(si("1 kgf"), 9.80665);
    close(si("1 lbf"), 4.448_221_615_260_5);
    close(si("1 eV"), 1.602_176_634e-19);
    close(si("1 cal"), 4.184);
    close(si("1 kcal"), 4184.0);
    close(si("1 BTU"), 1055.055_852_62);
    close(si("1 gal"), 3.785_411_784e-3);
    close(si("1 kn"), 1852.0 / 3600.0);
    close(si("1 mph"), 0.447_04);
    close(si("1 deg"), std::f64::consts::PI / 180.0);
    close(si("1 arcmin"), std::f64::consts::PI / 10_800.0);
    close(si("1 arcsec"), std::f64::consts::PI / 648_000.0);
    close(si("1 rpm"), 2.0 * std::f64::consts::PI / 60.0);
}

#[test]
fn sixty_mph_in_metres_per_second() {
    let v = q("60 mph").to(&u("m/s")).unwrap();
    close(v.display_value(), 26.8224);
}

#[test]
fn prefixes() {
    close(si("1 km"), 1e3);
    close(si("1 µm"), 1e-6);
    close(si("1 um"), 1e-6);
    close(si("1 μm"), 1e-6);
    close(si("1 nm"), 1e-9);
    close(si("1 GHz"), 1e9);
    close(si("1 THz"), 1e12);
    close(si("1 ym"), 1e-24);
    close(si("1 Ym"), 1e24);
    close(si("1 dam"), 10.0);
    close(si("1 hPa"), 100.0);
    close(si("1 kPa"), 1000.0);
    close(si("1 MPa"), 1e6);
    close(si("1 KiB"), 1024.0 * 8.0);
    close(si("1 MiB"), 1_048_576.0 * 8.0);
    close(si("1 kbit"), 1000.0);
    close(si("1 Gibit"), 1_073_741_824.0);
    close(si("1 keV"), 1.602_176_634e-16);
}

#[test]
fn exact_symbol_beats_prefix_reading() {
    // `min` is the minute (not milli-inch), `mi` the mile (not milli-?), `cd` the candela.
    assert_eq!(u("min").dimension(), &Dimension::TIME);
    assert_eq!(u("mi").dimension(), &Dimension::LENGTH);
    assert_eq!(u("cd").dimension(), &Dimension::LUMINOUS);
    assert_eq!(u("ft").dimension(), &Dimension::LENGTH);
    // Units that take no prefix reject one.
    assert!(Unit::parse("kft").is_err());
    assert!(Unit::parse("Kim").is_err());
    // Binary prefixes only on bit and byte.
    assert!(Unit::parse("Kim").is_err());
    assert!(Unit::parse("Kim").is_err());
    assert!(Unit::parse("Kis").is_err());
}

#[test]
fn every_unit_has_a_consistent_definition() {
    for def in registry::UNITS {
        assert!(def.factor.is_finite() && def.factor > 0.0, "{}", def.symbol);
        let (p, d) = registry::resolve(def.symbol).unwrap();
        assert!(p.is_none(), "{} parses as prefixed", def.symbol);
        assert_eq!(d.symbol, def.symbol);
        for a in def.aliases {
            let (p, d) = registry::resolve(a).unwrap();
            assert!(p.is_none());
            assert_eq!(d.symbol, def.symbol);
        }
        Unit::parse(def.symbol).unwrap();
    }
    let mut names: Vec<&str> = registry::UNITS
        .iter()
        .flat_map(|u| std::iter::once(u.symbol).chain(u.aliases.iter().copied()))
        .collect();
    names.sort_unstable();
    let n = names.len();
    names.dedup();
    assert_eq!(n, names.len(), "duplicate unit symbol or alias");
}

#[test]
fn derived_units_have_the_right_dimensions() {
    let d = |s: &str| *u(s).dimension();
    assert_eq!(d("N"), d("kg*m/s^2"));
    assert_eq!(d("Pa"), d("N/m^2"));
    assert_eq!(d("J"), d("N*m"));
    assert_eq!(d("W"), d("J/s"));
    assert_eq!(d("C"), d("A*s"));
    assert_eq!(d("V"), d("W/A"));
    assert_eq!(d("F"), d("C/V"));
    assert_eq!(d("Ω"), d("V/A"));
    assert_eq!(d("ohm"), d("V/A"));
    assert_eq!(d("S"), d("1/ohm"));
    assert_eq!(d("Wb"), d("V*s"));
    assert_eq!(d("T"), d("Wb/m^2"));
    assert_eq!(d("H"), d("Wb/A"));
    assert_eq!(d("Hz"), d("1/s"));
    assert_eq!(d("lx"), d("lm/m^2"));
    assert_eq!(d("Gy"), d("J/kg"));
    assert_eq!(d("kat"), d("mol/s"));
    assert!(d("rad").is_dimensionless());
    assert!(d("sr").is_dimensionless());
}

// ---------- parsing and formatting ----------

#[test]
fn parse_unit_expressions() {
    close(u("m/s^2").scale(), 1.0);
    close(u("kg*m/s^2").scale(), 1.0);
    close(u("N·m").scale(), 1.0);
    close(u("km/h").scale(), 1000.0 / 3600.0);
    close(u("µm").scale(), 1e-6);
    close(u("mm^2").scale(), 1e-6);
    close(u("Ω").scale(), 1.0);
    close(u("1/s").scale(), 1.0);
    close(u("kg m/s^2").scale(), 1.0);
    close(u("m²").scale(), 1.0);
    close(u("s⁻¹").scale(), 1.0);
    close(u("W/(m*K)").scale(), 1.0);
    close(u("W/m/K").scale(), 1.0);
    close(u("m^(1/2)").scale(), 1.0);
    close(u("m^0.5").scale(), 1.0);
    close(u("cm^3").scale(), 1e-6);
    close(u("g/cm^3").scale(), 1000.0);
    assert_eq!(u("W/m/K").dimension(), u("W/(m*K)").dimension());
    assert_eq!(
        u("m^(1/2)").dimension(),
        &Dimension::LENGTH.pow(Ratio::new(1, 2).unwrap()).unwrap()
    );
}

#[test]
fn unit_display_round_trips() {
    for s in [
        "m/s^2",
        "kg*m/s^2",
        "km/h",
        "µm",
        "W/(m*K)",
        "1/s",
        "m^(1/2)",
        "mm^2",
        "N*m",
        "kg/(m*s^2)",
        "degC",
        "delta_degC/min",
    ] {
        let a = u(s);
        let again = u(&a.to_string());
        assert_eq!(a.dimension(), again.dimension(), "{s}");
        close(a.scale(), again.scale());
        assert_eq!(a.to_string(), again.to_string(), "{s}");
    }
    assert_eq!(u("kg m/s^2").to_string(), "kg*m/s^2");
    assert_eq!(u("1/s").to_string(), "1/s");
    assert_eq!(u("W/m/K").to_string(), "W/(m*K)");
    assert_eq!(u("m*m").to_string(), "m^2");
    assert_eq!(u("m/m").to_string(), "1");
}

#[test]
fn unit_parse_errors() {
    assert!(matches!(Unit::parse(""), Err(UnitError::Empty)));
    assert!(matches!(Unit::parse("   "), Err(UnitError::Empty)));
    assert!(matches!(Unit::parse("m/"), Err(UnitError::Syntax { .. })));
    assert!(matches!(Unit::parse("(m"), Err(UnitError::Syntax { .. })));
    assert!(matches!(Unit::parse("m)"), Err(UnitError::Syntax { .. })));
    assert!(matches!(Unit::parse("m^"), Err(UnitError::Syntax { .. })));
    assert!(matches!(Unit::parse("2 m"), Err(UnitError::Syntax { .. })));
    assert!(matches!(
        Unit::parse("m^(1/0)"),
        Err(UnitError::Syntax { .. })
    ));
    assert!(matches!(Unit::parse("m$"), Err(UnitError::Syntax { .. })));
    assert!(matches!(
        Unit::parse(&"m".repeat(300)),
        Err(UnitError::TooLong(_))
    ));
    assert!(matches!(
        Unit::parse("metr"),
        Err(UnitError::UnknownUnit { .. })
    ));
    match Unit::parse("mter") {
        Err(UnitError::UnknownUnit { suggestion, .. }) => {
            // whatever is suggested must itself be a unit
            assert!(suggestion.is_none_or(|s| Unit::parse(&s).is_ok()));
        }
        other => panic!("{other:?}"),
    }
    match Unit::parse("Kg") {
        Err(UnitError::UnknownUnit { suggestion, .. }) => {
            assert_eq!(suggestion.as_deref(), Some("kg"))
        }
        other => panic!("{other:?}"),
    }
    let e = Unit::parse("kilogramm").unwrap_err().to_string();
    assert!(e.contains("unknown unit `kilogramm`"), "{e}");
    let e = Unit::parse("foo").unwrap_err().to_string();
    assert!(e.contains("unknown unit `foo`"), "{e}");
}

#[test]
fn deep_nesting_is_refused_not_overflowed() {
    let s = format!("{}m{}", "(".repeat(5000), ")".repeat(5000));
    assert!(Unit::parse(&s).is_err());
    let s = format!("{}m{}", "(".repeat(50), ")".repeat(50));
    assert!(Unit::parse(&s).is_err());
    let s = format!("{}m{}", "(".repeat(10), ")".repeat(10));
    assert!(Unit::parse(&s).is_ok());
}

// ---------- dimension algebra ----------

#[test]
fn dimension_algebra() {
    let l = Dimension::LENGTH;
    let t = Dimension::TIME;
    let v = l.div(&t).unwrap();
    assert_eq!(v.name(), Some("velocity"));
    let a = v.div(&t).unwrap();
    assert_eq!(a.name(), Some("acceleration"));
    assert_eq!(a.mul(&Dimension::MASS).unwrap().name(), Some("force"));
    assert!(l.div(&l).unwrap().is_dimensionless());
    let half = Dimension::TIME.recip().unwrap().root(2).unwrap();
    assert_eq!(half.to_string(), "T^(-1/2)");
    assert_eq!(
        half.pow(Ratio::int(2)).unwrap(),
        Dimension::TIME.recip().unwrap()
    );
    assert_eq!(Dimension::NONE.to_string(), "1");
    assert_eq!(a.to_string(), "L T^-2");
    assert!(l.root(0).is_err());
}

#[test]
fn dimension_exponent_overflow_is_an_error() {
    let mut d = Dimension::LENGTH;
    let mut failed = false;
    for _ in 0..40 {
        match d.pow(Ratio::int(2)) {
            Ok(n) => d = n,
            Err(UnitError::Overflow(_)) => {
                failed = true;
                break;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(failed);
}

fn arb_ratio() -> impl Strategy<Value = Ratio> {
    (-6i64..=6, 1i64..=4).prop_map(|(n, d)| Ratio::new(n, d).unwrap())
}

fn arb_dim() -> impl Strategy<Value = Dimension> {
    prop::array::uniform7(arb_ratio()).prop_map(Dimension::from_ratios)
}

proptest! {
    #[test]
    fn mul_then_div_is_identity(a in arb_dim(), b in arb_dim()) {
        prop_assert_eq!(a.mul(&b).unwrap().div(&b).unwrap(), a);
        prop_assert_eq!(a.div(&b).unwrap().mul(&b).unwrap(), a);
        prop_assert_eq!(a.mul(&b).unwrap(), b.mul(&a).unwrap());
    }

    #[test]
    fn pow_and_root_are_inverse(a in arb_dim(), n in prop::sample::select(vec![1i16, 2, 3, 4, 5, -2, -3])) {
        let p = a.pow(Ratio::int(n)).unwrap();
        prop_assert_eq!(p.root(n).unwrap(), a);
        let r = a.root(n).unwrap();
        prop_assert_eq!(r.pow(Ratio::int(n)).unwrap(), a);
    }

    #[test]
    fn recip_twice_is_identity(a in arb_dim()) {
        prop_assert_eq!(a.recip().unwrap().recip().unwrap(), a);
        prop_assert!(a.div(&a).unwrap().is_dimensionless());
    }

    #[test]
    fn ratio_arithmetic_matches_f64(an in -50i64..50, ad in 1i64..20, bn in -50i64..50, bd in 1i64..20) {
        let a = Ratio::new(an, ad).unwrap();
        let b = Ratio::new(bn, bd).unwrap();
        let eps = 1e-12;
        prop_assert!((a.checked_add(b).unwrap().to_f64() - (a.to_f64() + b.to_f64())).abs() < eps);
        prop_assert!((a.checked_sub(b).unwrap().to_f64() - (a.to_f64() - b.to_f64())).abs() < eps);
        prop_assert!((a.checked_mul(b).unwrap().to_f64() - (a.to_f64() * b.to_f64())).abs() < eps);
        if bn != 0 {
            prop_assert!((a.checked_div(b).unwrap().to_f64() - (a.to_f64() / b.to_f64())).abs() < eps);
        }
    }

    #[test]
    fn quantity_mul_div_roundtrip(x in 1e-6f64..1e6, y in 1e-6f64..1e6) {
        let a = Quantity::new(x, &u("m"));
        let b = Quantity::new(y, &u("s"));
        let back = a.mul(&b).unwrap().div(&b).unwrap();
        prop_assert!(back.approx_eq(&a, 1e-12));
        prop_assert_eq!(back.dimension(), a.dimension());
    }

    #[test]
    fn conversion_round_trip(x in -1e6f64..1e6) {
        for unit in ["km", "ft", "mi", "in", "nmi"] {
            let a = Quantity::new(x, &u(unit));
            let m = a.to(&u("m")).unwrap();
            let back = m.value_in(&u(unit)).unwrap();
            prop_assert!((back - x).abs() <= 1e-12 * x.abs().max(1e-9));
        }
    }

    #[test]
    fn random_strings_never_panic(s in ".{0,80}") {
        let _ = Unit::parse(&s);
        let _ = Quantity::parse(&s);
        let _ = registry::resolve(&s);
        let _ = nodalia_units::suggest(&s);
    }

    #[test]
    fn random_unit_like_strings_never_panic(s in "[a-zA-Zµ°Ω1-9 ./*^()+\\-²³⁻]{0,60}") {
        if let Ok(unit) = Unit::parse(&s) {
            // anything that parses must print and re-parse to the same dimension
            let again = Unit::parse(&unit.to_string()).unwrap();
            prop_assert_eq!(unit.dimension(), again.dimension());
        }
        let _ = Quantity::parse(&format!("3 {s}"));
    }

    #[test]
    fn format_sig_never_panics(x in prop::num::f64::ANY, sig in 0usize..30) {
        let _ = format_sig(x, sig);
    }
}

// ---------- quantities ----------

#[test]
fn add_requires_equal_dimensions() {
    let e = q("5 m").add(&q("3 s")).unwrap_err().to_string();
    assert_eq!(e, "cannot add m and s (length and time)");
    let e = q("5 m").sub(&q("3 kg")).unwrap_err().to_string();
    assert_eq!(e, "cannot subtract m and kg (length and mass)");
    let e = q("5 m")
        .add(&Quantity::dimensionless(1.0))
        .unwrap_err()
        .to_string();
    assert_eq!(e, "cannot add m and a number (length and dimensionless)");
    let e = q("1 N").add(&q("1 J")).unwrap_err().to_string();
    assert!(e.contains("force and energy"), "{e}");
    let s = q("1 km").add(&q("500 m")).unwrap();
    assert_eq!(s.format(6), "1.5 km");
    let s = q("500 m").add(&q("1 km")).unwrap();
    assert_eq!(s.format(6), "1500 m");
}

#[test]
fn mul_div_units() {
    let f = q("10 kg").mul(&q("9.81 m/s^2")).unwrap();
    assert_eq!(f.dimension().name(), Some("force"));
    close(f.si_value(), 98.1);
    assert_eq!(f.format(6), "98.1 kg*m/s^2");
    assert_eq!(f.simplify().format(6), "98.1 N");
    assert_eq!(f.to(&u("N")).unwrap().format(6), "98.1 N");
    assert_eq!(f.to(&u("kN")).unwrap().format(6), "0.0981 kN");
    let d = q("60 km/h").mul(&q("2 h")).unwrap();
    assert_eq!(d.to(&u("km")).unwrap().format(6), "120 km");
    // units cancel to a plain number
    let r = q("1 m").div(&q("1 km")).unwrap();
    assert!(r.dimension().is_dimensionless());
    assert!(r.display_unit().is_none());
    close(r.si_value(), 1e-3);
    // scalar times quantity keeps the unit
    let s = Quantity::dimensionless(3.0).mul(&q("2 ft")).unwrap();
    assert_eq!(s.format(6), "6 ft");
    // division by zero
    assert!(q("1 m").div(&q("0 s")).is_err());
}

#[test]
fn simplify_picks_derived_units() {
    assert_eq!(q("2 kg*m^2/s^2").simplify().format(6), "2 J");
    assert_eq!(q("3 kg*m^2/s^3").simplify().format(6), "3 W");
    assert_eq!(q("3 kg/(m*s^2)").simplify().format(6), "3 Pa");
    assert_eq!(q("3 A*s").simplify().format(6), "3 C");
    assert_eq!(q("4 1/s").simplify().format(6), "4 Hz");
    assert_eq!(q("4 m/s").simplify().format(6), "4 m/s");
    assert_eq!(q("4 m*m").simplify().format(6), "4 m^2");
    let v = Quantity::from_si(2.0, Dimension::LENGTH.div(&Dimension::TIME).unwrap());
    assert_eq!(v.format(6), "2 m/s");
}

#[test]
fn pow_and_roots() {
    let a = q("4 m^2").sqrt().unwrap();
    assert_eq!(a.format(6), "2 m");
    let h = q("9 m").sqrt().unwrap();
    assert_eq!(h.dimension().to_string(), "L^(1/2)");
    close(h.si_value(), 3.0);
    assert_eq!(h.format(6), "3 m^(1/2)");
    let cube = q("2 m").pow(Ratio::int(3)).unwrap();
    assert_eq!(cube.format(6), "8 m^3");
    close(q("8 m^3").root(3).unwrap().si_value(), 2.0);
    assert!(q("-4 m").sqrt().is_err());
    assert!(
        q("0 m").pow(Ratio::int(-1)).is_err()
            || !q("0 m").pow(Ratio::int(-1)).unwrap().si_value().is_finite()
    );
    assert_eq!(
        q("2 m").pow_f64(0.5).unwrap().dimension().to_string(),
        "L^(1/2)"
    );
    assert!(q("2 m").pow_f64(0.1234567).is_err());
    close(
        Quantity::dimensionless(2.0)
            .pow_f64(0.1234567)
            .unwrap()
            .si_value(),
        2f64.powf(0.1234567),
    );
}

#[test]
fn conversion_errors_and_values() {
    let e = q("5 m").to(&u("s")).unwrap_err().to_string();
    assert_eq!(e, "cannot convert m to s (length and time)");
    let r = q("1 psi").to(&u("kPa")).unwrap();
    assert!((r.display_value() - 6.894_757_293_168).abs() < 1e-9);
    assert_eq!(q("1 atm").to(&u("Pa")).unwrap().format(10), "101325 Pa");
    assert_eq!(q("1 L").to(&u("m^3")).unwrap().format(6), "0.001 m^3");
    assert_eq!(
        q("100 km/h").to(&u("m/s")).unwrap().format(6),
        "27.7778 m/s"
    );
    assert_eq!(q("1 kWh").to(&u("J")).unwrap().format(6), "3600000 J");
    assert_eq!(q("1 kWh").to(&u("MJ")).unwrap().format(6), "3.6 MJ");
    assert_eq!(q("180 deg").to(&u("rad")).unwrap().format(6), "3.14159 rad");
    assert_eq!(q("1 mi").to(&u("km")).unwrap().format(7), "1.609344 km");
}

// ---------- temperatures ----------

#[test]
fn temperature_conversions() {
    let c = q("20 degC");
    close(c.si_value(), 293.15);
    assert!(c.is_temperature_point());
    let k = c.to(&u("K")).unwrap();
    assert_eq!(k.format(8), "293.15 K");
    let f = c.to(&u("degF")).unwrap();
    assert!(
        (f.display_value() - 68.0).abs() < 1e-9,
        "{}",
        f.display_value()
    );
    let back = f.to(&u("degC")).unwrap();
    assert!((back.display_value() - 20.0).abs() < 1e-9);
    assert!((q("32 degF").si_value() - 273.15).abs() < 1e-9);
    assert!((q("212 degF").to(&u("degC")).unwrap().display_value() - 100.0).abs() < 1e-9);
    assert!((q("-40 degC").to(&u("degF")).unwrap().display_value() + 40.0).abs() < 1e-9);
    assert!((q("0 K").to(&u("degC")).unwrap().display_value() + 273.15).abs() < 1e-9);
    assert!((q("300 K").to(&u("degC")).unwrap().display_value() - 26.85).abs() < 1e-9);
    assert!((q("491.67 degR").to(&u("K")).unwrap().si_value() - 273.15).abs() < 1e-9);
    assert!((q("°C").si_value() - 274.15).abs() < 1e-12);
    assert!((q("20 °C").si_value() - 293.15).abs() < 1e-12);
}

#[test]
fn temperature_arithmetic_rules() {
    // point + point is an error, point - point is a difference in K
    let e = q("20 degC").add(&q("10 degC")).unwrap_err();
    assert!(matches!(e, UnitError::Affine(_)), "{e}");
    let d = q("30 degC").sub(&q("20 degC")).unwrap();
    assert!(!d.is_temperature_point());
    close(d.si_value(), 10.0);
    assert_eq!(d.format(6), "10 K");
    assert_eq!(d.to(&u("delta_degF")).unwrap().format(6), "18 delta_degF");
    // point + difference stays a point in the same scale
    let p = q("20 degC").add(&q("5 K")).unwrap();
    assert_eq!(p.format(6), "25 degC");
    let p = q("5 delta_degC").add(&q("20 degC")).unwrap();
    assert_eq!(p.format(6), "25 degC");
    let p = q("68 degF").sub(&q("9 delta_degF")).unwrap();
    assert!((p.to(&u("degC")).unwrap().display_value() - 15.0).abs() < 1e-9);
    // products, quotients, powers, negation of a point are errors
    for r in [
        q("20 degC").mul(&q("2 m")),
        q("2 m").mul(&q("20 degC")),
        q("20 degC").div(&q("2 s")),
        q("2 s").div(&q("20 degC")),
        q("20 degC").pow(Ratio::int(2)),
        q("20 degC").neg(),
        q("20 degC").abs(),
    ] {
        assert!(matches!(r, Err(UnitError::Affine(_))), "{r:?}");
    }
    let e = q("20 degC")
        .mul(&Quantity::dimensionless(2.0))
        .unwrap_err()
        .to_string();
    assert!(e.contains("convert to K first"), "{e}");
    // K is an ordinary quantity
    assert_eq!(
        q("300 K")
            .mul(&Quantity::dimensionless(2.0))
            .unwrap()
            .format(6),
        "600 K"
    );
    assert_eq!(q("300 K").add(&q("20 K")).unwrap().format(6), "320 K");
    // plain K minus a point is a plain value
    let r = q("300 K").sub(&q("20 degC")).unwrap();
    close(r.si_value(), 6.85);
    assert!(!r.is_temperature_point());
    // comparison works on absolute values
    assert!(q("20 degC").approx_eq(&q("68 degF"), 1e-12));
    assert_eq!(
        q("20 degC").compare(&q("69 degF")).unwrap(),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        q("0 degC").compare(&q("0 K")).unwrap(),
        std::cmp::Ordering::Greater
    );
    // a point cannot be converted to a delta unit
    assert!(matches!(
        q("20 degC").to(&u("delta_degC")),
        Err(UnitError::Affine(_))
    ));
}

#[test]
fn offset_scales_cannot_be_in_compound_units() {
    for s in [
        "degC/s", "degC*m", "degC^2", "1/degC", "m/degF", "mdegC", "kdegC",
    ] {
        assert!(Unit::parse(s).is_err(), "{s} should be rejected");
    }
    let e = Unit::parse("degC/min").unwrap_err().to_string();
    assert!(e.contains("delta_degC") && e.contains("K"), "{e}");
    assert!(Unit::parse("K/min").is_ok());
    assert!(Unit::parse("delta_degC/min").is_ok());
    close(u("delta_degF/h").scale(), 5.0 / 9.0 / 3600.0);
    assert!(Unit::parse("degC").is_ok());
    assert!(Unit::parse("1*degC").is_ok());
}

// ---------- constants ----------

#[test]
fn constants_are_consistent() {
    for c in CONSTANTS {
        let qty = c.quantity().unwrap_or_else(|e| panic!("{}: {e}", c.name));
        assert!(
            qty.si_value().is_finite() && qty.si_value() > 0.0,
            "{}",
            c.name
        );
        assert!(constants::lookup(c.name).is_some());
        for a in c.aliases {
            assert_eq!(constants::lookup(a).unwrap().name, c.name);
        }
    }
    let mut names: Vec<&str> = CONSTANTS
        .iter()
        .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
        .collect();
    names.sort_unstable();
    let n = names.len();
    names.dedup();
    assert_eq!(n, names.len(), "duplicate constant name");
}

fn k(name: &str) -> Quantity {
    constants::lookup(name).unwrap().quantity().unwrap()
}

#[test]
fn constants_match_codata_2018() {
    close(k("c").si_value(), 299_792_458.0);
    close(k("h_planck").si_value(), 6.626_070_15e-34);
    close(k("q_e").si_value(), 1.602_176_634e-19);
    close(k("k_B").si_value(), 1.380_649e-23);
    close(k("N_A").si_value(), 6.022_140_76e23);
    close(k("g0").si_value(), 9.80665);
    close(k("G").si_value(), 6.674_30e-11);
    close(k("eps0").si_value(), 8.854_187_812_8e-12);
    close(k("mu0").si_value(), 1.256_637_062_12e-6);
    close(k("m_e").si_value(), 9.109_383_701_5e-31);
    close(k("m_p").si_value(), 1.672_621_923_69e-27);
    assert!((k("R").si_value() - 8.314_462_618_153_24).abs() < 1e-12);
    assert!((k("hbar").si_value() - 1.054_571_817_646_156e-34).abs() < 1e-46);
}

#[test]
fn derived_constants_follow_from_exact_ones() {
    use std::f64::consts::PI;
    let (h, c, kb) = (6.626_070_15e-34f64, 299_792_458.0f64, 1.380_649e-23f64);
    let sigma = 2.0 * PI.powi(5) * kb.powi(4) / (15.0 * h.powi(3) * c * c);
    assert!(rel(sigma, k("sigma_SB").si_value()) < 1e-12);
    // mu0 * eps0 * c^2 = 1 to within the stated 2018 uncertainties
    let one = k("mu0")
        .mul(&k("eps0"))
        .unwrap()
        .mul(&k("c").pow(Ratio::int(2)).unwrap())
        .unwrap();
    assert!((one.si_value() - 1.0).abs() < 1e-9, "{}", one.si_value());
    assert!(one.dimension().is_dimensionless());
    // dimensions
    assert_eq!(k("k_B").dimension().to_string(), "L^2 M T^-2 Th^-1");
    assert_eq!(k("G").dimension().to_string(), "L^3 M^-1 T^-2");
    assert_eq!(k("R").dimension().to_string(), "L^2 M T^-2 Th^-1 N^-1");
    // gas law sanity: p = nRT/V for 1 mol at 273.15 K in 22.414 L is ~1 atm
    let p = Quantity::parse("1 mol")
        .unwrap()
        .mul(&k("R"))
        .unwrap()
        .mul(&q("273.15 K"))
        .unwrap()
        .div(&q("22.414 L"))
        .unwrap();
    assert!((p.to(&u("atm")).unwrap().display_value() - 1.0).abs() < 1e-3);
}

// ---------- formatting ----------

#[test]
fn format_sig_cases() {
    assert_eq!(format_sig(0.0, 6), "0");
    assert_eq!(format_sig(1.0, 6), "1");
    assert_eq!(format_sig(-1.5, 6), "-1.5");
    assert_eq!(format_sig(123456.0, 6), "123456");
    assert_eq!(format_sig(1234567.0, 6), "1234570");
    assert_eq!(format_sig(0.0001, 6), "0.0001");
    assert_eq!(format_sig(0.00001, 6), "1e-5");
    assert_eq!(format_sig(1.2345678e-7, 4), "1.235e-7");
    assert_eq!(format_sig(1e20, 6), "1e20");
    assert_eq!(format_sig(f64::NAN, 6), "NaN");
    assert_eq!(format_sig(f64::INFINITY, 6), "inf");
    assert_eq!(format_sig(f64::NEG_INFINITY, 6), "-inf");
    assert_eq!(format_sig(9.999999, 3), "10");
    assert_eq!(format_sig(0.5, 0), "0.5");
}

#[test]
fn quantity_parse_forms() {
    assert_eq!(q("9.81 m/s^2").format(6), "9.81 m/s^2");
    assert_eq!(q("1e3 Pa").format(6), "1000 Pa");
    assert_eq!(q("1_000 m").format(6), "1000 m");
    assert_eq!(q("-5 km").format(6), "-5 km");
    assert_eq!(q(".5 m").format(6), "0.5 m");
    assert_eq!(q("kg").format(6), "1 kg");
    assert_eq!(q("3").format(6), "3");
    assert_eq!(q("5Em").format(6), "5 Em");
    assert_eq!(q("5e3m").format(6), "5000 m");
    assert!(Quantity::parse("").is_err());
    assert!(Quantity::parse("5 furlong").is_err());
    assert!(Quantity::parse("1e999 m").is_err());
}

#[test]
fn non_finite_results_are_errors() {
    let big = Quantity::new(1e300, &u("m"));
    assert!(big.mul(&big).is_err());
    assert!(big.add(&big).is_ok());
    assert!(Quantity::dimensionless(10.0).pow_f64(1000.0).is_err());
}
