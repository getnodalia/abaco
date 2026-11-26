// SPDX-License-Identifier: MIT OR Apache-2.0
//! Compound units and the unit-expression parser.

use crate::dimension::Dimension;
use crate::error::UnitError;
use crate::ratio::Ratio;
use crate::registry::{self, Prefix, UnitDef};
use std::fmt;

/// Longest accepted unit expression, in bytes.
pub const MAX_UNIT_LEN: usize = 256;
const MAX_DEPTH: usize = 16;
const MAX_FACTORS: usize = 32;

/// One factor of a compound unit: an optionally prefixed unit raised to a power.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UnitFactor {
    /// Prefix, if any.
    pub prefix: Option<&'static Prefix>,
    /// The unit.
    pub def: &'static UnitDef,
    /// Exponent.
    pub exp: Ratio,
}

impl UnitFactor {
    fn same_base(&self, o: &UnitFactor) -> bool {
        self.def.symbol == o.def.symbol
            && self.prefix.map(|p| p.symbol) == o.prefix.map(|p| p.symbol)
    }

    fn symbol(&self) -> String {
        match self.prefix {
            Some(p) => format!("{}{}", p.symbol, self.def.symbol),
            None => self.def.symbol.to_string(),
        }
    }

    fn scale(&self) -> f64 {
        let base = self.prefix.map_or(1.0, |p| p.factor) * self.def.factor;
        if self.exp.is_integer() {
            base.powi(i32::from(self.exp.numer()))
        } else {
            base.powf(self.exp.to_f64())
        }
    }
}

/// A unit: a product of prefixed unit symbols with rational exponents, such as
/// `kg*m/s^2`, `km/h` or `degC`.
///
/// Offset temperature scales (`degC`, `degF`) are only allowed alone, never in a
/// product, quotient or power; use `K` or the `delta_` units for rates.
#[derive(Clone, PartialEq, Debug)]
pub struct Unit {
    factors: Vec<UnitFactor>,
    dim: Dimension,
    scale: f64,
    zero: f64,
}

impl Unit {
    /// The empty unit: the dimensionless number 1.
    pub fn one() -> Unit {
        Unit {
            factors: Vec::new(),
            dim: Dimension::NONE,
            scale: 1.0,
            zero: 0.0,
        }
    }

    /// Parses a unit expression: `m/s^2`, `kg*m/s^2`, `N·m`, `km/h`, `µm`, `um`, `Ω`,
    /// `ohm`, `mm^2`, `1/s`, `W/(m*K)`, `s^-1`, `m^(1/2)`, `m²`, `kg m/s^2`.
    ///
    /// `*`, `·`, `⋅`, `×` and whitespace multiply; `/` divides left to right; `^`
    /// binds tighter and takes an integer, decimal or parenthesised fraction.
    pub fn parse(src: &str) -> Result<Unit, UnitError> {
        if src.len() > MAX_UNIT_LEN {
            return Err(UnitError::TooLong(MAX_UNIT_LEN));
        }
        if src.trim().is_empty() {
            return Err(UnitError::Empty);
        }
        let toks = lex(src)?;
        let mut p = Parser {
            toks: &toks,
            i: 0,
            len: src.len(),
        };
        let u = p.product(0)?;
        if let Some(t) = p.toks.get(p.i) {
            return Err(UnitError::syntax(
                t.pos,
                format!("unexpected {}", t.kind.describe()),
            ));
        }
        Ok(u)
    }

    /// Builds a unit from one resolved name such as `kg` or `degC`.
    pub fn from_name(name: &str) -> Result<Unit, UnitError> {
        let (prefix, def) = registry::resolve(name).ok_or_else(|| unknown(name))?;
        Ok(Unit::single(prefix, def, Ratio::ONE))
    }

    fn single(prefix: Option<&'static Prefix>, def: &'static UnitDef, exp: Ratio) -> Unit {
        let f = UnitFactor { prefix, def, exp };
        let zero = if def.is_affine() { def.zero } else { 0.0 };
        Unit {
            dim: def.dim.pow(exp).unwrap_or(Dimension::NONE),
            scale: f.scale(),
            zero,
            factors: vec![f],
        }
    }

    /// The coherent SI unit for a dimension, such as `kg*m/s^2` for force.
    pub fn coherent_si(dim: &Dimension) -> Unit {
        const SYMS: [&str; 7] = ["m", "kg", "s", "A", "K", "mol", "cd"];
        let mut factors = Vec::new();
        for (sym, e) in SYMS.iter().zip(dim.exponents().iter()) {
            if e.is_zero() {
                continue;
            }
            if let Some((prefix, def)) = registry::resolve(sym) {
                factors.push(UnitFactor {
                    prefix,
                    def,
                    exp: *e,
                });
            }
        }
        Unit {
            factors,
            dim: *dim,
            scale: 1.0,
            zero: 0.0,
        }
    }

    /// The dimension of the unit.
    pub fn dimension(&self) -> &Dimension {
        &self.dim
    }

    /// SI value of one of this unit (interval size for offset scales).
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Offset in the unit's own scale (`273.15` for `degC`), `0` otherwise.
    pub fn zero(&self) -> f64 {
        self.zero
    }

    /// `true` for `degC` and `degF` alone.
    pub fn is_affine(&self) -> bool {
        self.zero != 0.0
    }

    /// `true` when this is exactly a temperature-difference unit (`delta_degC`).
    pub fn is_delta(&self) -> bool {
        matches!(self.factors.as_slice(), [f] if f.def.delta && f.exp == Ratio::ONE)
    }

    /// `true` for the empty unit.
    pub fn is_one(&self) -> bool {
        self.factors.is_empty()
    }

    /// The factors in order of appearance.
    pub fn factors(&self) -> &[UnitFactor] {
        &self.factors
    }

    fn check_not_affine(&self, op: &str) -> Result<(), UnitError> {
        if self.is_affine() {
            return Err(UnitError::Affine(format!(
                "{} is an offset temperature scale and cannot be used in a {op}; \
                 use K, or delta_{} for temperature differences",
                self,
                self.factors.first().map_or("", |f| f.def.symbol)
            )));
        }
        Ok(())
    }

    /// Product of two units.
    pub fn mul(&self, o: &Unit) -> Result<Unit, UnitError> {
        if self.is_one() {
            return Ok(o.clone());
        }
        if o.is_one() {
            return Ok(self.clone());
        }
        self.check_not_affine("product")?;
        o.check_not_affine("product")?;
        self.combine(o, false)
    }

    /// Quotient of two units.
    pub fn div(&self, o: &Unit) -> Result<Unit, UnitError> {
        if o.is_one() {
            return Ok(self.clone());
        }
        self.check_not_affine("quotient")?;
        o.check_not_affine("quotient")?;
        self.combine(o, true)
    }

    fn combine(&self, o: &Unit, invert: bool) -> Result<Unit, UnitError> {
        let mut factors = self.factors.clone();
        for f in &o.factors {
            let e = if invert {
                f.exp
                    .checked_neg()
                    .ok_or(UnitError::Overflow("unit exponent"))?
            } else {
                f.exp
            };
            match factors.iter_mut().find(|g| g.same_base(f)) {
                Some(g) => {
                    g.exp = g
                        .exp
                        .checked_add(e)
                        .ok_or(UnitError::Overflow("unit exponent"))?
                }
                None => factors.push(UnitFactor { exp: e, ..*f }),
            }
        }
        factors.retain(|f| !f.exp.is_zero());
        if factors.len() > MAX_FACTORS {
            return Err(UnitError::Domain(format!(
                "a unit may have at most {MAX_FACTORS} factors"
            )));
        }
        let dim = if invert {
            self.dim.div(&o.dim)?
        } else {
            self.dim.mul(&o.dim)?
        };
        let scale = factors.iter().map(UnitFactor::scale).product();
        Ok(Unit {
            factors,
            dim,
            scale,
            zero: 0.0,
        })
    }

    /// Raises the unit to a rational power.
    pub fn pow(&self, e: Ratio) -> Result<Unit, UnitError> {
        if e == Ratio::ONE || self.is_one() {
            return Ok(self.clone());
        }
        self.check_not_affine("power")?;
        let mut factors = Vec::with_capacity(self.factors.len());
        for f in &self.factors {
            let exp = f
                .exp
                .checked_mul(e)
                .ok_or(UnitError::Overflow("unit exponent"))?;
            if !exp.is_zero() {
                factors.push(UnitFactor { exp, ..*f });
            }
        }
        let scale = factors.iter().map(UnitFactor::scale).product();
        Ok(Unit {
            factors,
            dim: self.dim.pow(e)?,
            scale,
            zero: 0.0,
        })
    }

    /// The reciprocal unit.
    pub fn recip(&self) -> Result<Unit, UnitError> {
        Unit::one().div(self)
    }
}

impl fmt::Display for Unit {
    /// ASCII form: `kg*m/s^2`, `W/(m*K)`, `1/s`; the empty unit prints as `1`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.factors.is_empty() {
            return write!(f, "1");
        }
        let term = |u: &UnitFactor, e: Ratio| -> String {
            if e == Ratio::ONE {
                u.symbol()
            } else if e.is_integer() {
                format!("{}^{}", u.symbol(), e)
            } else {
                format!("{}^({})", u.symbol(), e)
            }
        };
        let num: Vec<String> = self
            .factors
            .iter()
            .filter(|u| u.exp > Ratio::ZERO)
            .map(|u| term(u, u.exp))
            .collect();
        let den: Vec<String> = self
            .factors
            .iter()
            .filter(|u| u.exp < Ratio::ZERO)
            .map(|u| term(u, u.exp.checked_neg().unwrap_or(Ratio::ONE)))
            .collect();
        let n = if num.is_empty() {
            "1".to_string()
        } else {
            num.join("*")
        };
        match den.len() {
            0 => write!(f, "{n}"),
            1 => write!(f, "{n}/{}", den[0]),
            _ => write!(f, "{n}/({})", den.join("*")),
        }
    }
}

fn unknown(name: &str) -> UnitError {
    UnitError::UnknownUnit {
        name: name.to_string(),
        suggestion: suggest(name),
    }
}

/// The closest known unit name to `name`, if one is near enough to be a likely typo.
pub fn suggest(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mut upper_first = lower.clone();
    if let Some(c) = upper_first.get(..1) {
        upper_first = format!("{}{}", c.to_uppercase(), &lower[1..]);
    }
    for cand in [lower.clone(), upper_first, name.to_uppercase()] {
        if cand != name && registry::is_unit_name(&cand) {
            return Some(cand);
        }
    }
    let limit = if name.chars().count() <= 3 { 1 } else { 2 };
    let mut best: Option<(usize, &str)> = None;
    for cand in registry::all_names() {
        let dist = edit_distance(name, cand, limit);
        if dist <= limit && cand != name && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, cand));
        }
    }
    best.map(|(_, s)| s.to_string())
}

/// Damerau-free Levenshtein distance, early-exiting above `cap`.
pub(crate) fn edit_distance(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > cap {
        return cap + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            cur.push(sub.min(del).min(ins));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(cap + 1)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(String),
    Star,
    Slash,
    Caret,
    Minus,
    Plus,
    LParen,
    RParen,
    Sup(i32),
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Num(s) => format!("number `{s}`"),
            Tok::Star => "`*`".into(),
            Tok::Slash => "`/`".into(),
            Tok::Caret => "`^`".into(),
            Tok::Minus => "`-`".into(),
            Tok::Plus => "`+`".into(),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::Sup(_) => "superscript".into(),
        }
    }
}

struct Spanned {
    kind: Tok,
    pos: usize,
}

fn sup_digit(c: char) -> Option<i32> {
    "⁰¹²³⁴⁵⁶⁷⁸⁹".chars().position(|d| d == c).map(|i| i as i32)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '°' | '′' | '″' | '℃' | '℉' | 'Ω' | 'Å' | '_')
}

fn is_ident_part(c: char) -> bool {
    (c.is_alphabetic() || c == '_') && sup_digit(c).is_none()
}

fn lex(src: &str) -> Result<Vec<Spanned>, UnitError> {
    let mut out = Vec::new();
    let mut it = src.char_indices().peekable();
    while let Some((pos, c)) = it.next() {
        let kind = match c {
            c if c.is_whitespace() => continue,
            '*' | '·' | '⋅' | '×' => Tok::Star,
            '/' => Tok::Slash,
            '^' => Tok::Caret,
            '-' | '−' => Tok::Minus,
            '+' => Tok::Plus,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '0'..='9' | '.' => {
                let mut s = String::from(c);
                while let Some(&(_, n)) = it.peek() {
                    if n.is_ascii_digit() || n == '.' {
                        s.push(n);
                        it.next();
                    } else {
                        break;
                    }
                }
                Tok::Num(s)
            }
            c if c == '⁻' || sup_digit(c).is_some() => {
                let neg = c == '⁻';
                let mut v: i32 = sup_digit(c).unwrap_or(0);
                let mut digits = usize::from(!neg);
                while let Some(&(_, n)) = it.peek() {
                    if let Some(dv) = sup_digit(n) {
                        v = v.saturating_mul(10).saturating_add(dv);
                        digits += 1;
                        it.next();
                    } else {
                        break;
                    }
                }
                if digits == 0 {
                    return Err(UnitError::syntax(
                        pos,
                        "a superscript exponent needs digits",
                    ));
                }
                Tok::Sup(if neg { -v } else { v })
            }
            c if is_ident_start(c) => {
                let mut s = String::from(c);
                while let Some(&(_, n)) = it.peek() {
                    if is_ident_part(n) {
                        s.push(n);
                        it.next();
                    } else {
                        break;
                    }
                }
                Tok::Ident(s)
            }
            other => {
                return Err(UnitError::syntax(
                    pos,
                    format!("unexpected character `{other}`"),
                ))
            }
        };
        out.push(Spanned { kind, pos });
    }
    Ok(out)
}

struct Parser<'a> {
    toks: &'a [Spanned],
    i: usize,
    len: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Spanned> {
        self.toks.get(self.i)
    }

    fn here(&self) -> usize {
        self.peek().map_or(self.len, |t| t.pos)
    }

    fn product(&mut self, depth: usize) -> Result<Unit, UnitError> {
        if depth > MAX_DEPTH {
            return Err(UnitError::syntax(
                self.here(),
                "parentheses nested too deeply",
            ));
        }
        let mut acc = self.power(depth)?;
        loop {
            match self.peek().map(|t| &t.kind) {
                Some(Tok::Star) => {
                    self.i += 1;
                    let r = self.power(depth)?;
                    acc = acc.mul(&r)?;
                }
                Some(Tok::Slash) => {
                    self.i += 1;
                    let r = self.power(depth)?;
                    acc = acc.div(&r)?;
                }
                Some(Tok::Ident(_) | Tok::LParen) => {
                    let r = self.power(depth)?;
                    acc = acc.mul(&r)?;
                }
                _ => return Ok(acc),
            }
        }
    }

    fn power(&mut self, depth: usize) -> Result<Unit, UnitError> {
        let base = self.atom(depth)?;
        match self.peek().map(|t| &t.kind) {
            Some(Tok::Caret) => {
                self.i += 1;
                let e = self.exponent()?;
                base.pow(e)
            }
            Some(Tok::Sup(n)) => {
                let n = *n;
                let pos = self.here();
                self.i += 1;
                let e = i16::try_from(n)
                    .ok()
                    .map(Ratio::int)
                    .ok_or_else(|| UnitError::syntax(pos, "exponent out of range"))?;
                base.pow(e)
            }
            _ => Ok(base),
        }
    }

    fn atom(&mut self, depth: usize) -> Result<Unit, UnitError> {
        let Some(t) = self.toks.get(self.i) else {
            return Err(UnitError::syntax(self.len, "expected a unit"));
        };
        let pos = t.pos;
        match &t.kind {
            Tok::Ident(name) => {
                self.i += 1;
                let (prefix, def) = registry::resolve(name).ok_or_else(|| unknown(name))?;
                Ok(Unit::single(prefix, def, Ratio::ONE))
            }
            Tok::LParen => {
                self.i += 1;
                let u = self.product(depth + 1)?;
                match self.peek().map(|t| &t.kind) {
                    Some(Tok::RParen) => {
                        self.i += 1;
                        Ok(u)
                    }
                    _ => Err(UnitError::syntax(self.here(), "missing `)`")),
                }
            }
            Tok::Num(s) => {
                self.i += 1;
                if s == "1" {
                    Ok(Unit::one())
                } else {
                    Err(UnitError::syntax(
                        pos,
                        format!("only the number 1 is allowed in a unit (found `{s}`)"),
                    ))
                }
            }
            other => Err(UnitError::syntax(
                pos,
                format!("expected a unit, found {}", other.describe()),
            )),
        }
    }

    fn signed_number(&mut self) -> Result<Ratio, UnitError> {
        let mut neg = false;
        match self.peek().map(|t| &t.kind) {
            Some(Tok::Minus) => {
                neg = true;
                self.i += 1;
            }
            Some(Tok::Plus) => self.i += 1,
            _ => {}
        }
        let pos = self.here();
        match self.peek().map(|t| &t.kind) {
            Some(Tok::Num(s)) => {
                let s = s.clone();
                self.i += 1;
                let r = Ratio::from_decimal_str(&s)
                    .ok_or_else(|| UnitError::syntax(pos, format!("invalid exponent `{s}`")))?;
                if neg {
                    r.checked_neg()
                        .ok_or_else(|| UnitError::syntax(pos, "exponent out of range"))
                } else {
                    Ok(r)
                }
            }
            _ => Err(UnitError::syntax(pos, "expected an exponent after `^`")),
        }
    }

    fn exponent(&mut self) -> Result<Ratio, UnitError> {
        if matches!(self.peek().map(|t| &t.kind), Some(Tok::LParen)) {
            self.i += 1;
            let num = self.signed_number()?;
            let mut r = num;
            if matches!(self.peek().map(|t| &t.kind), Some(Tok::Slash)) {
                self.i += 1;
                let pos = self.here();
                let den = self.signed_number()?;
                r = num
                    .checked_div(den)
                    .ok_or_else(|| UnitError::syntax(pos, "invalid fractional exponent"))?;
            }
            if !matches!(self.peek().map(|t| &t.kind), Some(Tok::RParen)) {
                return Err(UnitError::syntax(self.here(), "missing `)` after exponent"));
            }
            self.i += 1;
            Ok(r)
        } else {
            self.signed_number()
        }
    }
}
