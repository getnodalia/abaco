# Ábaco

Ábaco is Nodalia's mathematics and scientific-computing application: executable maths documents in
which every number carries its unit and every cell recomputes when what it depends on changes. It is
written in Rust with engines of its own, and belongs to the Engineering suite with Forja, Torno,
Vortia and Circuito. Licence: `MIT OR Apache-2.0`.

## What runs today

Version 0.1 is the engine, a command line and a first desktop app (`abaco-app`).

```console
$ abaco eval '10 kg * 9.81 m/s^2 -> N'
98.1 N
$ abaco eval '100 degC -> degF'
212 degF
$ abaco eval '5 m + 3 s'
error: cannot add m and s (length and time)
```

* **Units** (`nodalia-units`): dimensions, SI and common units, conversion, parsing (`9.81 m/s^2`),
  CODATA constants. A mismatch is an error that names both dimensions; overflow is an error, never
  infinity.
* **Expression language** (`abaco-expr`): numbers with units, complex numbers, vectors and matrices,
  user functions (lexical closures), and errors with line and column and suggestions for misspelt
  names. Every input is bounded: source size, nesting, call depth, element count and a step budget.
* **Maths documents** (`abaco-doc`): cells with a dependency graph, cycle detection, incremental
  evaluation, and the `.abaco` JSON file (versioned, deterministic, atomic save).
* **Numerics** (`nodalia-numerics`): dense real and complex linear algebra (LU with condition
  estimate, Cholesky, QR least squares, eigenvalues), root finding, adaptive quadrature and an
  adaptive ODE integrator, each tested against analytic or published values.
* **CLI** (`abaco`): `eval`, `run FILE`, `units list`, `units convert`, `--json`, `--digits N`.
* **Desktop app** (`abaco-app`, egui with `nodalia-theme`): a document of cells. A maths cell has its
  input (coloured as you type) with its result and unit beside it; an error shows in place with
  line, column and the offending line quoted. Add, duplicate, delete and reorder cells; an outline of
  the cells; a side panel with the constants, the units and a converter, and the document's settings
  (significant digits, evaluation order). Opens and saves `.abaco` files (`Ctrl+O`, `Ctrl+S`),
  `Ctrl+Enter` adds a cell below, and a first launch opens a worked example (a beam, mesh currents, a
  function). Evaluation runs on a worker thread, so typing never waits for it. English and Spanish,
  light and dark. The Linux menu is exported to the shell's top bar. It has been run on Linux only;
  it has not been run on macOS or Windows.

What does not exist yet (see [`ROADMAP.md`](ROADMAP.md)): undo and redo in the app, plots, symbolic computation,
statistics, control-systems and signal tools, optimisation, scripting beyond the expression language.
Nothing here is validated for safety-critical use.

## Install

No release has been published yet. A release is made by merging a version bump with its notes
(`docs/release-notes/vX.Y.Z.md`): the workflows in `.github/workflows/` then build these files and
attach them to the GitHub release, where `abaco-X.Y.Z-<os>-<arch>` names each of them.

| Platform | Files |
|---|---|
| Linux (x86_64) | `.AppImage`, `.deb`, `.rpm`, `.tar.gz` |
| Windows (x64) | `.msi` (for one account or for everyone) and a `-portable.zip` |
| macOS (universal) | `.dmg`, ad hoc signed and not notarized: open it with right click > Open |

Every package installs the app as `abaco` and the command line as `abaco-cli`. The scripts that
build them are in `packaging/`; the Linux ones have been run here, the macOS and Windows ones have
not been run yet. Windows builds are unsigned and macOS builds are not notarized unless signing secrets are set, so
SmartScreen and Gatekeeper warn the first time.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
cargo run -p abaco-cli -- eval '2 m + 30 cm -> m'
cargo run -p abaco-app                      # the app (needs libxkbcommon-x11-0 and libegl1 on Linux)
```

## Layout

| Path | What it is |
|---|---|
| `crates/nodalia-units` | Dimensions, units, quantities, constants. Shared with other Nodalia apps |
| `crates/nodalia-numerics` | Dense linear algebra, roots, quadrature, ODEs. Shared with other Nodalia apps |
| `crates/abaco-expr` | Lexer, parser and evaluator |
| `crates/abaco-doc` | The reactive document and the `.abaco` format |
| `apps/abaco-cli` | The `abaco` command |
| `apps/abaco-app` | The `abaco-app` window: view-model and engine thread in the library, drawing in the binary |

Contributing and agent rules: [`AGENTS.md`](AGENTS.md). Security: [`SECURITY.md`](SECURITY.md).
