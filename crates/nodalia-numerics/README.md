# nodalia-numerics

The numerical engine shared by Nodalia's engineering apps (Ábaco, Circuito and, later, Forja, Torno
and Vortia). Pure Rust, no `unsafe`, no C or Fortran underneath. Every failure is an error with the
numbers that explain it; nothing returns a plausible-looking wrong answer.

| Module | What it does | Verified against |
|---|---|---|
| `matrix` | Dense real and complex matrices, products, norms, bounded allocation | Hand-computed products, overflow and shape-limit tests |
| `lu` | LU with partial pivoting, solve, determinant, inverse, Hager–Higham condition estimate | Known systems, Hilbert matrix (cond ≈ 1e10), property tests on residuals, complex systems |
| `cholesky` | Cholesky for symmetric positive-definite matrices | Known factors, indefinite and asymmetric inputs refused |
| `qr` | Householder QR and least squares, rank deficiency reported | Exact lines, the normal equations on noisy data |
| `eigen` | Symmetric Jacobi (values and vectors); general real matrices by Hessenberg + Francis QR | Closed-form tridiagonal spectrum, `A v = λ v`, orthonormality, trace and determinant on pseudo-random matrices |
| `svd` | One-sided Jacobi SVD (thin), rank, pseudo-inverse, null space | Known singular values, `U Σ Vᵀ = A`, `A⁺A = I`, `A·null(A) = 0` on pseudo-random and rank-deficient matrices |
| `matfun` | Matrix exponential (Padé 13 with scaling and squaring); eigenvectors of general real matrices by inverse iteration | Rotation generators, nilpotent and diagonal matrices, `A v = λ v` |
| `roots` | Bisection, Brent, Newton, polynomial roots via the companion matrix | √2, `cos x = x`, `(x−1)…(x−5)`, a pole is rejected as "not a root" |
| `quad` | Adaptive Gauss–Kronrod G7/K15, infinite limits | Polynomial exactness, Gaussian and Lorentzian integrals, an endpoint singularity, a divergent integral reports failure |
| `ode` | Dormand–Prince 5(4), adaptive steps, exact landing on output times, cancellation | `e^−t`, harmonic oscillator over a period, `t²`, blow-up reported |

## Limits, stated plainly

* Dense only. There are no sparse solvers yet; the circuit simulator will need them for large netlists.
* `eigen::general` returns eigenvalues only, not eigenvectors, and accuracy for defective matrices is
  limited to about `ε^(1/m)` for a Jordan block of size `m` (that is a property of the problem).
* `ode` is explicit: stiff problems will take very small steps and may hit the step budget. An implicit
  solver is on the roadmap, not here.
* Algorithms are implemented from the published papers and textbooks named in each module; no code was
  copied from another numerical library.

## Licence

`MIT OR Apache-2.0`.
