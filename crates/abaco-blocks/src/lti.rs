// SPDX-License-Identifier: MIT OR Apache-2.0
//! Linearisation: the state-space model of a diagram about an operating point.

use nodalia_numerics::control::StateSpace;
use nodalia_numerics::Matrix;

use crate::compile::Compiled;
use crate::error::{BlockError, Result};

/// Linearises the diagram from its top-level `Inport` blocks to its `Outport` blocks about
/// `(t, xc, u)` by central differences, returning `x' = A x + B u`, `y = C x + D u`.
///
/// The diagram must be continuous (no discrete states). The perturbation is `1e-6` relative with
/// a floor, so the model should be smooth around the operating point.
pub fn linearize(c: &Compiled, t: f64, xc: &[f64], u: &[f64]) -> Result<StateSpace> {
    if c.n_xd != 0 {
        return Err(BlockError::Solver(
            "linearisation needs a continuous diagram; this one has discrete states".into(),
        ));
    }
    let (n, m, p) = (c.n_xc, c.input_count(), c.output_width());
    if m == 0 || p == 0 {
        return Err(BlockError::Solver(
            "linearisation needs at least one Inport and one Outport block at the top level".into(),
        ));
    }
    let mut u0 = u.to_vec();
    u0.resize(m, 0.0);
    let mut a = vec![0.0; n * n];
    let mut b = vec![0.0; n * m];
    let mut cm = vec![0.0; p * n];
    let mut d = vec![0.0; p * m];
    let none: Vec<f64> = Vec::new();
    for j in 0..n {
        let xj = xc.get(j).copied().unwrap_or(0.0);
        let dl = 1e-6 * xj.abs().max(1.0);
        let (mut xp, mut xm) = (xc.to_vec(), xc.to_vec());
        if let (Some(p_), Some(m_)) = (xp.get_mut(j), xm.get_mut(j)) {
            *p_ = xj + dl;
            *m_ = xj - dl;
        }
        let (fp, yp) = c.evaluate(t, &xp, &none, &u0)?;
        let (fm, ym) = c.evaluate(t, &xm, &none, &u0)?;
        for i in 0..n {
            if let Some(v) = a.get_mut(i * n + j) {
                *v = (fp.get(i).copied().unwrap_or(0.0) - fm.get(i).copied().unwrap_or(0.0))
                    / (2.0 * dl);
            }
        }
        for i in 0..p {
            if let Some(v) = cm.get_mut(i * n + j) {
                *v = (yp.get(i).copied().unwrap_or(0.0) - ym.get(i).copied().unwrap_or(0.0))
                    / (2.0 * dl);
            }
        }
    }
    for j in 0..m {
        let uj = u0.get(j).copied().unwrap_or(0.0);
        let dl = 1e-6 * uj.abs().max(1.0);
        let (mut up, mut um) = (u0.clone(), u0.clone());
        if let (Some(p_), Some(m_)) = (up.get_mut(j), um.get_mut(j)) {
            *p_ = uj + dl;
            *m_ = uj - dl;
        }
        let (fp, yp) = c.evaluate(t, xc, &none, &up)?;
        let (fm, ym) = c.evaluate(t, xc, &none, &um)?;
        for i in 0..n {
            if let Some(v) = b.get_mut(i * m + j) {
                *v = (fp.get(i).copied().unwrap_or(0.0) - fm.get(i).copied().unwrap_or(0.0))
                    / (2.0 * dl);
            }
        }
        for i in 0..p {
            if let Some(v) = d.get_mut(i * m + j) {
                *v = (yp.get(i).copied().unwrap_or(0.0) - ym.get(i).copied().unwrap_or(0.0))
                    / (2.0 * dl);
            }
        }
    }
    let wrap = |e: nodalia_numerics::NumError| BlockError::Solver(e.to_string());
    StateSpace::new(
        Matrix::new(n, n, a).map_err(wrap)?,
        Matrix::new(n, m, b).map_err(wrap)?,
        Matrix::new(p, n, cm).map_err(wrap)?,
        Matrix::new(p, m, d).map_err(wrap)?,
    )
    .map_err(wrap)
}
