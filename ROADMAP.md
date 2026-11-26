# Ábaco roadmap

Only what runs is described as existing. Maturity levels: planned, experimental, partially
functional, functional, validated (against analytic or published references), production-mature.

| Area | State |
|---|---|
| Units, quantities, constants | Functional; conversions checked against fixed reference values |
| Expression language (units, complex, vectors, matrices, functions) | Functional; fuzzed for panics |
| Maths document, dependency graph, `.abaco` v1 | Functional; incremental result equals from-scratch result on random edit sequences |
| Dense numerics (LU, Cholesky, QR, eigenvalues, roots, quadrature, ODE) | Functional; verified against closed forms (see the crate README) |
| Ábaco Script (matrix language: scripts, functions, control flow, cells, structs, handles, 350+ built-ins, plot recording) | Functional in the library and CLI (`abaco script`, `abaco repl`); verified against analytic results, fuzzed for panics, stack and step bounded; not yet in the app; 2-D arrays only |
| CLI | Functional on Linux; not tested on macOS or Windows |
| egui app with NDL | Partially functional: cells, results and errors in place, units panel, open and save, English and Spanish; Linux only so far; no undo, no plots |
| Plots | Recorded by the language (`plot`, `surf`, `hist`, …); drawing in the app is planned |
| Symbolic computation | Planned |
| Linear systems with sparse matrices, implicit ODE solvers | Planned (needed for stiff and large problems) |
| Statistics, regression | Basic statistics and `polyfit` in Ábaco Script; the rest planned |
| Optimisation, interpolation | Planned (next) |
| Control systems, signals and filters | Planned (next) |
| Block-diagram modelling and simulation (Simulink-class) | Planned |
| ODE/DAE solver suite (`ode45`, stiff solvers, events) | Planned; a single explicit solver exists in `nodalia-numerics` |
| File I/O (CSV, MAT-like) | Planned |

## Next

MATLAB and Simulink coverage is being built in increments (each ends in a pull request):

1. **Language and base library** (done): Ábaco Script, its interpreter, the function library, the
   `abaco script` and `abaco repl` commands.
2. **Scientific library**: signals and FFT, filters, control systems, optimisation, interpolation,
   statistics, the ODE/DAE suite, symbolic basics, file I/O.
3. **App**: command window, workspace browser, script editor, 2-D and 3-D plot drawing, undo and redo,
   English and Spanish.
4. **Block diagrams**: a Simulink-style modeller with its own solvers (sources, scopes, math,
   continuous and discrete blocks, transfer functions, state space, PID, subsystems).

Also: selection of several cells, drag to reorder, running on macOS and Windows; symbolic
differentiation and simplification.

## Known limitations

* Ábaco Script is two-dimensional (no N-d arrays or struct arrays), has no integer types (`int32` and the
  like only round and saturate; values are stored as doubles), and does not know units yet; complex
  eigenvalue problems and complex least squares are not supported; `svd` returns the economy-size
  factors; there are no multi-row character arrays and no file functions yet. See `docs/script.md`.
* `h` is the hour; Planck's constant is `h_planck`; `e` is Euler's number; variables shadow units.
* `1e` lexes as `1*e`; implicit multiplication binds tighter than `/` (`12/2x` is `12/(2x)`).
* Display precision defaults to 6 significant digits and affects display only.
* Unknown fields in `.abaco` files are ignored on load and dropped on save.
* Ill-conditioned solves produce a warning with the numbers, not an error.
* Debug-build timing for a 5,000-cell document is about 76 ms.
