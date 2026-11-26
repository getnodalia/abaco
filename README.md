# Ábaco

Ábaco is Nodalia's mathematics and scientific-computing application: executable maths documents in
which every number carries its unit and every cell recomputes when what it depends on changes. It is
written in Rust with engines of its own, and belongs to the Engineering suite with Forja, Torno,
Vortia and Circuito. Licence: `MIT OR Apache-2.0`.

## What runs today

Version 0.1 is the engine and a command line. There is no graphical app yet.

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

What does not exist yet (see [`ROADMAP.md`](ROADMAP.md)): the egui app, plots, symbolic computation,
statistics, control-systems and signal tools, optimisation, scripting beyond the expression language.
Nothing here is validated for safety-critical use.

## Build and test

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
cargo run -p abaco-cli -- eval '2 m + 30 cm -> m'
```

## Layout

| Path | What it is |
|---|---|
| `crates/nodalia-units` | Dimensions, units, quantities, constants. Shared with other Nodalia apps |
| `crates/nodalia-numerics` | Dense linear algebra, roots, quadrature, ODEs. Shared with other Nodalia apps |
| `crates/abaco-expr` | Lexer, parser and evaluator |
| `crates/abaco-doc` | The reactive document and the `.abaco` format |
| `apps/abaco-cli` | The `abaco` command |

Contributing and agent rules: [`AGENTS.md`](AGENTS.md). Security: [`SECURITY.md`](SECURITY.md).
