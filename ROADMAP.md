# Ábaco roadmap

Only what runs is described as existing. Maturity levels: planned, experimental, partially
functional, functional, validated (against analytic or published references), production-mature.

| Area | State |
|---|---|
| Units, quantities, constants | Functional; conversions checked against fixed reference values |
| Expression language (units, complex, vectors, matrices, functions) | Functional; fuzzed for panics |
| Maths document, dependency graph, `.abaco` v1 | Functional; incremental result equals from-scratch result on random edit sequences |
| Dense numerics (LU, Cholesky, QR, eigenvalues, roots, quadrature, ODE) | Functional; verified against closed forms (see the crate README) |
| CLI | Functional on Linux; not tested on macOS or Windows |
| egui app with NDL | Partially functional: cells, results and errors in place, units panel, open and save, English and Spanish; Linux only so far; no undo, no plots |
| Plots | Planned |
| Symbolic computation | Planned |
| Linear systems with sparse matrices, implicit ODE solvers | Planned (needed for stiff and large problems) |
| Statistics, regression, optimisation | Planned |
| Control systems and signals | Planned |

## Next

1. App: undo and redo, selection of several cells, drag to reorder, running on macOS and Windows.
2. Plot cells.
3. Matrix operators (`\` for solve) and statistics functions in the language.
4. Symbolic differentiation and simplification.

## Known limitations

* `h` is the hour; Planck's constant is `h_planck`; `e` is Euler's number; variables shadow units.
* `1e` lexes as `1*e`; implicit multiplication binds tighter than `/` (`12/2x` is `12/(2x)`).
* Display precision defaults to 6 significant digits and affects display only.
* Unknown fields in `.abaco` files are ignored on load and dropped on save.
* Ill-conditioned solves produce a warning with the numbers, not an error.
* Debug-build timing for a 5,000-cell document is about 76 ms.
