// SPDX-License-Identifier: MIT OR Apache-2.0
//! Resource limits for hostile input.

/// Bounds on what one parse and evaluation may use. The defaults suit documents; a host that
/// evaluates untrusted input may lower them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Longest accepted source text, in bytes.
    pub max_source: usize,
    /// Tallest expression tree. A long chain such as `a + b + c + ...` is as tall as it has
    /// terms.
    pub max_depth: u32,
    /// Deepest nesting of brackets, calls and prefix/power operators; bounds parser recursion
    /// and therefore native stack use.
    pub max_nesting: u32,
    /// Most elements in one vector or matrix (literals, ranges, `zeros`, results).
    pub max_elements: usize,
    /// Deepest nesting of user-function calls (recursion included).
    pub max_call_depth: usize,
    /// Deepest nesting of evaluator frames (tree depth plus user-function recursion). Keeps
    /// the native stack bounded.
    pub max_eval_depth: usize,
    /// Most evaluation steps (node visits plus element operations) per evaluation.
    pub max_steps: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_source: 100_000,
            max_depth: 200,
            max_nesting: 32,
            max_elements: 100_000,
            max_call_depth: 64,
            max_eval_depth: 400,
            max_steps: 5_000_000,
        }
    }
}
