# Ábaco Blocks

Ábaco Blocks is the block-diagram modeller of Ábaco, in the spirit of Simulink, written in Rust with
solvers of its own (`crates/abaco-blocks`). A diagram is a set of named blocks wired by lines. It
compiles to a checked, sorted model and runs with fixed or variable steps, exactly on discrete
sample times and on switching times, with zero-crossing detection for saturations, switches, relays
and comparisons. The `abaco sim` command runs a diagram from a JSON file:

```text
abaco sim examples/servo.json
abaco sim examples/servo.json --stop 3 --csv servo.csv
abaco sim model.json --json
```

## Blocks

* **Sources**: `Constant`, `Step`, `Ramp`, `Sine`, `Pulse`, `Wave` (square, sawtooth, triangle),
  `Chirp`, `Clock`, `Ground`, `Noise` (seeded, deterministic), `Table` (lookup by time, linear interpolation), `Inport`.
* **Sinks**: `Scope` (one or more inputs), `ToWorkspace`, `Display`, `Terminator`, `Stop`, `Outport`.
* **Math**: `Sum`, `Gain`, `MatrixGain`, `Product`, `Math` (abs, sign, sqrt, exp, log, trigonometric
  and hyperbolic functions…), `Power`, `Mod`, `Saturation`, `DeadZone`, `Bias`, `Rounding`, `MinMax`,
  `Relational`, `CompareConst`, `Logical`, `Switch`, `Relay`, `Quantizer`, `DotProduct`,
  `SumElements`.
* **Routing**: `Mux`, `Demux`, `Selector`, `Goto`/`From`, and `Subsystem` (a nested diagram with
  `Inport` and `Outport` blocks). Signals are scalars or vectors; widths are inferred and checked.
* **Continuous**: `Integrator` (with limits and a vector initial value), `Derivative`,
  `TransferFcn`, `ZeroPole`, `StateSpace`, `Pid` (with filtered derivative) and `TransportDelay`.
* **Discrete**: `UnitDelay`, `Delay`, `ZeroOrderHold`, `DiscreteIntegrator` (forward, backward and
  trapezoidal), `DiscreteTransferFcn`, `DiscreteStateSpace` and `DiscreteDerivative`. Each block has
  its own sample time and samples exactly at `t₀ + k·Ts`.

## Solvers

Fixed step: `euler`, `heun`, `bs3`, `rk4`. Variable step: `dopri5` (Dormand–Prince 5(4), the
default), `bs23` (Bogacki–Shampine 3(2)) and `rosenbrock23` (the stiff method of `ode23s`, with a
numerical Jacobian, up to 400 states). The step is controlled by `rtol` and `atol`; `step` and
`max_step` bound it. A run reports its steps, rejected steps, zero crossings and diagram
evaluations. Solver settings live in the model's `solver` object:

```json
{ "method": "dopri5", "start_time": 0.0, "stop_time": 10.0, "rtol": 0.001, "atol": 1e-6 }
```

## File format

A model is a JSON object (`name`, `solver`, `blocks`, `lines`). Each block has a unique `name`, a
`type` and its parameters; each line goes from `["block", output_port]` to `["block", input_port]`
with zero-based ports. A block may carry an optional editor `position`. Files are bounded (size,
blocks, signal width, steps and recorded points) and every problem is reported with the block
that causes it: an unwired input, two lines into one input, a width mismatch, a `From` without a
`Goto`, a non-finite parameter, a non-finite signal during the run and an **algebraic loop**
(with the blocks in it).

## Linearisation

`linearize(&compiled, t, &x, &u)` returns the state-space matrices `A, B, C, D` of a continuous
diagram by central differences about an operating point (time, continuous states, inport values), so
a nonlinear model can be analysed with the control functions of `nodalia-numerics`.

## What was verified

Against analytic or published references, with the tolerance in each test (`crates/abaco-blocks/tests`):
order of convergence of the fixed methods (Euler 1, Heun 2, BS3 3, RK4 4) on `x' = -x`; the
variable-step methods against their tolerance; the second-order step response in closed form;
`TransferFcn`, `ZeroPole` and `StateSpace` agreeing with each other; a stiff problem
(`y' = -10⁴(y - cos t)`) solved accurately and far cheaper by the stiff method than by the explicit
one; discrete blocks against their difference equations; exact switching of `Step`, `Pulse` and
`Square`; saturation corners located by bisection; relay hysteresis; a transport delay of a step;
a PID loop in closed form; a Van der Pol limit-cycle amplitude of 2.0086; seeded noise moments; and
linearisation of a pendulum. A property-based fuzz test runs random (including nonsensical) diagrams
and file text; 60,000 cases ran without a panic.

## The editor

The app's block workspace (View ▸ Block diagrams, `Ctrl+Shift+L`) edits the same JSON files.

* **Library**: the blocks listed above, grouped and searchable; clicking one adds it at the middle
  of the canvas with default parameters.
* **Canvas**: drag a block to move it (it snaps to a grid), drag from an output port to an input
  port to wire them (a new line replaces the one already on that input), click a line to select it,
  `Delete` removes the selection, scroll or drag the background to pan, `Ctrl`+scroll or pinch to
  zoom, and the fit button frames the whole diagram.
* **Inspector**: the selected block's name and parameters. Numbers, booleans, text and lists are
  edited as text and applied with `Enter`; a value the engine refuses (an unknown shape, a matrix
  that is not rectangular) is explained below the fields and nothing changes. With nothing
  selected, it shows the solver settings (method, start and stop time, steps, tolerances).
* **Run**: `F6` compiles and runs the diagram on a worker thread, `Shift+F6` stops it at the next
  step, and each `Scope`, `ToWorkspace` and `Outport` is plotted. An error names the block.
* Blocks without a saved position are laid out in columns from the sources; a feedback loop is cut
  where it closes, so the layout stays as wide as the longest forward path.

## Simulink files

Ábaco reads and writes `.mdl` (text) and `.slx` (the zip-and-XML package) with `abaco-formats`, and
converts them to and from its own blocks in `abaco_blocks::simulink`. The app opens them from the
Open dialog (a converted model has no Ábaco file yet, so Save asks for one and never overwrites
the original) and exports the diagram with *Export as a Simulink model*; the CLI does the same with
`abaco convert plant.slx plant.json`, `abaco convert servo.json servo.slx` and
`abaco sim plant.slx`.

* Block parameters are expressions: numbers, matrices, ranges, complex numbers, a few functions and
  variables. Variables come from a MAT-file (`--vars`; the app uses a `.mat` of the same name beside
  the model). A name that is defined nowhere is an error that says which.
* Sample time `-1` takes the model's fixed step, or fails when there is none. Solver names map to
  Ábaco's (`ode45`, `ode23`, `ode23s`, `ode1`–`ode4`); the others are approximated and a warning says so.
* **Strict by default.** A block, a parameter or a feature with no equivalent makes the conversion fail
  and lists every one (`--lenient` drops them instead, with a warning for each, and the lines that
  touched them). Exporting fails the same way for the blocks Simulink has no counterpart for in this
  version (Chirp, Table, Pid, Sawtooth and Triangle waves, DiscreteDerivative, Identity).
* Not carried: triggered, enabled and action subsystems, library links and masks, Stateflow, Simscape.
  A `Derivative` with an infinite coefficient is approximated and warns.
* Verified by round trips (Ábaco → `.mdl`/`.slx` → Ábaco gives the same blocks and the same
  simulation, for the worked servo example and a model of about 25 block kinds), by checking that the
  `.slx` is a valid zip with the package structure Simulink uses, and by importing a hand-written `.mdl`
  whose result is known (`e^-2`). **It has not been opened in MATLAB or Simulink**, which is not
  available here: an `.slx` written by Ábaco follows the published structure but is unproven against the
  real program.

## Not done yet

The editor has no undo and redo, no multiple selection, no copy and paste, no way to open a
subsystem to edit it (it shows how many blocks it holds; edit its JSON), no routing of lines around
blocks (they are curves), and a block cannot be dragged straight from the library. There are no Stateflow-style state machines, no
physical-network (Simscape-style) blocks, no code generation, no variable-size or integer/fixed-point
signals (everything is `f64`), no algebraic-loop solver (loops are reported, not solved), and no
conversion of the Simulink features listed above. The stiff solver is limited to 400 states. Nothing here is validated for
safety-critical use.
