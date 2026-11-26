# abaco-cli

The headless `abaco` command of Ábaco: evaluate an expression, run a `.abaco` document, convert
units. It uses the same engines as the app (`abaco-expr`, `abaco-doc`, `nodalia-units`), so a
script and the window always agree.

```text
abaco eval '10 kg * 9.81 m/s^2 -> N'      # 98.1 N
abaco run notebook.abaco                   # every cell, in page order
abaco units list                           # the units and constants that are known
abaco units convert '1 atm' kPa            # 101.325 kPa
```

## Options

| Option | Meaning |
|---|---|
| `--json` | Machine-readable output on stdout (keys are sorted, so the output is stable) |
| `--digits N` | Significant digits to show (default 6). Display only: results are never rounded |
| `--help`, `--version` | As named |

Options may appear anywhere on the line.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Everything evaluated |
| 1 | An expression, a cell or a conversion failed (the message says where: line and column) |
| 2 | Bad usage, or a file that cannot be used (missing, too big, not a `.abaco` document, newer than this version) |

`run` prints one line per cell, `[n] <source> = <value>`, and for a failing cell the error code,
the message and `at cell n, line l, column c`. A summary of how many cells failed goes to stderr.
Warnings (for example an ill-conditioned linear solve) are printed with the result and never
change the exit code.

## Behaviour that matters

- Every failure is an error with a message, never a panic. Expressions and files are hostile:
  the limits of `abaco-expr` and `abaco-doc` (source size, nesting depth, element count, steps,
  file size, cell count) apply, and a closed pipe (`abaco run x | head -1`) ends quietly.
- `run` never modifies the file.

## Tested

`cargo test -p abaco-cli`: unit tests for the argument parser and integration tests that run the
real binary (output text, JSON shape, exit codes, error positions, hostile inputs, unusable files,
closed pipe, unit conversions checked against known factors: `1 atm = 101.325 kPa`,
`100 degC = 212 degF`, `1 kWh = 3.6e6 J`).

Not tested: other platforms than Linux, terminals with unusual encodings.

Licence: `MIT OR Apache-2.0`.
