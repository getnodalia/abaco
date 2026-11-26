# Ábaco roadmap

Only what runs is described as existing. Maturity levels: planned, experimental, partially
functional, functional, validated (against analytic or published references), production-mature.

| Area | State |
|---|---|
| Units, quantities, constants | Functional; conversions checked against fixed reference values |
| Expression language (units, complex, vectors, matrices, functions) | Functional; fuzzed for panics |
| Maths document, dependency graph, `.abaco` v1 | Functional; incremental result equals from-scratch result on random edit sequences |
| Dense numerics (LU, Cholesky, QR, eigenvalues, roots, quadrature, ODE) | Functional; verified against closed forms (see the crate README) |
| Ábaco Script (matrix language: scripts, functions, control flow, cells, structs, handles, 350+ built-ins, plot recording) | Functional in the library and CLI (`abaco script`, `abaco repl`); verified against analytic results, fuzzed for panics, stack and step bounded; also in the app's script workspace; 2-D arrays only |
| CLI | Functional on Linux; not tested on macOS or Windows |
| egui app with NDL | Partially functional: cells, results and errors in place, units panel, open and save, a script workspace (editor, command window with history, variable browser, figures), English and Spanish; Linux only so far; no undo in the document |
| Plots | Recorded by the language (`plot`, `surf`, `hist`, …) and drawn in the app (2-D with ticks, grid, legend and log axes; 3-D surfaces and curves with an orbit camera); no export to PNG/SVG yet; drawn on the CPU-side of egui, checked by eye in a software-rendered window, not against reference images |
| Symbolic computation | Planned |
| Linear systems with sparse matrices, implicit ODE solvers | Planned (needed for stiff and large problems) |
| Statistics, distributions, special functions | Functional in Ábaco Script (moments, quantiles, moving windows, normal/t/χ²/F/binomial/Poisson, incomplete gamma and beta); regression beyond `polyfit` planned |
| Interpolation | Functional (`interp1`, `interp2`, splines, PCHIP, piecewise polynomials) |
| Optimisation | Functional in Ábaco Script (Nelder–Mead, BFGS, Brent, Levenberg–Marquardt, simplex `linprog`); no constrained nonlinear solvers or quadratic programming yet |
| Signals and filters | Functional (FFT of any length, `filter`, `filtfilt`, Butterworth/Chebyshev/FIR design, windows, spectra helpers) |
| Control systems (`tf`/`zpk`/`ss`, step/bode/nyquist/margin/rlocus, `place`, `lqr`, Riccati and Lyapunov, `c2d`) | Functional in the library (`nodalia-numerics::control`) and in Ábaco Script; verified against closed forms (first-order step, second-order overshoot, gain margin 6 of 1/(s(s+1)(s+2)), LQR of a double integrator, ZOH, Riccati/Lyapunov residuals); SISO interconnection only, no operator overloading |
| Block-diagram modelling and simulation (Simulink-class) | Functional in the library and CLI (`abaco-blocks`, `abaco sim`): about 60 blocks, fixed and variable step solvers including a stiff one, exact discrete and switching events, zero-crossing detection, subsystems, linearisation; verified against analytic solutions and fuzzed; no graphical editor in the app yet, no Stateflow, physical networks, code generation or `.slx` import (`docs/blocks.md`) |
| ODE/DAE solver suite (`ode45`, stiff solvers, events) | Partially functional: Dormand–Prince 5(4) and Rosenbrock (2,3) with dense output, events, backward integration and a constant mass matrix (index-1 DAEs); no variable-order BDF/Adams, no `ode15i`, no delay equations |
| File I/O (CSV, MAT-like) | Planned |

## Next

MATLAB and Simulink coverage is being built in increments (each ends in a pull request):

1. **Language and base library** (done): Ábaco Script, its interpreter, the function library, the
   `abaco script` and `abaco repl` commands.
2. **Scientific library**: signals and FFT, filters, control systems, optimisation, interpolation,
   statistics, the ODE/DAE suite, symbolic basics, file I/O.
3. **App** (script workspace done: command window, variable browser, script editor, 2-D and 3-D plot
   drawing, English and Spanish; undo and redo in the document and figure export still open).
4. **Block diagrams**: a Simulink-style modeller with its own solvers (engine and `abaco sim` done;
   the graphical editor in the app is next).

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
