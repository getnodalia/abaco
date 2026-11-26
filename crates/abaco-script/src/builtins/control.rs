// SPDX-License-Identifier: MIT OR Apache-2.0
//! Control systems: LTI models as structs (`tf`, `zpk`, `ss`), analysis (`pole`, `zero`, `dcgain`,
//! `bode`, `margin`, `step`, `impulse`, `lsim`, `stepinfo`), design (`place`, `lqr`, `care`,
//! `c2d`, `rlocus`, `pid`) and interconnection (`series`, `parallel`, `feedback`).
//!
//! A model is a struct with a `type` field (`'tf'`, `'zpk'` or `'ss'`) and the sample time `Ts`
//! (0 for continuous time). Operators are not overloaded: use `series`, `parallel` and `feedback`.

use super::{arg, arity, one, real_arg, scalar_arg, Bf, Out};
use crate::error::{err, Error, Result};
use crate::interp::Interp;
use crate::linalg::{num_err, to_matrix};
use crate::value::{Arr, StructV, Value};
use nodalia_numerics::control::{self as nc, StateSpace};
use nodalia_numerics::{eigen, Matrix};
use num_complex::Complex64;

const MAX_POINTS: usize = 200_000;

enum Kind {
    Tf(Vec<f64>, Vec<f64>),
    Zpk(Vec<Complex64>, Vec<Complex64>, f64),
    Ss(StateSpace),
}

struct Model {
    kind: Kind,
    ts: f64,
}

fn trim(p: &[f64]) -> Vec<f64> {
    let first = p
        .iter()
        .position(|&c| c != 0.0)
        .unwrap_or(p.len().saturating_sub(1));
    p[first..].to_vec()
}

fn poly_from_roots(r: &[Complex64]) -> Result<Vec<f64>> {
    let mut p = vec![Complex64::new(1.0, 0.0)];
    for &l in r {
        let mut next = vec![Complex64::new(0.0, 0.0); p.len() + 1];
        for (i, c) in p.iter().enumerate() {
            next[i] += *c;
            next[i + 1] -= *c * l;
        }
        p = next;
    }
    let scale = p.iter().map(|c| c.norm()).fold(1.0, f64::max);
    if p.iter().any(|c| c.im.abs() > 1e-9 * scale) {
        return err(
            "Abaco:domain",
            "complex zeros and poles must come in conjugate pairs",
        );
    }
    Ok(p.iter().map(|c| c.re).collect())
}

fn roots_of(c: &[f64]) -> Result<Vec<Complex64>> {
    let c = trim(c);
    let trailing = c.iter().rev().take_while(|&&x| x == 0.0).count();
    let core = &c[..c.len() - trailing.min(c.len())];
    let mut r = nc::poly_roots(core).map_err(num_err)?;
    r.extend(std::iter::repeat_n(Complex64::new(0.0, 0.0), trailing));
    Ok(r)
}

fn matrix_of(v: &Value, name: &str) -> Result<Matrix<f64>> {
    let a = v
        .to_real()
        .map_err(|e| Error::new(&e.id, format!("`{name}`: {}", e.message)))?;
    to_matrix(&a)
}

fn mat_value(m: &Matrix<f64>) -> Value {
    let (r, c) = (m.rows(), m.cols());
    let mut d = Vec::with_capacity(r * c);
    for j in 0..c {
        for i in 0..r {
            d.push(m.get(i, j).unwrap_or(0.0));
        }
    }
    Value::mat(r, c, d)
}

fn field<'a>(s: &'a StructV, name: &str, what: &str) -> Result<&'a Value> {
    s.get(name).ok_or_else(|| {
        Error::new(
            "Abaco:type",
            format!("the {what} model has no field `{name}`"),
        )
    })
}

fn parse(v: &Value, name: &str) -> Result<Model> {
    let Value::Struct(s) = v else {
        return err(
            "Abaco:type",
            format!(
                "`{name}` needs a model made with tf, zpk or ss, got {}",
                v.describe()
            ),
        );
    };
    let ts = match s.get("Ts") {
        Some(t) => t.scalar_f64("Ts")?,
        None => 0.0,
    };
    let ty = field(s, "type", "LTI")?
        .as_str("the model type")?
        .to_ascii_lowercase();
    let kind = match ty.as_str() {
        "tf" => Kind::Tf(
            field(s, "num", "tf")?.to_real()?.d,
            field(s, "den", "tf")?.to_real()?.d,
        ),
        "zpk" => Kind::Zpk(
            field(s, "z", "zpk")?.to_complex()?.d,
            field(s, "p", "zpk")?.to_complex()?.d,
            field(s, "k", "zpk")?.scalar_f64("the gain k")?,
        ),
        "ss" => Kind::Ss(
            StateSpace::new(
                matrix_of(field(s, "A", "ss")?, name)?,
                matrix_of(field(s, "B", "ss")?, name)?,
                matrix_of(field(s, "C", "ss")?, name)?,
                matrix_of(field(s, "D", "ss")?, name)?,
            )
            .map_err(num_err)?,
        ),
        other => {
            return err(
                "Abaco:type",
                format!("`{name}`: unknown model type '{other}'"),
            )
        }
    };
    if !ts.is_finite() || ts < 0.0 {
        return err(
            "Abaco:domain",
            "the sample time Ts must be 0 (continuous) or positive",
        );
    }
    Ok(Model { kind, ts })
}

fn tf_value(num: Vec<f64>, den: Vec<f64>, ts: f64) -> Value {
    let mut s = StructV::default();
    s.set("type", Value::text("tf"));
    s.set("num", Value::row(num));
    s.set("den", Value::row(den));
    s.set("Ts", Value::num(ts));
    Value::Struct(Box::new(s))
}

fn zpk_value(z: Vec<Complex64>, p: Vec<Complex64>, k: f64, ts: f64) -> Value {
    let mut s = StructV::default();
    s.set("type", Value::text("zpk"));
    s.set(
        "z",
        if z.is_empty() {
            Value::empty()
        } else {
            Value::cplx(Arr::col(z))
        },
    );
    s.set(
        "p",
        if p.is_empty() {
            Value::empty()
        } else {
            Value::cplx(Arr::col(p))
        },
    );
    s.set("k", Value::num(k));
    s.set("Ts", Value::num(ts));
    Value::Struct(Box::new(s))
}

fn ss_value(ss: &StateSpace, ts: f64) -> Value {
    let mut s = StructV::default();
    s.set("type", Value::text("ss"));
    s.set("A", mat_value(&ss.a));
    s.set("B", mat_value(&ss.b));
    s.set("C", mat_value(&ss.c));
    s.set("D", mat_value(&ss.d));
    s.set("Ts", Value::num(ts));
    Value::Struct(Box::new(s))
}

impl Model {
    fn discrete(&self) -> bool {
        self.ts > 0.0
    }

    fn siso_tf(&self) -> Result<(Vec<f64>, Vec<f64>)> {
        match &self.kind {
            Kind::Tf(n, d) => {
                let (n, d) = (trim(n), trim(d));
                if d.first().is_none_or(|&c| c == 0.0) || n.iter().chain(&d).any(|v| !v.is_finite())
                {
                    return err(
                        "Abaco:domain",
                        "the denominator must be non-zero and the coefficients finite",
                    );
                }
                Ok((n, d))
            }
            Kind::Zpk(z, p, k) => {
                let mut n = poly_from_roots(z)?;
                n.iter_mut().for_each(|c| *c *= k);
                Ok((n, poly_from_roots(p)?))
            }
            Kind::Ss(ss) => nc::ss2tf(ss).map_err(num_err),
        }
    }

    fn to_ss(&self) -> Result<StateSpace> {
        match &self.kind {
            Kind::Ss(s) => Ok(s.clone()),
            _ => {
                let (n, d) = self.siso_tf()?;
                nc::tf2ss(&n, &d).map_err(num_err)
            }
        }
    }

    fn poles(&self) -> Result<Vec<Complex64>> {
        match &self.kind {
            Kind::Zpk(_, p, _) => Ok(p.clone()),
            Kind::Ss(s) => eigen::general(&s.a).map_err(num_err),
            Kind::Tf(..) => roots_of(&self.siso_tf()?.1),
        }
    }

    fn zeros(&self) -> Result<Vec<Complex64>> {
        match &self.kind {
            Kind::Zpk(z, _, _) => Ok(z.clone()),
            _ => roots_of(&self.siso_tf()?.0),
        }
    }

    /// The SISO response at the complex point `s`.
    fn eval(&self, s: Complex64) -> Result<Complex64> {
        match &self.kind {
            Kind::Ss(ss) => {
                let m = nc::freqresp(ss, s).map_err(num_err)?;
                if m.rows() != 1 || m.cols() != 1 {
                    return err(
                        "Abaco:size",
                        "this needs a single-input single-output model",
                    );
                }
                Ok(m.get(0, 0).unwrap_or(Complex64::new(f64::NAN, 0.0)))
            }
            _ => {
                let (n, d) = self.siso_tf()?;
                Ok(nc::tf_eval(&n, &d, s))
            }
        }
    }

    fn point(&self, w: f64) -> Complex64 {
        if self.discrete() {
            Complex64::from_polar(1.0, w * self.ts)
        } else {
            Complex64::new(0.0, w)
        }
    }
}

fn gain_model(x: f64, ts: f64) -> Model {
    Model {
        kind: Kind::Tf(vec![x], vec![1.0]),
        ts,
    }
}

/// A model, or a number taken as a static gain.
fn parse_or_gain(v: &Value, name: &str, ts: f64) -> Result<Model> {
    match v {
        Value::Struct(_) => parse(v, name),
        other => Ok(gain_model(other.scalar_f64("a static gain")?, ts)),
    }
}

fn common_ts(a: f64, b: f64) -> Result<f64> {
    if a == b || b == 0.0 {
        Ok(a)
    } else if a == 0.0 {
        Ok(b)
    } else {
        err(
            "Abaco:domain",
            format!("the models have different sample times ({a} and {b})"),
        )
    }
}

// ---------- constructors and conversions ----------

fn row_arg(a: &[Value], i: usize, name: &str) -> Result<Vec<f64>> {
    let v = real_arg(a, i, name)?.d;
    if v.iter().any(|x| !x.is_finite()) {
        return err(
            "Abaco:domain",
            format!("`{name}`: the coefficients must be finite"),
        );
    }
    Ok(v)
}

fn convert_to(v: &Value, name: &str, to: &str, extra: &[Value]) -> Out {
    let m = parse(v, name)?;
    let ts = if let Some(t) = extra.first() {
        t.scalar_f64("Ts")?
    } else {
        m.ts
    };
    match to {
        "tf" => {
            let (n, d) = m.siso_tf()?;
            one(tf_value(n, d, ts))
        }
        "zpk" => {
            if let Kind::Zpk(..) = m.kind {
                return one(v.clone());
            }
            let (n, d) = m.siso_tf()?;
            let k = n.first().copied().unwrap_or(0.0);
            let z = roots_of(&n)?;
            let p = roots_of(&d)?;
            let lead = d.first().copied().unwrap_or(1.0);
            one(zpk_value(z, p, k / lead, ts))
        }
        _ => {
            let ss = m.to_ss()?;
            one(ss_value(&ss, ts))
        }
    }
}

fn f_tf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "tf")?;
    if let Value::Struct(_) = &a[0] {
        return convert_to(&a[0], "tf", "tf", &a[1..]);
    }
    arity(a, 2, 3, "tf")?;
    let (n, d) = (row_arg(a, 0, "tf")?, row_arg(a, 1, "tf")?);
    let ts = if a.len() > 2 {
        scalar_arg(a, 2, "tf")?
    } else {
        0.0
    };
    let m = Model {
        kind: Kind::Tf(n.clone(), d.clone()),
        ts,
    };
    m.siso_tf()?;
    if m.ts < 0.0 || !m.ts.is_finite() {
        return err("Abaco:domain", "the sample time must be 0 or positive");
    }
    one(tf_value(trim(&n), trim(&d), ts))
}

fn f_zpk(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 4, "zpk")?;
    if let Value::Struct(_) = &a[0] {
        return convert_to(&a[0], "zpk", "zpk", &a[1..]);
    }
    arity(a, 3, 4, "zpk")?;
    let z = arg(a, 0, "zpk")?.to_complex()?.d;
    let p = arg(a, 1, "zpk")?.to_complex()?.d;
    let k = scalar_arg(a, 2, "zpk")?;
    let ts = if a.len() > 3 {
        scalar_arg(a, 3, "zpk")?
    } else {
        0.0
    };
    if z.len() > p.len() {
        return err("Abaco:domain", "`zpk`: more zeros than poles (improper)");
    }
    poly_from_roots(&z)?;
    poly_from_roots(&p)?;
    one(zpk_value(z, p, k, ts))
}

fn f_ss(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 5, "ss")?;
    if let Value::Struct(_) = &a[0] {
        return convert_to(&a[0], "ss", "ss", &a[1..]);
    }
    arity(a, 4, 5, "ss")?;
    let ss = StateSpace::new(
        matrix_of(&a[0], "ss")?,
        matrix_of(&a[1], "ss")?,
        matrix_of(&a[2], "ss")?,
        matrix_of(&a[3], "ss")?,
    )
    .map_err(num_err)?;
    let ts = if a.len() > 4 {
        scalar_arg(a, 4, "ss")?
    } else {
        0.0
    };
    one(ss_value(&ss, ts))
}

fn f_tf2ss(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "tf2ss")?;
    let ss = nc::tf2ss(&row_arg(a, 0, "tf2ss")?, &row_arg(a, 1, "tf2ss")?).map_err(num_err)?;
    Ok(vec![
        mat_value(&ss.a),
        mat_value(&ss.b),
        mat_value(&ss.c),
        mat_value(&ss.d),
    ])
}

fn f_ss2tf(_: &mut Interp, a: &[Value], _: usize) -> Out {
    let ss = if let Some(Value::Struct(_)) = a.first() {
        arity(a, 1, 1, "ss2tf")?;
        parse(&a[0], "ss2tf")?.to_ss()?
    } else {
        arity(a, 4, 4, "ss2tf")?;
        StateSpace::new(
            matrix_of(&a[0], "ss2tf")?,
            matrix_of(&a[1], "ss2tf")?,
            matrix_of(&a[2], "ss2tf")?,
            matrix_of(&a[3], "ss2tf")?,
        )
        .map_err(num_err)?
    };
    let (n, d) = nc::ss2tf(&ss).map_err(num_err)?;
    Ok(vec![Value::row(n), Value::row(d)])
}

fn f_tfdata(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "tfdata")?;
    let (n, d) = parse(&a[0], "tfdata")?.siso_tf()?;
    Ok(vec![Value::row(n), Value::row(d)])
}

fn f_ssdata(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "ssdata")?;
    let ss = parse(&a[0], "ssdata")?.to_ss()?;
    Ok(vec![
        mat_value(&ss.a),
        mat_value(&ss.b),
        mat_value(&ss.c),
        mat_value(&ss.d),
    ])
}

fn f_zpkdata(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "zpkdata")?;
    let m = parse(&a[0], "zpkdata")?;
    let (z, p) = (m.zeros()?, m.poles()?);
    let k = match &m.kind {
        Kind::Zpk(_, _, k) => *k,
        _ => {
            let (n, d) = m.siso_tf()?;
            n.first().copied().unwrap_or(0.0) / d.first().copied().unwrap_or(1.0)
        }
    };
    let cv = |v: Vec<Complex64>| {
        if v.is_empty() {
            Value::empty()
        } else {
            Value::cplx(Arr::col(v))
        }
    };
    Ok(vec![cv(z), cv(p), Value::num(k)])
}

// ---------- analysis ----------

fn f_pole(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "pole")?;
    let p = parse(&a[0], "pole")?.poles()?;
    one(if p.is_empty() {
        Value::empty()
    } else {
        Value::cplx(Arr::col(p))
    })
}

fn f_zero(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "zero")?;
    let z = parse(&a[0], "zero")?.zeros()?;
    one(if z.is_empty() {
        Value::empty()
    } else {
        Value::cplx(Arr::col(z))
    })
}

fn f_pzmap(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "pzmap")?;
    let m = parse(&a[0], "pzmap")?;
    let cv = |v: Vec<Complex64>| {
        if v.is_empty() {
            Value::empty()
        } else {
            Value::cplx(Arr::col(v))
        }
    };
    Ok(vec![cv(m.poles()?), cv(m.zeros()?)])
}

fn is_stable(m: &Model) -> Result<bool> {
    Ok(m.poles()?.iter().all(|p| {
        if m.discrete() {
            p.norm() < 1.0
        } else {
            p.re < 0.0
        }
    }))
}

fn f_isstable(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "isstable")?;
    one(Value::boolean(is_stable(&parse(&a[0], "isstable")?)?))
}

fn f_dcgain(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "dcgain")?;
    let m = parse(&a[0], "dcgain")?;
    let s = if m.discrete() {
        Complex64::new(1.0, 0.0)
    } else {
        Complex64::new(0.0, 0.0)
    };
    if let Kind::Ss(ss) = &m.kind {
        if ss.inputs() != 1 || ss.outputs() != 1 {
            let g = match nc::freqresp(ss, s) {
                Ok(g) => g,
                Err(_) => return one(Value::num(f64::INFINITY)),
            };
            let mut d = Vec::new();
            for j in 0..g.cols() {
                for i in 0..g.rows() {
                    d.push(g.get(i, j).map_or(f64::NAN, |z| z.re));
                }
            }
            return one(Value::mat(g.rows(), g.cols(), d));
        }
    }
    let (n, d) = match &m.kind {
        Kind::Ss(_) => (Vec::new(), Vec::new()),
        _ => m.siso_tf()?,
    };
    let g = match &m.kind {
        Kind::Ss(ss) => nc::freqresp(ss, s)
            .ok()
            .and_then(|g| g.get(0, 0))
            .unwrap_or(Complex64::new(f64::INFINITY, 0.0)),
        _ => {
            let den = d
                .iter()
                .fold(Complex64::new(0.0, 0.0), |acc, &k| acc * s + k);
            if den.norm() < 1e-12 * d.iter().map(|v| v.abs()).fold(1.0, f64::max) {
                Complex64::new(f64::INFINITY, 0.0)
            } else {
                nc::tf_eval(&n, &d, s)
            }
        }
    };
    one(Value::num(g.re))
}

fn f_damp(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "damp")?;
    let m = parse(&a[0], "damp")?;
    let mut wn = Vec::new();
    let mut zeta = Vec::new();
    let mut ps = Vec::new();
    for p in m.poles()? {
        let s = if m.discrete() { p.ln() / m.ts } else { p };
        let w = s.norm();
        wn.push(w);
        zeta.push(if w == 0.0 { -1.0 } else { -s.re / w });
        ps.push(p);
    }
    Ok(vec![
        Value::col(wn),
        Value::col(zeta),
        Value::cplx(Arr::col(ps)),
    ])
}

fn f_evalfr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "evalfr")?;
    let m = parse(&a[0], "evalfr")?;
    let s = arg(a, 1, "evalfr")?.to_complex()?;
    if s.d.len() != 1 {
        return err("Abaco:size", "`evalfr` evaluates at one complex point");
    }
    one(Value::cscalar(m.eval(s.d[0])?))
}

fn f_ctrb(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "ctrb")?;
    one(mat_value(
        &nc::ctrb(&matrix_of(&a[0], "ctrb")?, &matrix_of(&a[1], "ctrb")?).map_err(num_err)?,
    ))
}

fn f_obsv(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "obsv")?;
    one(mat_value(
        &nc::obsv(&matrix_of(&a[0], "obsv")?, &matrix_of(&a[1], "obsv")?).map_err(num_err)?,
    ))
}

// ---------- interconnection ----------

fn polymul(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

fn polyadd(a: &[f64], b: &[f64], sign: f64) -> Vec<f64> {
    let n = a.len().max(b.len());
    let mut out = vec![0.0; n];
    for (i, x) in a.iter().enumerate() {
        out[n - a.len() + i] += x;
    }
    for (i, y) in b.iter().enumerate() {
        out[n - b.len() + i] += sign * y;
    }
    out
}

fn two_models(a: &[Value], name: &str) -> Result<(Model, Model, f64)> {
    let first = parse_or_gain(arg(a, 0, name)?, name, 0.0)?;
    let second = parse_or_gain(arg(a, 1, name)?, name, first.ts)?;
    let ts = common_ts(first.ts, second.ts)?;
    Ok((first, second, ts))
}

fn f_series(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "series")?;
    let (g1, g2, ts) = two_models(a, "series")?;
    let ((n1, d1), (n2, d2)) = (g1.siso_tf()?, g2.siso_tf()?);
    one(tf_value(trim(&polymul(&n1, &n2)), polymul(&d1, &d2), ts))
}

fn f_parallel(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "parallel")?;
    let (g1, g2, ts) = two_models(a, "parallel")?;
    let ((n1, d1), (n2, d2)) = (g1.siso_tf()?, g2.siso_tf()?);
    let num = polyadd(&polymul(&n1, &d2), &polymul(&n2, &d1), 1.0);
    one(tf_value(trim(&num), polymul(&d1, &d2), ts))
}

fn f_feedback(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 3, "feedback")?;
    let g = parse_or_gain(&a[0], "feedback", 0.0)?;
    let h = match a.get(1) {
        Some(v) => parse_or_gain(v, "feedback", g.ts)?,
        None => gain_model(1.0, g.ts),
    };
    let sign = if a.len() > 2 {
        scalar_arg(a, 2, "feedback")?
    } else {
        -1.0
    };
    if sign != 1.0 && sign != -1.0 {
        return err(
            "Abaco:domain",
            "the feedback sign must be -1 (negative) or +1 (positive)",
        );
    }
    let ts = common_ts(g.ts, h.ts)?;
    let ((n1, d1), (n2, d2)) = (g.siso_tf()?, h.siso_tf()?);
    let num = polymul(&n1, &d2);
    let den = polyadd(&polymul(&d1, &d2), &polymul(&n1, &n2), -sign);
    one(tf_value(trim(&num), trim(&den), ts))
}

fn f_pid(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 4, "pid")?;
    let g = |i: usize| -> Result<f64> {
        if i < a.len() {
            scalar_arg(a, i, "pid")
        } else {
            Ok(0.0)
        }
    };
    let (kp, ki, kd, tf) = (g(0)?, g(1)?, g(2)?, g(3)?);
    if tf < 0.0 || (kd != 0.0 && tf == 0.0) {
        return err(
            "Abaco:domain",
            "a derivative term needs a positive filter time constant Tf (the ideal derivative is improper)",
        );
    }
    // C = Kp + Ki/s + Kd s/(Tf s + 1)  over  s (Tf s + 1)
    let den = vec![tf, 1.0, 0.0];
    let num = polyadd(
        &polyadd(&[kp * tf, kp, 0.0], &[ki * tf, ki], 1.0),
        &[kd, 0.0, 0.0],
        1.0,
    );
    if ki == 0.0 {
        // Cancel the integrator pole: C = (Kp (Tf s + 1) + Kd s) / (Tf s + 1).
        let num2 = polyadd(&[kp * tf, kp], &[kd, 0.0], 1.0);
        return one(tf_value(
            trim(&num2),
            vec![tf, 1.0]
                .into_iter()
                .skip_while(|&c| c == 0.0)
                .collect(),
            0.0,
        ));
    }
    one(tf_value(trim(&num), trim(&den), 0.0))
}

// ---------- time responses ----------

#[derive(Clone, Copy, PartialEq)]
enum Resp {
    Step,
    Impulse,
}

fn auto_horizon(m: &Model) -> Result<(f64, usize)> {
    let poles = m.poles()?;
    if m.discrete() {
        let slow = poles
            .iter()
            .filter(|p| p.norm() > 1e-12 && p.norm() < 1.0)
            .map(|p| -p.norm().ln())
            .fold(f64::INFINITY, f64::min);
        let steps = if slow.is_finite() {
            (7.0 / slow).ceil().clamp(20.0, 5000.0) as usize
        } else {
            100
        };
        return Ok(((steps as f64) * m.ts, steps + 1));
    }
    let slow = poles
        .iter()
        .filter(|p| p.re < -1e-9)
        .map(|p| -p.re)
        .fold(f64::INFINITY, f64::min);
    let fast = poles.iter().map(|p| p.norm()).fold(0.0, f64::max);
    let any_unstable = poles.iter().any(|p| p.re > 1e-9);
    let tfinal = if any_unstable {
        (5.0 / poles.iter().map(|p| p.re).fold(0.0, f64::max)).min(1e6)
    } else if slow.is_finite() {
        7.0 / slow
    } else if fast > 0.0 {
        20.0 / fast
    } else {
        10.0
    };
    let n = (tfinal * fast * 10.0).ceil().clamp(400.0, 5000.0) as usize;
    Ok((tfinal, n + 1))
}

/// The sample times for a response: from `tfinal`, a vector, or chosen automatically.
fn time_grid(m: &Model, v: Option<&Value>, name: &str) -> Result<(Vec<f64>, f64)> {
    let make = |tfinal: f64, n: usize| -> Result<(Vec<f64>, f64)> {
        if !(tfinal.is_finite() && tfinal > 0.0) {
            return err(
                "Abaco:domain",
                format!("`{name}`: the final time must be positive and finite"),
            );
        }
        if m.discrete() {
            let steps = ((tfinal / m.ts).floor() as usize).clamp(1, MAX_POINTS);
            return Ok(((0..=steps).map(|k| k as f64 * m.ts).collect(), m.ts));
        }
        let n = n.clamp(2, MAX_POINTS);
        let dt = tfinal / (n - 1) as f64;
        Ok(((0..n).map(|k| k as f64 * dt).collect(), dt))
    };
    match v {
        None => {
            let (tf, n) = auto_horizon(m)?;
            make(tf, n)
        }
        Some(v) => {
            let t = v.to_real()?.d;
            match t.len() {
                0 => {
                    let (tf, n) = auto_horizon(m)?;
                    make(tf, n)
                }
                1 => {
                    let tf = t[0];
                    let n = if m.discrete() { 0 } else { auto_horizon(m)?.1 };
                    make(tf, n)
                }
                n => {
                    if n > MAX_POINTS || t.iter().any(|x| !x.is_finite()) {
                        return err(
                            "Abaco:limit",
                            format!("`{name}`: the time vector is too long or not finite"),
                        );
                    }
                    let dt = t[1] - t[0];
                    if dt <= 0.0 || t.windows(2).any(|w| ((w[1] - w[0]) - dt).abs() > 1e-6 * dt) {
                        return err(
                            "Abaco:domain",
                            format!(
                                "`{name}`: the time vector must be uniformly spaced and increasing"
                            ),
                        );
                    }
                    if m.discrete() && (dt - m.ts).abs() > 1e-9 * m.ts {
                        return err(
                            "Abaco:domain",
                            format!(
                                "`{name}`: the time step must equal the sample time {}",
                                m.ts
                            ),
                        );
                    }
                    Ok((t.clone(), dt))
                }
            }
        }
    }
}

fn plot_response(i: &mut Interp, t: &[f64], y: &[f64], title: &str) -> Result<()> {
    let args = vec![Value::col(t.to_vec()), Value::col(y.to_vec())];
    super::call(i, "plot", &args, 0)?;
    super::call(i, "xlabel", &[Value::text("Time")], 0)?;
    super::call(i, "ylabel", &[Value::text("Amplitude")], 0)?;
    super::call(i, "title", &[Value::text(title)], 0)?;
    super::call(i, "grid", &[Value::text("on")], 0)?;
    Ok(())
}

fn response(i: &mut Interp, a: &[Value], nargout: usize, name: &str, kind: Resp) -> Out {
    arity(a, 1, 2, name)?;
    let m = parse(&a[0], name)?;
    let ss = m.to_ss()?;
    if ss.inputs() != 1 || ss.outputs() != 1 {
        return err(
            "Abaco:size",
            format!(
                "`{name}` needs a single-input single-output model; use lsim for several inputs"
            ),
        );
    }
    let (t, dt) = time_grid(&m, a.get(1), name)?;
    i.charge(t.len() as u64)?;
    let n = t.len();
    let discrete = m.discrete();
    let (u, x0): (Vec<Vec<f64>>, Vec<f64>) = match kind {
        Resp::Step => (vec![vec![1.0]; n], vec![0.0; ss.states()]),
        Resp::Impulse => {
            if discrete {
                let mut u = vec![vec![0.0]; n];
                u[0][0] = 1.0;
                (u, vec![0.0; ss.states()])
            } else {
                // The Dirac input puts B into the state at t = 0.
                (
                    vec![vec![0.0]; n],
                    (0..ss.states())
                        .map(|k| ss.b.get(k, 0).unwrap_or(0.0))
                        .collect(),
                )
            }
        }
    };
    let (ys, xs) = nc::simulate(&ss, discrete, dt, &u, &x0).map_err(num_err)?;
    let y: Vec<f64> = ys.iter().map(|r| r[0]).collect();
    if nargout == 0 {
        plot_response(
            i,
            &t,
            &y,
            if kind == Resp::Step {
                "Step Response"
            } else {
                "Impulse Response"
            },
        )?;
        return Ok(vec![]);
    }
    let ns = ss.states();
    let mut xcols = Vec::with_capacity(n * ns);
    for k in 0..ns {
        for row in &xs {
            xcols.push(row[k]);
        }
    }
    Ok(vec![Value::col(y), Value::col(t), Value::mat(n, ns, xcols)])
}

fn f_step(i: &mut Interp, a: &[Value], n: usize) -> Out {
    response(i, a, n, "step", Resp::Step)
}

fn f_impulse(i: &mut Interp, a: &[Value], n: usize) -> Out {
    response(i, a, n, "impulse", Resp::Impulse)
}

fn f_lsim(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 3, 4, "lsim")?;
    let m = parse(&a[0], "lsim")?;
    let ss = m.to_ss()?;
    let u = real_arg(a, 1, "lsim")?;
    let t = real_arg(a, 2, "lsim")?.d;
    let (nt, inputs) = (t.len(), ss.inputs());
    if !(2..=MAX_POINTS).contains(&nt) {
        return err(
            "Abaco:size",
            "`lsim` needs between 2 and 200000 time points",
        );
    }
    let dt = t[1] - t[0];
    if dt.is_nan()
        || dt <= 0.0
        || t.iter().any(|x| !x.is_finite())
        || t.windows(2).any(|w| ((w[1] - w[0]) - dt).abs() > 1e-6 * dt)
    {
        return err(
            "Abaco:domain",
            "`lsim`: the time vector must be uniformly spaced and increasing",
        );
    }
    // u is nt x m (a vector of nt values is accepted for one input).
    let (ur, uc) = if u.r == 1 && inputs == 1 {
        (u.c, 1)
    } else {
        (u.r, u.c)
    };
    if ur != nt || uc != inputs {
        return err(
            "Abaco:size",
            format!(
                "`lsim`: the input must be {nt}x{inputs}, it is {}x{}",
                u.r, u.c
            ),
        );
    }
    let rows: Vec<Vec<f64>> = (0..nt)
        .map(|k| (0..inputs).map(|j| u.d[j * ur + k]).collect())
        .collect();
    let x0 = match a.get(3) {
        Some(v) => v.to_real()?.d,
        None => vec![0.0; ss.states()],
    };
    i.charge(nt as u64)?;
    let (ys, xs) = nc::simulate(&ss, m.discrete(), dt, &rows, &x0).map_err(num_err)?;
    let p = ss.outputs();
    let mut y = Vec::with_capacity(nt * p);
    for j in 0..p {
        for r in &ys {
            y.push(r[j]);
        }
    }
    if nargout == 0 {
        if p == 1 {
            plot_response(i, &t, &y, "Linear Simulation Results")?;
        }
        return Ok(vec![]);
    }
    let ns = ss.states();
    let mut xc = Vec::with_capacity(nt * ns);
    for k in 0..ns {
        for r in &xs {
            xc.push(r[k]);
        }
    }
    Ok(vec![
        Value::mat(nt, p, y),
        Value::col(t),
        Value::mat(nt, ns, xc),
    ])
}

fn f_initial(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 2, 3, "initial")?;
    let m = parse(&a[0], "initial")?;
    let ss = m.to_ss()?;
    let x0 = real_arg(a, 1, "initial")?.d;
    let (t, dt) = time_grid(&m, a.get(2), "initial")?;
    i.charge(t.len() as u64)?;
    let u = vec![vec![0.0; ss.inputs()]; t.len()];
    let (ys, _) = nc::simulate(&ss, m.discrete(), dt, &u, &x0).map_err(num_err)?;
    let p = ss.outputs();
    let n = t.len();
    let mut y = Vec::with_capacity(n * p);
    for j in 0..p {
        for r in &ys {
            y.push(r[j]);
        }
    }
    if nargout == 0 {
        if p == 1 {
            plot_response(i, &t, &y, "Initial Condition Results")?;
        }
        return Ok(vec![]);
    }
    Ok(vec![Value::mat(n, p, y), Value::col(t)])
}

fn f_stepinfo(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 2, "stepinfo")?;
    let m = parse(&a[0], "stepinfo")?;
    let ss = m.to_ss()?;
    if ss.inputs() != 1 || ss.outputs() != 1 {
        return err(
            "Abaco:size",
            "`stepinfo` needs a single-input single-output model",
        );
    }
    let (t, dt) = time_grid(&m, a.get(1), "stepinfo")?;
    i.charge(t.len() as u64)?;
    let u = vec![vec![1.0]; t.len()];
    let (ys, _) =
        nc::simulate(&ss, m.discrete(), dt, &u, &vec![0.0; ss.states()]).map_err(num_err)?;
    let y: Vec<f64> = ys.iter().map(|r| r[0]).collect();
    let s = if m.discrete() {
        Complex64::new(1.0, 0.0)
    } else {
        Complex64::new(0.0, 0.0)
    };
    let yss = if is_stable(&m)? {
        m.eval(s).map(|z| z.re).unwrap_or(f64::NAN)
    } else {
        f64::NAN
    };
    let last = y.last().copied().unwrap_or(f64::NAN);
    let fin = if yss.is_finite() { yss } else { last };
    let cross = |level: f64| -> f64 {
        for k in 1..y.len() {
            if (y[k - 1] - level) * (y[k] - level) <= 0.0 && y[k] != y[k - 1] {
                let f = (level - y[k - 1]) / (y[k] - y[k - 1]);
                return t[k - 1] + f * (t[k] - t[k - 1]);
            }
        }
        f64::NAN
    };
    let rise = if fin != 0.0 {
        cross(0.9 * fin) - cross(0.1 * fin)
    } else {
        f64::NAN
    };
    let band = 0.02 * fin.abs();
    let settle = match y.iter().rposition(|v| (v - fin).abs() > band) {
        None => 0.0,
        Some(k) if k + 1 < y.len() => t[k + 1],
        Some(_) => f64::NAN,
    };
    let (mut peak, mut peak_t) = (f64::NEG_INFINITY, 0.0);
    for (k, v) in y.iter().enumerate() {
        if v.abs() > peak.abs() || k == 0 {
            peak = *v;
            peak_t = t[k];
        }
    }
    let over = if fin != 0.0 {
        ((peak - fin) / fin.abs() * 100.0).max(0.0)
    } else {
        0.0
    };
    let under = if fin != 0.0 {
        (-(y.iter().cloned().fold(f64::INFINITY, f64::min)) / fin.abs() * 100.0).max(0.0)
    } else {
        0.0
    };
    let in_band: Vec<f64> = match y.iter().rposition(|v| (v - fin).abs() > band) {
        Some(k) => y[k + 1..].to_vec(),
        None => y.clone(),
    };
    let mut st = StructV::default();
    st.set("RiseTime", Value::num(rise));
    st.set("TransientTime", Value::num(settle));
    st.set("SettlingTime", Value::num(settle));
    st.set(
        "SettlingMin",
        Value::num(in_band.iter().cloned().fold(f64::INFINITY, f64::min)),
    );
    st.set(
        "SettlingMax",
        Value::num(in_band.iter().cloned().fold(f64::NEG_INFINITY, f64::max)),
    );
    st.set("Overshoot", Value::num(over));
    st.set("Undershoot", Value::num(under));
    st.set("Peak", Value::num(peak.abs()));
    st.set("PeakTime", Value::num(peak_t));
    st.set("SteadyState", Value::num(yss));
    one(Value::Struct(Box::new(st)))
}

// ---------- frequency responses ----------

fn auto_freqs(m: &Model) -> Result<Vec<f64>> {
    let mut mags: Vec<f64> = m
        .poles()?
        .iter()
        .chain(m.zeros()?.iter())
        .map(|z| {
            if m.discrete() {
                (z.ln() / m.ts).norm()
            } else {
                z.norm()
            }
        })
        .filter(|v| v.is_finite() && *v > 1e-9)
        .collect();
    let (lo, hi) = if mags.is_empty() {
        (0.1, 100.0)
    } else {
        mags.sort_by(|p, q| p.total_cmp(q));
        let (l, h) = (mags[0], mags[mags.len() - 1]);
        (l / 10.0, h * 10.0)
    };
    let hi = if m.discrete() {
        hi.min(std::f64::consts::PI / m.ts)
    } else {
        hi
    };
    let lo = lo.min(hi / 10.0);
    let n = 400;
    let (a, b) = (lo.log10(), hi.log10());
    Ok((0..n)
        .map(|k| 10f64.powf(a + (b - a) * k as f64 / (n - 1) as f64))
        .collect())
}

fn freq_arg(m: &Model, a: &[Value], i: usize, name: &str) -> Result<Vec<f64>> {
    match a.get(i) {
        None => auto_freqs(m),
        Some(v) => {
            let w = v.to_real()?.d;
            if w.is_empty() || w.len() > MAX_POINTS || w.iter().any(|x| !x.is_finite() || *x < 0.0)
            {
                return err("Abaco:domain", format!("`{name}`: the frequencies must be 1 to {MAX_POINTS} finite non-negative values"));
            }
            Ok(w)
        }
    }
}

fn unwrap_deg(args: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(args.len());
    let mut offset = 0.0;
    let mut prev = 0.0;
    for (k, &a) in args.iter().enumerate() {
        if k > 0 {
            let d = a - prev;
            if d > std::f64::consts::PI {
                offset -= 2.0 * std::f64::consts::PI;
            } else if d < -std::f64::consts::PI {
                offset += 2.0 * std::f64::consts::PI;
            }
        }
        prev = a;
        out.push((a + offset).to_degrees());
    }
    out
}

fn f_freqresp(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "freqresp")?;
    let m = parse(&a[0], "freqresp")?;
    let w = freq_arg(&m, a, 1, "freqresp")?;
    i.charge(w.len() as u64)?;
    let g: Result<Vec<Complex64>> = w.iter().map(|&x| m.eval(m.point(x))).collect();
    one(Value::cplx(Arr::col(g?)))
}

fn f_bode(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 1, 2, "bode")?;
    let m = parse(&a[0], "bode")?;
    let w = freq_arg(&m, a, 1, "bode")?;
    i.charge(w.len() as u64)?;
    let g = w
        .iter()
        .map(|&x| m.eval(m.point(x)))
        .collect::<Result<Vec<_>>>()?;
    let mag: Vec<f64> = g.iter().map(|z| z.norm()).collect();
    let phase = unwrap_deg(&g.iter().map(|z| z.arg()).collect::<Vec<_>>());
    if nargout == 0 {
        let db: Vec<f64> = mag.iter().map(|v| 20.0 * v.log10()).collect();
        super::call(
            i,
            "subplot",
            &[Value::num(2.0), Value::num(1.0), Value::num(1.0)],
            0,
        )?;
        super::call(i, "semilogx", &[Value::col(w.clone()), Value::col(db)], 0)?;
        super::call(i, "ylabel", &[Value::text("Magnitude (dB)")], 0)?;
        super::call(i, "title", &[Value::text("Bode Diagram")], 0)?;
        super::call(i, "grid", &[Value::text("on")], 0)?;
        super::call(
            i,
            "subplot",
            &[Value::num(2.0), Value::num(1.0), Value::num(2.0)],
            0,
        )?;
        super::call(i, "semilogx", &[Value::col(w), Value::col(phase)], 0)?;
        super::call(i, "ylabel", &[Value::text("Phase (deg)")], 0)?;
        super::call(i, "xlabel", &[Value::text("Frequency (rad/s)")], 0)?;
        super::call(i, "grid", &[Value::text("on")], 0)?;
        return Ok(vec![]);
    }
    Ok(vec![Value::col(mag), Value::col(phase), Value::col(w)])
}

fn f_nyquist(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 1, 2, "nyquist")?;
    let m = parse(&a[0], "nyquist")?;
    let w = freq_arg(&m, a, 1, "nyquist")?;
    i.charge(w.len() as u64)?;
    let g = w
        .iter()
        .map(|&x| m.eval(m.point(x)))
        .collect::<Result<Vec<_>>>()?;
    let (re, im): (Vec<f64>, Vec<f64>) = g.iter().map(|z| (z.re, z.im)).unzip();
    if nargout == 0 {
        super::call(i, "plot", &[Value::col(re), Value::col(im)], 0)?;
        super::call(i, "xlabel", &[Value::text("Real Axis")], 0)?;
        super::call(i, "ylabel", &[Value::text("Imaginary Axis")], 0)?;
        super::call(i, "title", &[Value::text("Nyquist Diagram")], 0)?;
        super::call(i, "grid", &[Value::text("on")], 0)?;
        return Ok(vec![]);
    }
    Ok(vec![Value::col(re), Value::col(im), Value::col(w)])
}

/// A wider grid than `bode`'s, for locating crossings: two decades beyond the
/// extreme poles and zeros, and up to the Nyquist frequency when discrete.
fn margin_freqs(m: &Model) -> Result<Vec<f64>> {
    let base = auto_freqs(m)?;
    let lo = base.first().copied().unwrap_or(0.1) / 100.0;
    let hi = if m.discrete() {
        std::f64::consts::PI / m.ts
    } else {
        base.last().copied().unwrap_or(100.0) * 100.0
    };
    let n = 6000;
    let (a, b) = (lo.log10(), hi.log10());
    Ok((0..n)
        .map(|k| 10f64.powf(a + (b - a) * k as f64 / (n - 1) as f64))
        .collect())
}

fn f_margin(i: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 1, 1, "margin")?;
    let m = parse(&a[0], "margin")?;
    let w = margin_freqs(&m)?;
    i.charge(w.len() as u64)?;
    let g = w
        .iter()
        .map(|&x| m.eval(m.point(x)))
        .collect::<Result<Vec<_>>>()?;
    let ph = unwrap_deg(&g.iter().map(|z| z.arg()).collect::<Vec<_>>());
    let mag: Vec<f64> = g.iter().map(|z| z.norm()).collect();
    // Bisection between two grid points on a scalar function of the frequency.
    let refine = |lo: f64, hi: f64, f: &dyn Fn(f64) -> f64| -> f64 {
        let (mut a0, mut b0) = (lo, hi);
        let fa = f(a0);
        for _ in 0..60 {
            let mid = (a0 * b0).sqrt();
            if (f(mid) < 0.0) == (fa < 0.0) {
                a0 = mid;
            } else {
                b0 = mid;
            }
        }
        (a0 * b0).sqrt()
    };
    let phase_at = |x: f64| -> f64 {
        // Continuous phase near x: take the principal value shifted to the grid's branch.
        m.eval(m.point(x))
            .map_or(f64::NAN, |z| z.arg().to_degrees())
    };
    let (mut gm, mut wcg) = (f64::INFINITY, f64::NAN);
    let (mut pm, mut wcp) = (f64::INFINITY, f64::NAN);
    for k in 1..w.len() {
        // Phase crosses -180 + 360 j.
        let (p0, p1) = ((ph[k - 1] + 180.0) / 360.0, (ph[k] + 180.0) / 360.0);
        if p0.floor() != p1.floor() {
            let target = p0.max(p1).floor() * 360.0 - 180.0;
            let wc = refine(w[k - 1], w[k], &|x| {
                let mut p = phase_at(x);
                // Bring the principal phase onto the unwrapped branch of the grid point.
                p += ((ph[k - 1] - p) / 360.0).round() * 360.0;
                p - target
            });
            let gain = m.eval(m.point(wc)).map_or(f64::NAN, |z| z.norm());
            if gain > 0.0 && 1.0 / gain < gm {
                gm = 1.0 / gain;
                wcg = wc;
            }
        }
        // |G| crosses 1.
        if (mag[k - 1] - 1.0) * (mag[k] - 1.0) < 0.0 {
            let wc = refine(w[k - 1], w[k], &|x| {
                m.eval(m.point(x)).map_or(f64::NAN, |z| z.norm()) - 1.0
            });
            let mut p = phase_at(wc);
            p += ((ph[k - 1] - p) / 360.0).round() * 360.0;
            let margin = 180.0 + p;
            if margin.abs() < pm.abs() || !pm.is_finite() {
                pm = margin;
                wcp = wc;
            }
        }
    }
    Ok(vec![
        Value::num(gm),
        Value::num(pm),
        Value::num(wcg),
        Value::num(wcp),
    ])
}

// ---------- design ----------

fn f_place(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 3, 3, "place")?;
    let poles = a[2].to_complex()?.d;
    let k = nc::place(
        &matrix_of(&a[0], "place")?,
        &matrix_of(&a[1], "place")?,
        &poles,
    )
    .map_err(num_err)?;
    one(Value::row(k))
}

type LqrArgs = (Matrix<f64>, Matrix<f64>, Matrix<f64>, Matrix<f64>, bool);

fn lqr_args(a: &[Value], name: &str) -> Result<LqrArgs> {
    if let Some(Value::Struct(_)) = a.first() {
        arity(a, 3, 3, name)?;
        let m = parse(&a[0], name)?;
        let ss = m.to_ss()?;
        return Ok((
            ss.a,
            ss.b,
            matrix_of(&a[1], name)?,
            matrix_of(&a[2], name)?,
            m.discrete(),
        ));
    }
    arity(a, 4, 4, name)?;
    Ok((
        matrix_of(&a[0], name)?,
        matrix_of(&a[1], name)?,
        matrix_of(&a[2], name)?,
        matrix_of(&a[3], name)?,
        false,
    ))
}

fn lqr_out(a: &[Value], name: &str, force_discrete: bool) -> Out {
    let (am, bm, q, r, disc) = lqr_args(a, name)?;
    let sol = nc::lqr(&am, &bm, &q, &r, disc || force_discrete).map_err(num_err)?;
    Ok(vec![
        mat_value(&sol.k),
        mat_value(&sol.s),
        Value::cplx(Arr::col(sol.poles)),
    ])
}

fn f_lqr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    lqr_out(a, "lqr", false)
}

fn f_dlqr(_: &mut Interp, a: &[Value], _: usize) -> Out {
    lqr_out(a, "dlqr", true)
}

fn f_care(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 4, 4, "care")?;
    let (am, bm, q, r) = (
        matrix_of(&a[0], "care")?,
        matrix_of(&a[1], "care")?,
        matrix_of(&a[2], "care")?,
        matrix_of(&a[3], "care")?,
    );
    one(mat_value(&nc::care(&am, &bm, &q, &r).map_err(num_err)?))
}

fn f_dare(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 4, 4, "dare")?;
    let (am, bm, q, r) = (
        matrix_of(&a[0], "dare")?,
        matrix_of(&a[1], "dare")?,
        matrix_of(&a[2], "dare")?,
        matrix_of(&a[3], "dare")?,
    );
    one(mat_value(&nc::dare(&am, &bm, &q, &r).map_err(num_err)?))
}

fn f_lyap(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "lyap")?;
    one(mat_value(
        &nc::lyap(&matrix_of(&a[0], "lyap")?, &matrix_of(&a[1], "lyap")?).map_err(num_err)?,
    ))
}

fn f_dlyap(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 2, "dlyap")?;
    one(mat_value(
        &nc::dlyap(&matrix_of(&a[0], "dlyap")?, &matrix_of(&a[1], "dlyap")?).map_err(num_err)?,
    ))
}

fn f_c2d(_: &mut Interp, a: &[Value], _: usize) -> Out {
    arity(a, 2, 3, "c2d")?;
    let m = parse(&a[0], "c2d")?;
    if m.discrete() {
        return err("Abaco:domain", "`c2d` needs a continuous-time model");
    }
    let ts = scalar_arg(a, 1, "c2d")?;
    let method = match a.get(2) {
        Some(v) => v.as_str("the method")?.to_ascii_lowercase(),
        None => "zoh".to_string(),
    };
    let ss = m.to_ss()?;
    let d = match method.as_str() {
        "zoh" => nc::c2d_zoh(&ss, ts),
        "tustin" | "bilinear" => nc::c2d_tustin(&ss, ts),
        other => {
            return err(
                "Abaco:domain",
                format!("`c2d`: unknown method '{other}' (zoh or tustin)"),
            )
        }
    }
    .map_err(num_err)?;
    match m.kind {
        Kind::Ss(_) => one(ss_value(&d, ts)),
        _ => {
            let (n, den) = nc::ss2tf(&d).map_err(num_err)?;
            one(tf_value(n, den, ts))
        }
    }
}

fn f_rlocus(i: &mut Interp, a: &[Value], nargout: usize) -> Out {
    arity(a, 1, 2, "rlocus")?;
    let m = parse(&a[0], "rlocus")?;
    let (num, den) = m.siso_tf()?;
    let ks: Vec<f64> = match a.get(1) {
        Some(v) => v.to_real()?.d,
        None => std::iter::once(0.0)
            .chain((0..300).map(|k| 10f64.powf(-3.0 + 6.0 * f64::from(k) / 299.0)))
            .collect(),
    };
    if ks.is_empty() || ks.len() > 20_000 || ks.iter().any(|k| !k.is_finite()) {
        return err(
            "Abaco:domain",
            "`rlocus`: the gains must be 1 to 20000 finite values",
        );
    }
    i.charge(ks.len() as u64 * 4)?;
    let n = den.len() - 1;
    let mut branches: Vec<Vec<Complex64>> = Vec::with_capacity(ks.len());
    for &k in &ks {
        let kn: Vec<f64> = num.iter().map(|c| c * k).collect();
        let poly = trim(&polyadd(&den, &kn, 1.0));
        let mut r = roots_of(&poly)?;
        while r.len() < n {
            r.push(Complex64::new(f64::INFINITY, 0.0));
        }
        branches.push(r);
    }
    // Order each set of roots to follow the previous one (nearest assignment).
    for k in 1..branches.len() {
        let prev = branches[k - 1].clone();
        let mut pool = branches[k].clone();
        let mut ordered = Vec::with_capacity(n);
        for p in &prev {
            if let Some((idx, _)) = pool
                .iter()
                .enumerate()
                .min_by(|x, y| (*x.1 - *p).norm().total_cmp(&(*y.1 - *p).norm()))
            {
                ordered.push(pool.remove(idx));
            }
        }
        branches[k] = ordered;
    }
    if nargout == 0 {
        for j in 0..n {
            let re: Vec<f64> = branches
                .iter()
                .filter_map(|b| b.get(j))
                .map(|z| z.re)
                .collect();
            let im: Vec<f64> = branches
                .iter()
                .filter_map(|b| b.get(j))
                .map(|z| z.im)
                .collect();
            super::call(i, "plot", &[Value::col(re), Value::col(im)], 0)?;
            super::call(i, "hold", &[Value::text("on")], 0)?;
        }
        super::call(i, "title", &[Value::text("Root Locus")], 0)?;
        super::call(i, "xlabel", &[Value::text("Real Axis")], 0)?;
        super::call(i, "ylabel", &[Value::text("Imaginary Axis")], 0)?;
        return Ok(vec![]);
    }
    let mut d = Vec::with_capacity(n * ks.len());
    for b in &branches {
        d.extend(b.iter().copied());
    }
    Ok(vec![Value::cplx(Arr::new(n, ks.len(), d)), Value::row(ks)])
}

pub const TABLE: &[(&str, Bf)] = &[
    ("c2d", f_c2d),
    ("care", f_care),
    ("ctrb", f_ctrb),
    ("damp", f_damp),
    ("dare", f_dare),
    ("dcgain", f_dcgain),
    ("dlqr", f_dlqr),
    ("dlyap", f_dlyap),
    ("evalfr", f_evalfr),
    ("feedback", f_feedback),
    ("freqresp", f_freqresp),
    ("impulse", f_impulse),
    ("initial", f_initial),
    ("isstable", f_isstable),
    ("lqr", f_lqr),
    ("lsim", f_lsim),
    ("lyap", f_lyap),
    ("margin", f_margin),
    ("nyquist", f_nyquist),
    ("obsv", f_obsv),
    ("parallel", f_parallel),
    ("pid", f_pid),
    ("place", f_place),
    ("pole", f_pole),
    ("pzmap", f_pzmap),
    ("rlocus", f_rlocus),
    ("series", f_series),
    ("ss", f_ss),
    ("ss2tf", f_ss2tf),
    ("ssdata", f_ssdata),
    ("step", f_step),
    ("stepinfo", f_stepinfo),
    ("tf", f_tf),
    ("tf2ss", f_tf2ss),
    ("tfdata", f_tfdata),
    ("zero", f_zero),
    ("zpk", f_zpk),
    ("zpkdata", f_zpkdata),
    ("bode", f_bode),
];
