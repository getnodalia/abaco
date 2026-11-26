// SPDX-License-Identifier: MIT OR Apache-2.0
//! Linear time-invariant systems: transfer function ↔ state space, discretisation, time and
//! frequency response, controllability, pole placement, Lyapunov and Riccati equations, LQR.
//!
//! Continuous time is `x' = A x + B u`, `y = C x + D u`; discrete time is `x[k+1] = A x[k] + B u[k]`.
//! Polynomials are coefficient slices, **highest power first**.
//!
//! * Riccati equations: the continuous one by the matrix sign function (Roberts 1971, with norm
//!   scaling); the discrete one by the structure-preserving doubling algorithm (Chu, Fan & Lin 2005).
//! * Discretisation: exact zero-order hold through the block matrix exponential (Van Loan 1978),
//!   or the bilinear (Tustin) transform.
//! * Simulation: the exact response to a piecewise-linear input (first-order hold), again through a
//!   block exponential, so there is no integration error.
//! * Pole placement: Ackermann's formula for one input.

use crate::error::{NumError, Result};
use crate::lu::Lu;
use crate::matfun::expm;
use crate::matrix::Matrix;
use crate::qr::least_squares;
use crate::{eigen, roots};
use num_complex::Complex64;

/// The largest number of states the dense algorithms here accept.
pub const MAX_STATES: usize = 400;

/// A state-space model.
#[derive(Debug, Clone, PartialEq)]
pub struct StateSpace {
    /// `n × n`.
    pub a: Matrix<f64>,
    /// `n × m`.
    pub b: Matrix<f64>,
    /// `p × n`.
    pub c: Matrix<f64>,
    /// `p × m`.
    pub d: Matrix<f64>,
}

impl StateSpace {
    /// Checks the shapes agree and every entry is finite.
    pub fn new(a: Matrix<f64>, b: Matrix<f64>, c: Matrix<f64>, d: Matrix<f64>) -> Result<Self> {
        let n = a.rows();
        if !a.is_square()
            || b.rows() != n
            || c.cols() != n
            || d.rows() != c.rows()
            || d.cols() != b.cols()
        {
            return Err(NumError::DimensionMismatch(format!(
                "A {}x{}, B {}x{}, C {}x{}, D {}x{} do not fit together",
                a.rows(),
                a.cols(),
                b.rows(),
                b.cols(),
                c.rows(),
                c.cols(),
                d.rows(),
                d.cols()
            )));
        }
        if n > MAX_STATES {
            return Err(NumError::TooLarge(format!(
                "{n} states (the limit is {MAX_STATES})"
            )));
        }
        Ok(Self { a, b, c, d })
    }

    /// Number of states.
    pub fn states(&self) -> usize {
        self.a.rows()
    }
    /// Number of inputs.
    pub fn inputs(&self) -> usize {
        self.b.cols()
    }
    /// Number of outputs.
    pub fn outputs(&self) -> usize {
        self.c.rows()
    }
}

fn zeros(r: usize, c: usize) -> Result<Matrix<f64>> {
    Matrix::zeros(r, c)
}

/// Builds a matrix from blocks laid out in a grid (row-major list of block rows).
fn blocks(grid: &[Vec<&Matrix<f64>>]) -> Result<Matrix<f64>> {
    let rows: usize = grid.iter().map(|r| r.first().map_or(0, |m| m.rows())).sum();
    let cols: usize = grid.first().map_or(0, |r| r.iter().map(|m| m.cols()).sum());
    let mut out = zeros(rows, cols)?;
    let mut r0 = 0;
    for row in grid {
        let h = row.first().map_or(0, |m| m.rows());
        let mut c0 = 0;
        for m in row {
            if m.rows() != h {
                return Err(NumError::DimensionMismatch(
                    "block rows of different heights".into(),
                ));
            }
            for i in 0..m.rows() {
                for j in 0..m.cols() {
                    out.set(r0 + i, c0 + j, m.get(i, j).unwrap_or(0.0))?;
                }
            }
            c0 += m.cols();
        }
        r0 += h;
    }
    Ok(out)
}

fn inv(m: &Matrix<f64>) -> Result<Matrix<f64>> {
    if m.rows() == 0 {
        return Ok(m.clone());
    }
    Lu::factor(m)?.inverse()
}

fn eye(n: usize) -> Result<Matrix<f64>> {
    Matrix::identity(n)
}

fn trim_leading(p: &[f64]) -> &[f64] {
    let first = p
        .iter()
        .position(|&c| c != 0.0)
        .unwrap_or(p.len().saturating_sub(1));
    &p[first..]
}

/// Characteristic polynomial `det(sI − A)`, monic, from the eigenvalues.
pub fn char_poly(a: &Matrix<f64>) -> Result<Vec<f64>> {
    let eig = eigen::general(a)?;
    let mut p = vec![Complex64::new(1.0, 0.0)];
    for l in eig {
        let mut next = vec![Complex64::new(0.0, 0.0); p.len() + 1];
        for (i, c) in p.iter().enumerate() {
            next[i] += *c;
            next[i + 1] -= *c * l;
        }
        p = next;
    }
    let scale = p.iter().map(|c| c.norm()).fold(1.0, f64::max);
    if p.iter().any(|c| c.im.abs() > 1e-8 * scale) {
        return Err(NumError::InvalidInput(
            "the eigenvalues of a real matrix did not come in conjugate pairs".into(),
        ));
    }
    Ok(p.iter().map(|c| c.re).collect())
}

/// A proper transfer function `num/den` as a controllable-canonical state-space model.
pub fn tf2ss(num: &[f64], den: &[f64]) -> Result<StateSpace> {
    if num.iter().chain(den).any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("transfer function coefficient"));
    }
    let den = trim_leading(den);
    let num = trim_leading(num);
    if den.is_empty() || den[0] == 0.0 {
        return Err(NumError::InvalidInput("the denominator is zero".into()));
    }
    let n = den.len() - 1;
    if num.len() > den.len() {
        return Err(NumError::InvalidInput(
            "the transfer function is improper (the numerator degree exceeds the denominator's)"
                .into(),
        ));
    }
    let lead = den[0];
    let a_co: Vec<f64> = den.iter().map(|v| v / lead).collect();
    let mut nm = vec![0.0; den.len() - num.len()];
    nm.extend(num.iter().map(|v| v / lead));
    let d0 = nm[0];
    let rest: Vec<f64> = (1..=n).map(|i| nm[i] - d0 * a_co[i]).collect();
    let mut a = zeros(n, n)?;
    for j in 0..n {
        a.set(0, j, -a_co[j + 1])?;
    }
    for i in 1..n {
        a.set(i, i - 1, 1.0)?;
    }
    let mut b = zeros(n, 1)?;
    if n > 0 {
        b.set(0, 0, 1.0)?;
    }
    let c = Matrix::new(1, n, rest)?;
    let d = Matrix::new(1, 1, vec![d0])?;
    StateSpace::new(a, b, c, d)
}

/// The transfer function of a single-input single-output model: `(num, den)`, `den` monic.
pub fn ss2tf(ss: &StateSpace) -> Result<(Vec<f64>, Vec<f64>)> {
    if ss.inputs() != 1 || ss.outputs() != 1 {
        return Err(NumError::InvalidInput(
            "only single-input single-output models have one transfer function".into(),
        ));
    }
    let d = ss.d.get(0, 0).unwrap_or(0.0);
    if ss.states() == 0 {
        return Ok((vec![d], vec![1.0]));
    }
    let den = char_poly(&ss.a)?;
    let bc = ss.b.matmul(&ss.c)?;
    let closed = char_poly(&ss.a.sub(&bc)?)?;
    let mut num: Vec<f64> = closed
        .iter()
        .zip(&den)
        .map(|(c, p)| c + (d - 1.0) * p)
        .collect();
    let scale = num.iter().chain(&den).fold(0.0f64, |m, v| m.max(v.abs()));
    for v in &mut num {
        if v.abs() < 1e-12 * scale {
            *v = 0.0;
        }
    }
    let num = trim_leading(&num).to_vec();
    Ok((if num.is_empty() { vec![0.0] } else { num }, den))
}

/// Exact zero-order-hold discretisation with sample time `ts`.
pub fn c2d_zoh(ss: &StateSpace, ts: f64) -> Result<StateSpace> {
    check_ts(ts)?;
    let (n, m) = (ss.states(), ss.inputs());
    if n == 0 {
        return Ok(ss.clone());
    }
    let top = blocks(&[vec![&ss.a, &ss.b]])?;
    let bottom = zeros(m, n + m)?;
    let big = blocks(&[vec![&top], vec![&bottom]])?.scale(ts)?;
    let e = expm(&big)?;
    let mut ad = zeros(n, n)?;
    let mut bd = zeros(n, m)?;
    for i in 0..n {
        for j in 0..n {
            ad.set(i, j, e.get(i, j).unwrap_or(0.0))?;
        }
        for j in 0..m {
            bd.set(i, j, e.get(i, n + j).unwrap_or(0.0))?;
        }
    }
    StateSpace::new(ad, bd, ss.c.clone(), ss.d.clone())
}

/// The bilinear (Tustin) discretisation `s = (2/Ts)(z−1)/(z+1)`.
pub fn c2d_tustin(ss: &StateSpace, ts: f64) -> Result<StateSpace> {
    check_ts(ts)?;
    let n = ss.states();
    if n == 0 {
        return Ok(ss.clone());
    }
    let i = eye(n)?;
    let half = ss.a.scale(ts / 2.0)?;
    let minus = inv(&i.sub(&half)?)?;
    let ad = minus.matmul(&i.add(&half)?)?;
    // The symmetric form: both B and C carry sqrt(Ts), so the transfer function is the bilinear one.
    let root = ts.sqrt();
    let bd = minus.matmul(&ss.b)?.scale(root)?;
    let cd = ss.c.matmul(&minus)?.scale(root)?;
    let dd =
        ss.d.add(&ss.c.matmul(&minus)?.matmul(&ss.b)?.scale(ts / 2.0)?)?;
    StateSpace::new(ad, bd, cd, dd)
}

fn check_ts(ts: f64) -> Result<()> {
    if ts.is_finite() && ts > 0.0 {
        Ok(())
    } else {
        Err(NumError::InvalidInput(
            "the sample time must be positive and finite".into(),
        ))
    }
}

/// The controllability matrix `[B AB … Aⁿ⁻¹B]`.
pub fn ctrb(a: &Matrix<f64>, b: &Matrix<f64>) -> Result<Matrix<f64>> {
    let n = a.rows();
    if !a.is_square() || b.rows() != n {
        return Err(NumError::DimensionMismatch(
            "A must be square and B must have as many rows".into(),
        ));
    }
    let mut cols = Vec::new();
    let mut cur = b.clone();
    for _ in 0..n {
        cols.push(cur.clone());
        cur = a.matmul(&cur)?;
    }
    let refs: Vec<&Matrix<f64>> = cols.iter().collect();
    blocks(&[refs])
}

/// The observability matrix `[C; CA; …; CAⁿ⁻¹]`.
pub fn obsv(a: &Matrix<f64>, c: &Matrix<f64>) -> Result<Matrix<f64>> {
    let n = a.rows();
    if !a.is_square() || c.cols() != n {
        return Err(NumError::DimensionMismatch(
            "A must be square and C must have as many columns".into(),
        ));
    }
    let mut rows = Vec::new();
    let mut cur = c.clone();
    for _ in 0..n {
        rows.push(cur.clone());
        cur = cur.matmul(a)?;
    }
    let grid: Vec<Vec<&Matrix<f64>>> = rows.iter().map(|m| vec![m]).collect();
    blocks(&grid)
}

/// State feedback gain `K` (`u = −K x`) that puts the eigenvalues of `A − B K` at `poles`
/// (Ackermann's formula; one input). Complex poles must come with their conjugates.
pub fn place(a: &Matrix<f64>, b: &Matrix<f64>, poles: &[Complex64]) -> Result<Vec<f64>> {
    let n = a.rows();
    if !a.is_square() || b.rows() != n || b.cols() != 1 {
        return Err(NumError::InvalidInput(
            "place needs a square A and a single-column B".into(),
        ));
    }
    if poles.len() != n {
        return Err(NumError::DimensionMismatch(format!(
            "{n} poles are needed, got {}",
            poles.len()
        )));
    }
    let mut p = vec![Complex64::new(1.0, 0.0)];
    for &l in poles {
        let mut next = vec![Complex64::new(0.0, 0.0); p.len() + 1];
        for (i, c) in p.iter().enumerate() {
            next[i] += *c;
            next[i + 1] -= *c * l;
        }
        p = next;
    }
    let scale = p.iter().map(|c| c.norm()).fold(1.0, f64::max);
    if p.iter().any(|c| c.im.abs() > 1e-9 * scale) {
        return Err(NumError::InvalidInput(
            "complex poles must come in conjugate pairs".into(),
        ));
    }
    let coef: Vec<f64> = p.iter().map(|c| c.re).collect();
    let wc = ctrb(a, b)?;
    let rcond = Lu::factor(&wc).map(|l| l.rcond_estimate()).unwrap_or(0.0);
    if rcond < 1e-13 {
        return Err(NumError::Singular { column: 0 });
    }
    // phi(A) by Horner.
    let mut phi = zeros(n, n)?;
    for &c in &coef {
        phi = phi.matmul(a)?.add(&eye(n)?.scale(c)?)?;
    }
    // K = [0 … 0 1] Wc⁻¹ phi(A): solve Wcᵀ z = e_n, then K = zᵀ phi.
    let mut en = vec![0.0; n];
    if let Some(l) = en.last_mut() {
        *l = 1.0;
    }
    let z = Lu::factor(&wc.transpose())?.solve(&en)?;
    let zm = Matrix::new(1, n, z)?;
    Ok(zm.matmul(&phi)?.as_slice().to_vec())
}

/// Solves `A X + X Aᵀ + Q = 0` (continuous Lyapunov).
pub fn lyap(a: &Matrix<f64>, q: &Matrix<f64>) -> Result<Matrix<f64>> {
    kron_solve(a, q, false)
}

/// Solves `A X Aᵀ − X + Q = 0` (discrete Lyapunov).
pub fn dlyap(a: &Matrix<f64>, q: &Matrix<f64>) -> Result<Matrix<f64>> {
    kron_solve(a, q, true)
}

fn kron_solve(a: &Matrix<f64>, q: &Matrix<f64>, discrete: bool) -> Result<Matrix<f64>> {
    let n = a.rows();
    if !a.is_square() || q.rows() != n || q.cols() != n {
        return Err(NumError::DimensionMismatch(
            "A and Q must be square of the same size".into(),
        ));
    }
    if n > 24 {
        return Err(NumError::TooLarge(format!(
            "{n} states for a Lyapunov equation (the limit is 24)"
        )));
    }
    let nn = n * n;
    let mut m = zeros(nn, nn)?;
    // vec(X) is column-major: index (i, j) -> j*n + i.
    for i in 0..n {
        for j in 0..n {
            let row = j * n + i;
            for k in 0..n {
                if discrete {
                    // (A X Aᵀ)_{ij} = Σ_kl a_ik X_kl a_jl
                    for l in 0..n {
                        let v = a.get(i, k).unwrap_or(0.0) * a.get(j, l).unwrap_or(0.0);
                        m.add_at(row, l * n + k, v)?;
                    }
                } else {
                    // (A X)_{ij} = Σ_k a_ik X_kj ; (X Aᵀ)_{ij} = Σ_k X_ik a_jk
                    m.add_at(row, j * n + k, a.get(i, k).unwrap_or(0.0))?;
                    m.add_at(row, k * n + i, a.get(j, k).unwrap_or(0.0))?;
                }
            }
            if discrete {
                m.add_at(row, row, -1.0)?;
            }
        }
    }
    let mut rhs = vec![0.0; nn];
    for i in 0..n {
        for j in 0..n {
            rhs[j * n + i] = -q.get(i, j).unwrap_or(0.0);
        }
    }
    let x = Lu::factor(&m)?.solve(&rhs)?;
    let mut out = zeros(n, n)?;
    for i in 0..n {
        for j in 0..n {
            out.set(i, j, x[j * n + i])?;
        }
    }
    Ok(out)
}

fn symmetrise(m: &Matrix<f64>) -> Result<Matrix<f64>> {
    m.add(&m.transpose())?.scale(0.5)
}

fn riccati_check(res: &Matrix<f64>, scale: &Matrix<f64>) -> Result<()> {
    let s = scale.norm_frobenius().max(1.0);
    if res.norm_frobenius() > 1e-6 * s {
        return Err(NumError::NoConvergence {
            iterations: 0,
            residual: res.norm_frobenius() / s,
        });
    }
    Ok(())
}

/// The stabilising solution `S` of `Aᵀ S + S A − S B R⁻¹ Bᵀ S + Q = 0`.
pub fn care(
    a: &Matrix<f64>,
    b: &Matrix<f64>,
    q: &Matrix<f64>,
    r: &Matrix<f64>,
) -> Result<Matrix<f64>> {
    let n = a.rows();
    check_riccati(a, b, q, r)?;
    let g = b.matmul(&inv(r)?)?.matmul(&b.transpose())?;
    let h = blocks(&[
        vec![a, &g.scale(-1.0)?],
        vec![&q.scale(-1.0)?, &a.transpose().scale(-1.0)?],
    ])?;
    let mut z = h;
    for it in 0..200 {
        let zi = inv(&z)?;
        let c = (zi.norm_1() / z.norm_1()).sqrt();
        let next = z.scale(c)?.add(&zi.scale(1.0 / c)?)?.scale(0.5)?;
        let delta = next.sub(&z)?.norm_1();
        z = next;
        if delta <= 1e-13 * z.norm_1() {
            break;
        }
        if it == 199 {
            return Err(NumError::NoConvergence {
                iterations: 200,
                residual: delta,
            });
        }
    }
    let part = |r0: usize, c0: usize| -> Result<Matrix<f64>> {
        let mut m = zeros(n, n)?;
        for i in 0..n {
            for j in 0..n {
                m.set(i, j, z.get(r0 + i, c0 + j).unwrap_or(0.0))?;
            }
        }
        Ok(m)
    };
    let (w11, w12, w21, w22) = (part(0, 0)?, part(0, n)?, part(n, 0)?, part(n, n)?);
    let i = eye(n)?;
    let lhs = blocks(&[vec![&w12], vec![&w22.add(&i)?]])?;
    let rhs = blocks(&[vec![&w11.add(&i)?], vec![&w21]])?.scale(-1.0)?;
    let mut s = zeros(n, n)?;
    for j in 0..n {
        let col = rhs.column(j).unwrap_or_default();
        let x = least_squares(&lhs, &col)?.x;
        for (k, v) in x.iter().enumerate() {
            s.set(k, j, *v)?;
        }
    }
    let s = symmetrise(&s)?;
    let res = a
        .transpose()
        .matmul(&s)?
        .add(&s.matmul(a)?)?
        .sub(&s.matmul(&g)?.matmul(&s)?)?
        .add(q)?;
    riccati_check(&res, &s)?;
    Ok(s)
}

fn check_riccati(a: &Matrix<f64>, b: &Matrix<f64>, q: &Matrix<f64>, r: &Matrix<f64>) -> Result<()> {
    let (n, m) = (a.rows(), b.cols());
    if !a.is_square()
        || b.rows() != n
        || q.rows() != n
        || q.cols() != n
        || r.rows() != m
        || r.cols() != m
    {
        return Err(NumError::DimensionMismatch(
            "A, B, Q and R must have compatible sizes".into(),
        ));
    }
    if n == 0 || n > MAX_STATES {
        return Err(NumError::InvalidInput(format!(
            "the Riccati equation needs 1 to {MAX_STATES} states"
        )));
    }
    Ok(())
}

/// The stabilising solution `X` of the discrete Riccati equation
/// `X = Aᵀ X A − Aᵀ X B (R + Bᵀ X B)⁻¹ Bᵀ X A + Q`.
pub fn dare(
    a: &Matrix<f64>,
    b: &Matrix<f64>,
    q: &Matrix<f64>,
    r: &Matrix<f64>,
) -> Result<Matrix<f64>> {
    let n = a.rows();
    check_riccati(a, b, q, r)?;
    let mut ak = a.clone();
    let mut gk = b.matmul(&inv(r)?)?.matmul(&b.transpose())?;
    let mut hk = q.clone();
    let i = eye(n)?;
    for it in 0..100 {
        let w = inv(&i.add(&gk.matmul(&hk)?)?)?;
        let aw = ak.matmul(&w)?;
        let next_a = aw.matmul(&ak)?;
        let next_g = gk.add(&aw.matmul(&gk)?.matmul(&ak.transpose())?)?;
        let next_h = hk.add(&ak.transpose().matmul(&hk)?.matmul(&w)?.matmul(&ak)?)?;
        let delta = next_h.sub(&hk)?.norm_1();
        ak = next_a;
        gk = symmetrise(&next_g)?;
        hk = symmetrise(&next_h)?;
        if delta <= 1e-13 * hk.norm_1().max(1.0) {
            break;
        }
        if it == 99 {
            return Err(NumError::NoConvergence {
                iterations: 100,
                residual: delta,
            });
        }
    }
    let at = a.transpose();
    let xb = hk.matmul(b)?;
    let mid = inv(&r.add(&b.transpose().matmul(&xb)?)?)?;
    let res = at
        .matmul(&hk)?
        .matmul(a)?
        .sub(
            &at.matmul(&xb)?
                .matmul(&mid)?
                .matmul(&b.transpose())?
                .matmul(&hk)?
                .matmul(a)?,
        )?
        .add(q)?
        .sub(&hk)?;
    riccati_check(&res, &hk)?;
    Ok(hk)
}

/// The optimal gain, Riccati solution and closed-loop eigenvalues of the LQR problem
/// `min ∫ xᵀQx + uᵀRu` (or its discrete sum).
pub struct Lqr {
    /// `u = −K x`.
    pub k: Matrix<f64>,
    /// The Riccati solution.
    pub s: Matrix<f64>,
    /// Eigenvalues of `A − B K`.
    pub poles: Vec<Complex64>,
}

/// Linear-quadratic regulator for a continuous (`discrete == false`) or discrete system.
pub fn lqr(
    a: &Matrix<f64>,
    b: &Matrix<f64>,
    q: &Matrix<f64>,
    r: &Matrix<f64>,
    discrete: bool,
) -> Result<Lqr> {
    let bt = b.transpose();
    let (k, s) = if discrete {
        let x = dare(a, b, q, r)?;
        let k = inv(&r.add(&bt.matmul(&x)?.matmul(b)?)?)?
            .matmul(&bt)?
            .matmul(&x)?
            .matmul(a)?;
        (k, x)
    } else {
        let s = care(a, b, q, r)?;
        (inv(r)?.matmul(&bt)?.matmul(&s)?, s)
    };
    let poles = eigen::general(&a.sub(&b.matmul(&k)?)?)?;
    Ok(Lqr { k, s, poles })
}

/// Outputs and states of a simulation, one entry per sample.
pub type Response = (Vec<Vec<f64>>, Vec<Vec<f64>>);

/// The response of `ss` to the input samples `u[k]` (each a vector of `m` values) at the uniform
/// times `k·dt`, starting from `x0`. Continuous models assume the input is piecewise linear
/// between samples (exact); discrete models step once per sample (`dt` is then ignored).
/// Returns the outputs and the states, one entry per sample.
pub fn simulate(
    ss: &StateSpace,
    discrete: bool,
    dt: f64,
    u: &[Vec<f64>],
    x0: &[f64],
) -> Result<Response> {
    let (n, m, p) = (ss.states(), ss.inputs(), ss.outputs());
    if x0.len() != n {
        return Err(NumError::DimensionMismatch(format!(
            "x0 has {} values, the model has {n} states",
            x0.len()
        )));
    }
    if u.iter().any(|row| row.len() != m) {
        return Err(NumError::DimensionMismatch(format!(
            "every input sample needs {m} values"
        )));
    }
    if u.iter().flatten().chain(x0).any(|v| !v.is_finite()) {
        return Err(NumError::NonFinite("the input or the initial state"));
    }
    let (phi, g1, g2) = if discrete {
        (ss.a.clone(), ss.b.clone(), zeros(n, m)?)
    } else {
        check_ts(dt)?;
        if n == 0 {
            (zeros(0, 0)?, zeros(0, m)?, zeros(0, m)?)
        } else {
            let top = blocks(&[vec![&ss.a, &ss.b, &zeros(n, m)?]])?;
            let mid = blocks(&[vec![&zeros(m, n)?, &zeros(m, m)?, &eye(m)?]])?;
            let bot = zeros(m, n + 2 * m)?;
            let big = blocks(&[vec![&top], vec![&mid], vec![&bot]])?.scale(dt)?;
            let e = expm(&big)?;
            let cut = |c0: usize, w: usize| -> Result<Matrix<f64>> {
                let mut out = zeros(n, w)?;
                for i in 0..n {
                    for j in 0..w {
                        out.set(i, j, e.get(i, c0 + j).unwrap_or(0.0))?;
                    }
                }
                Ok(out)
            };
            (cut(0, n)?, cut(n, m)?, cut(n + m, m)?.scale(1.0 / dt)?)
        }
    };
    let mut x = x0.to_vec();
    let mut ys = Vec::with_capacity(u.len());
    let mut xs = Vec::with_capacity(u.len());
    for (k, uk) in u.iter().enumerate() {
        let mut y = ss.c.mul_vec(&x)?;
        let du = ss.d.mul_vec(uk)?;
        for (yi, di) in y.iter_mut().zip(&du) {
            *yi += di;
        }
        debug_assert_eq!(y.len(), p);
        ys.push(y);
        xs.push(x.clone());
        if k + 1 < u.len() {
            let mut nx = phi.mul_vec(&x)?;
            let t1 = g1.mul_vec(uk)?;
            let diff: Vec<f64> = if discrete {
                vec![0.0; m]
            } else {
                u[k + 1].iter().zip(uk).map(|(a, b)| a - b).collect()
            };
            let t2 = g2.mul_vec(&diff)?;
            for i in 0..n {
                nx[i] += t1[i] + t2[i];
            }
            if nx.iter().any(|v| !v.is_finite()) {
                return Err(NumError::NonFinite(
                    "the simulated state (the system is unstable or the span too long)",
                ));
            }
            x = nx;
        }
    }
    Ok((ys, xs))
}

/// The frequency response `C (sI − A)⁻¹ B + D` at the complex point `s` (`p × m`, row-major).
pub fn freqresp(ss: &StateSpace, s: Complex64) -> Result<Matrix<Complex64>> {
    let (n, m, p) = (ss.states(), ss.inputs(), ss.outputs());
    let mut out = vec![Complex64::new(0.0, 0.0); p * m];
    for i in 0..p {
        for j in 0..m {
            out[i * m + j] = Complex64::new(ss.d.get(i, j).unwrap_or(0.0), 0.0);
        }
    }
    if n > 0 {
        let mut w = vec![Complex64::new(0.0, 0.0); n * n];
        for i in 0..n {
            for j in 0..n {
                let mut v = Complex64::new(-ss.a.get(i, j).unwrap_or(0.0), 0.0);
                if i == j {
                    v += s;
                }
                w[i * n + j] = v;
            }
        }
        let lu = Lu::factor(&Matrix::new(n, n, w)?)?;
        for j in 0..m {
            let col: Vec<Complex64> = (0..n)
                .map(|i| Complex64::new(ss.b.get(i, j).unwrap_or(0.0), 0.0))
                .collect();
            let x = lu.solve(&col)?;
            for i in 0..p {
                let mut acc = Complex64::new(0.0, 0.0);
                for (k, xk) in x.iter().enumerate() {
                    acc += *xk * ss.c.get(i, k).unwrap_or(0.0);
                }
                out[i * m + j] += acc;
            }
        }
    }
    Matrix::new(p, m, out)
}

/// `num(s)/den(s)` at `s` by Horner's rule.
pub fn tf_eval(num: &[f64], den: &[f64], s: Complex64) -> Complex64 {
    let horner = |c: &[f64]| {
        c.iter()
            .fold(Complex64::new(0.0, 0.0), |acc, &k| acc * s + k)
    };
    horner(num) / horner(den)
}

/// Roots of a polynomial given highest power first; the zero polynomial or a constant has none.
pub fn poly_roots(c: &[f64]) -> Result<Vec<Complex64>> {
    let c = trim_leading(c);
    if c.len() <= 1 {
        return Ok(Vec::new());
    }
    roots::poly_roots(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat(rows: usize, cols: usize, d: &[f64]) -> Matrix<f64> {
        Matrix::new(rows, cols, d.to_vec()).unwrap()
    }

    #[test]
    fn tf2ss_and_back_round_trip() {
        let ss = tf2ss(&[1.0, 3.0], &[1.0, 3.0, 2.0]).unwrap();
        let (num, den) = ss2tf(&ss).unwrap();
        assert_eq!(den.len(), 3);
        assert!((den[1] - 3.0).abs() < 1e-9 && (den[2] - 2.0).abs() < 1e-9);
        assert!((num[0] - 1.0).abs() < 1e-9 && (num[1] - 3.0).abs() < 1e-9);
        // A direct feed-through term.
        let ss = tf2ss(&[2.0, 1.0], &[1.0, 1.0]).unwrap();
        assert!((ss.d.get(0, 0).unwrap() - 2.0).abs() < 1e-12);
        let (num, _) = ss2tf(&ss).unwrap();
        assert!((num[0] - 2.0).abs() < 1e-9 && (num[1] - 1.0).abs() < 1e-9);
        assert!(tf2ss(&[1.0, 0.0, 0.0], &[1.0, 1.0]).is_err());
    }

    #[test]
    fn first_order_step_response_is_exact() {
        // 1/(s+1): y(t) = 1 - e^-t, from a unit step sampled every 0.1.
        let ss = tf2ss(&[1.0], &[1.0, 1.0]).unwrap();
        let u = vec![vec![1.0]; 51];
        let (y, _) = simulate(&ss, false, 0.1, &u, &[0.0]).unwrap();
        for (k, yk) in y.iter().enumerate() {
            let t = k as f64 * 0.1;
            assert!((yk[0] - (1.0 - (-t).exp())).abs() < 1e-12, "k={k}");
        }
    }

    #[test]
    fn a_ramp_input_is_followed_exactly_by_the_hold() {
        // x' = -x + u with u = t: x(t) = t - 1 + e^-t.
        let ss = tf2ss(&[1.0], &[1.0, 1.0]).unwrap();
        let u: Vec<Vec<f64>> = (0..21).map(|k| vec![k as f64 * 0.25]).collect();
        let (y, _) = simulate(&ss, false, 0.25, &u, &[0.0]).unwrap();
        for (k, yk) in y.iter().enumerate() {
            let t = k as f64 * 0.25;
            assert!((yk[0] - (t - 1.0 + (-t).exp())).abs() < 1e-11);
        }
    }

    #[test]
    fn zoh_of_an_integrator_and_of_a_decay() {
        let ss = tf2ss(&[1.0], &[1.0, 0.0]).unwrap();
        let d = c2d_zoh(&ss, 0.5).unwrap();
        assert!((d.a.get(0, 0).unwrap() - 1.0).abs() < 1e-12);
        assert!((d.b.get(0, 0).unwrap() - 0.5).abs() < 1e-12);
        let ss = tf2ss(&[1.0], &[1.0, 2.0]).unwrap();
        let d = c2d_zoh(&ss, 0.1).unwrap();
        assert!((d.a.get(0, 0).unwrap() - (-0.2f64).exp()).abs() < 1e-12);
        assert!((d.b.get(0, 0).unwrap() - (1.0 - (-0.2f64).exp()) / 2.0).abs() < 1e-12);
    }

    #[test]
    fn tustin_preserves_the_dc_gain_and_maps_stable_to_stable() {
        let ss = tf2ss(&[10.0], &[1.0, 5.0]).unwrap();
        let d = c2d_tustin(&ss, 0.05).unwrap();
        let g = freqresp(&d, Complex64::new(1.0, 0.0))
            .unwrap()
            .get(0, 0)
            .unwrap();
        assert!((g.re - 2.0).abs() < 1e-9, "{g}");
        assert!(d.a.get(0, 0).unwrap().abs() < 1.0);
    }

    #[test]
    fn pole_placement_puts_the_poles_where_asked() {
        let a = mat(2, 2, &[0.0, 1.0, -2.0, -3.0]);
        let b = mat(2, 1, &[0.0, 1.0]);
        let k = place(
            &a,
            &b,
            &[Complex64::new(-5.0, 2.0), Complex64::new(-5.0, -2.0)],
        )
        .unwrap();
        let acl = a
            .sub(&b.matmul(&Matrix::new(1, 2, k).unwrap()).unwrap())
            .unwrap();
        let mut e = eigen::general(&acl).unwrap();
        e.sort_by(|p, q| p.im.total_cmp(&q.im));
        assert!(
            (e[0] - Complex64::new(-5.0, -2.0)).norm() < 1e-9
                && (e[1] - Complex64::new(-5.0, 2.0)).norm() < 1e-9
        );
        // An uncontrollable pair is refused.
        let a2 = mat(2, 2, &[1.0, 0.0, 0.0, 2.0]);
        assert!(place(
            &a2,
            &mat(2, 1, &[1.0, 0.0]),
            &[Complex64::new(-1.0, 0.0), Complex64::new(-2.0, 0.0)]
        )
        .is_err());
    }

    #[test]
    fn lyapunov_equations_satisfy_their_definitions() {
        let a = mat(2, 2, &[-1.0, 2.0, 0.0, -3.0]);
        let q = mat(2, 2, &[2.0, 0.5, 0.5, 1.0]);
        let x = lyap(&a, &q).unwrap();
        let r = a
            .matmul(&x)
            .unwrap()
            .add(&x.matmul(&a.transpose()).unwrap())
            .unwrap()
            .add(&q)
            .unwrap();
        assert!(r.norm_frobenius() < 1e-12);
        let ad = mat(2, 2, &[0.5, 0.1, 0.0, 0.3]);
        let x = dlyap(&ad, &q).unwrap();
        let r = ad
            .matmul(&x)
            .unwrap()
            .matmul(&ad.transpose())
            .unwrap()
            .sub(&x)
            .unwrap()
            .add(&q)
            .unwrap();
        assert!(r.norm_frobenius() < 1e-12);
    }

    #[test]
    fn scalar_lqr_matches_the_closed_form() {
        // x' = a x + u, q = 1, r = 1: S = a + sqrt(a^2+1), K = S.
        let a = 0.5;
        let sol = lqr(
            &mat(1, 1, &[a]),
            &mat(1, 1, &[1.0]),
            &mat(1, 1, &[1.0]),
            &mat(1, 1, &[1.0]),
            false,
        )
        .unwrap();
        let want = a + (a * a + 1.0f64).sqrt();
        assert!((sol.k.get(0, 0).unwrap() - want).abs() < 1e-9);
        assert!((sol.poles[0].re + (a * a + 1.0f64).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn double_integrator_lqr_gives_the_known_gains() {
        // x'' = u with Q = I, R = 1: K = [1, sqrt(3)] (a standard textbook result).
        let a = mat(2, 2, &[0.0, 1.0, 0.0, 0.0]);
        let b = mat(2, 1, &[0.0, 1.0]);
        let sol = lqr(
            &a,
            &b,
            &Matrix::identity(2).unwrap(),
            &mat(1, 1, &[1.0]),
            false,
        )
        .unwrap();
        assert!((sol.k.get(0, 0).unwrap() - 1.0).abs() < 1e-8, "{:?}", sol.k);
        assert!((sol.k.get(0, 1).unwrap() - 3f64.sqrt()).abs() < 1e-8);
        assert!(sol.poles.iter().all(|p| p.re < 0.0));
    }

    #[test]
    fn discrete_scalar_lqr_matches_the_closed_form() {
        // x+ = a x + b u, q = r = 1: X solves b²X² + (1-a²-b²... ) via the scalar DARE.
        let (a, b) = (1.2f64, 1.0f64);
        let sol = lqr(
            &mat(1, 1, &[a]),
            &mat(1, 1, &[b]),
            &mat(1, 1, &[1.0]),
            &mat(1, 1, &[1.0]),
            true,
        )
        .unwrap();
        let x = sol.s.get(0, 0).unwrap();
        // The scalar DARE residual.
        let res = a * a * x - (a * x * b).powi(2) / (1.0 + b * b * x) + 1.0 - x;
        assert!(res.abs() < 1e-9, "{res}");
        assert!(sol.poles[0].norm() < 1.0);
    }

    #[test]
    fn frequency_response_of_a_first_order_lag() {
        let ss = tf2ss(&[1.0], &[1.0, 1.0]).unwrap();
        let g = freqresp(&ss, Complex64::new(0.0, 1.0))
            .unwrap()
            .get(0, 0)
            .unwrap();
        assert!((g.norm() - 0.5f64.sqrt()).abs() < 1e-12);
        assert!((g.arg() + std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        let h = tf_eval(&[1.0], &[1.0, 1.0], Complex64::new(0.0, 1.0));
        assert!((h - g).norm() < 1e-12);
    }

    #[test]
    fn controllability_and_observability_matrices() {
        let a = mat(2, 2, &[0.0, 1.0, -2.0, -3.0]);
        let b = mat(2, 1, &[0.0, 1.0]);
        let w = ctrb(&a, &b).unwrap();
        assert_eq!(w.as_slice(), &[0.0, 1.0, 1.0, -3.0]);
        let o = obsv(&a, &mat(1, 2, &[1.0, 0.0])).unwrap();
        assert_eq!(o.as_slice(), &[1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn hostile_input_is_refused() {
        assert!(tf2ss(&[f64::NAN], &[1.0, 1.0]).is_err());
        assert!(tf2ss(&[1.0], &[0.0]).is_err());
        assert!(c2d_zoh(&tf2ss(&[1.0], &[1.0, 1.0]).unwrap(), -1.0).is_err());
        assert!(care(
            &mat(1, 1, &[1.0]),
            &mat(2, 1, &[1.0, 1.0]),
            &mat(1, 1, &[1.0]),
            &mat(1, 1, &[1.0])
        )
        .is_err());
        let ss = tf2ss(&[1.0], &[1.0, -1.0]).unwrap();
        assert!(simulate(&ss, false, 1.0, &vec![vec![1.0]; 5000], &[0.0]).is_err());
        assert!(simulate(&ss, false, 1.0, &[vec![1.0, 2.0]], &[0.0]).is_err());
    }
}
