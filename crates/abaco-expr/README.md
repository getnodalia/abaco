# abaco-expr

The expression language of Ábaco: a hand-written lexer and precedence-climbing parser, an AST
whose every node carries its source span, and an evaluator over real, complex, quantity
(a number with a unit), boolean, string, vector and matrix values. It depends on
`nodalia-units` for dimensions and units and on `nodalia-numerics` for `det`, `inv` and
`solve`. Nothing here touches a window, a file or the network.

```text
10 kg * 9.81 m/s^2 -> N          98.1 N
sin(30 deg)                       0.5
f(x, y) = x^2 + y;  f(3, 1)       10
A = [1, 2; 3, 4];  A[2, :]        [3, 4]
sqrt(-4 + 0i)                     2i
25 degC -> degF                   77 degF
```

## Public API

`parse(src)` returns a `Program` (statements separated by `;` or newlines; each is
`Assign`, `FuncDef` or `Expr`). `eval_str(src, &mut Env)` parses and evaluates a cell and returns
an `Evaluation` (`value` of the last statement, the `defined` names, `warnings`, `steps`);
`eval_program` and `eval_str_with` take explicit `Limits`. `Env` holds variables and user
functions (`set_var`, `var`, `set_fn`, `func`, `remove`). `Program::used_names()` and
`defined_names()` give the free and defined names of a cell, which is what a document needs
for its dependency graph. `Value` has `display(sig)` (a one-line rendering with `sig`
significant digits) and `to_json(sig)`. `ExprError` carries a `kind`, a message and a byte
`span`; `line_col(src)` gives the 1-based line and column (in characters) and `render(src)`
draws the source line with a caret underline. A failed cell is all-or-nothing: definitions
made by its earlier statements are undone.

## Language summary

* Numbers: `1`, `1.5e-3`, `.5`, `1_000`, imaginary `3i`. Strings `"..."`. Comments from `#`.
* Units by juxtaposition: `9.81 m/s^2`, `(2+3) m`, `25 degC`. Conversion with `expr -> unit` or
  `expr to unit`. See `nodalia-units` for the unit grammar and temperature rules.
* Precedence, loosest to tightest: `->`/`to`, `or`, `and`, `not`, comparisons, ranges `a:b`,
  `+ -`, `* / .* ./`, implicit multiplication, unary minus, `^ .^` (right associative, so
  `-2^2 = -4` and `2^3^2 = 512`). Implicit multiplication binds tighter than `/`:
  `12/2x` is `12/(2x)`, and `5 m / 2 s` is `2.5 m/s`.
* Names resolve as: parameter or variable, then `pi`, `e`, `tau`, `i`, `j`, then physical
  constants (`c`, `g0`, `k_B`, `N_A`, `h_planck`, ...), then units (`h` is the hour; Planck's
  constant is `h_planck`). A variable therefore shadows a unit or constant of the same name.
  `x(3)` with `x` a variable multiplies. `1e` is `1*e`, not an exponent.
* `if(cond, a, b)` evaluates only the branch it takes and needs a boolean. `and`/`or`
  short-circuit and need booleans.
* Vectors `[1, 2, 3]` are columns for matrix products; matrices `[1, 2; 3, 4]`. Indexing is
  1-based: `v[2]`, `A[1, 2]`, `A[2, :]`, `v[2:3]`. `*` between two vectors is an error (use
  `dot` or `.*`). Ranges `1:10`, `1:2:10` include the stop value; a range of quantities needs
  an explicit step.
* User functions `f(x, y) = ...` are closures over the names visible when they are defined; they
  never see the caller's variables. Recursion works up to `max_call_depth`. Errors inside a
  body are reported at the call, prefixed with ``in `f`:``.
* Built-ins: trigonometric, inverse and hyperbolic (a plain number is radians; `30 deg` works;
  inverse functions return radians), `exp ln log log10 log2 sqrt cbrt abs sign floor ceil round
  trunc mod hypot atan2`, `gamma factorial`, `re im conj arg complex`, `min max sum prod mean
  median var std` (sample statistics, divide by n-1), `linspace logspace range size length
  zeros ones eye`, `dot cross norm transpose det inv solve`. `sqrt` of a quantity halves the
  dimension; `exp`, `ln` and the trigonometric functions reject dimensioned arguments.
  Real functions never silently return a complex number: `sqrt(-4)` is a domain error and
  `sqrt(-4 + 0i)` is `2i`. Complex numbers cannot carry units.
* A result that overflows `f64` is an error, never `inf`. `solve` and `inv` report a singular
  matrix, and `solve` adds a warning when the system is ill-conditioned.

## Limits (hostile input)

`Limits` bounds one parse and evaluation. Defaults: source 100,000 bytes; expression height 200
(a chain `a + b + ...` is as tall as it has terms); nesting of brackets, calls and prefix or power
operators 32; vector or matrix elements 100,000; user-function call depth 64; evaluator depth
400; evaluation steps 5,000,000 (node visits plus element operations). Hitting a limit gives an
`ErrorKind::Limit` error. The deepest accepted input runs in a 1 MB thread stack even in a debug
build, which the tests check. There is no `unsafe` code and no panicking indexing or `unwrap` in
non-test code.

## What is verified

Run `cargo test -p abaco-expr`. `tests/language.rs` checks, to the stated tolerance, against
hand-computed or published values: the precedence table; `sin`, `cos`, `tan`, `exp`, `ln`,
`sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` at 1e-14 or better; `gamma(0.5) = sqrt(pi)`,
`gamma(5) = 24`, `gamma(10.5)`, `factorial(10)` and `factorial(170)`; Euler's identity; complex
products, quotients, `conj`, `arg`; `E = m c^2` for 1 kg; unit conversions (mile, atmosphere, kWh,
horsepower, inch, mph, temperature points); `det`, `inv` and `solve` of a 3x3 tridiagonal
matrix against its closed form; sample variance and standard deviation; error positions as
line and column. `tests/hostile.rs` runs property tests over random text, random bytes, token
soup and grammar-generated expressions: nothing may panic, error spans must lie inside the
source on character boundaries, evaluation must be deterministic and bounded in time; it also
checks that runaway work (huge matrix products, exponential recursion) stops at the step budget.

Not tested here: behaviour on 32-bit targets, and floating-point results beyond the tolerances
above (the `inv` and `solve` checks use 1e-12).
