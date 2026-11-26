# nodalia-units

Physical dimensions, units, quantities and CODATA 2018 constants, with dimensional analysis.
A shared Nodalia engine: no UI, no I/O, `#![forbid(unsafe_code)]`, no dependency but `thiserror`.

## Public API

`Dimension` holds the exponents of the seven SI base dimensions as exact rationals (`Ratio`), so
`sqrt(Hz)` is exact. `Unit::parse("kg*m/s^2")` parses a unit expression (`*`, `·`, space, `/`,
`^`, superscripts, parentheses, SI and binary prefixes, `µm`/`um`, `Ω`/`ohm`) and knows its
dimension and its exact SI scale. `Quantity` is an `f64` in coherent SI plus its `Dimension` and
the unit it is displayed in; `add`, `sub`, `mul`, `div`, `pow`, `sqrt`, `compare`, `to(&Unit)`,
`simplify()` and `format(sig_digits)` are all checked and return `UnitError` (never panic) with
messages such as `cannot add m and s (length and time)`. `constants::CONSTANTS` lists the
constants with value, unit and relative uncertainty; `registry::UNITS` and `PREFIXES` are the
tables behind the parser.

## Behaviour worth knowing

- An exact symbol wins over a prefix reading (`min` is the minute, `mi` the mile); units such as
  `ft`, `psi`, `h` take no prefix. `kg` is `k` + `g`. Names are case sensitive.
- Angles are dimensionless (`rad`, `sr`, `deg`, `rev`). **`rpm` is defined as 2π/60 rad/s**,
  i.e. `1 rpm -> Hz` is 0.1047: Hz and rad/s share a dimension here. Use `rev/min` for cycles.
- Temperatures: `K` and the `delta_degC`/`delta_degF` units are ordinary quantities. `degC`,
  `degF` (and `°C`, `°F`) are absolute points on an offset scale and are only allowed alone,
  never in a product, quotient or power (`degC/s` is rejected: write `K/s` or
  `delta_degC/s`). point − point is a difference in K; point ± difference is a point;
  point + point, `2 * 20 degC`, negation and powers of a point are errors. `degR` is a plain
  scale. A point cannot be converted to a `delta_` unit.
- Prefix `da` is deca; `µ`, `μ` and `u` all mean micro. `B` is the byte (8 bit), `bit`, `byte`
  accept SI and binary prefixes.
- Arithmetic whose result is not finite is an error, not `inf`.
- Constants are named `c`, `h_planck`, `hbar`, `q_e` (elementary charge), `k_B`, `N_A`, `G`,
  `g0`, `R`, `eps0`, `mu0` (2018 value), `sigma_SB`, `m_e`, `m_p`, `m_n`, `alpha`, `atm_std`;
  `h` and `e` are left for the hour and Euler's number. `atm` is a unit.

## Limits

Unit expressions: 256 bytes, 16 nested parentheses, 32 factors. Dimension exponents are `i16`
rationals; overflow is an error. Conversion factors are `f64` (the nearest double to the exact
definition), so round trips agree to about 1e-15 relative.

## Verified

Against exact definitions: 1 ft = 0.3048 m, 1 in = 0.0254 m, 1 lb = 0.45359237 kg, 1 psi =
6894.757293168… Pa, 1 kWh = 3.6e6 J, 1 hp = 745.69987158227022 W, 1 atm = 101325 Pa,
1 L = 1e-3 m³, 60 mph = 26.8224 m/s, 1 mmHg = 133.322387415 Pa; 20 °C = 293.15 K = 68 °F,
−40 °C = −40 °F; Stefan–Boltzmann computed from h, c, k_B; μ0·ε0·c² = 1 within the 2018
uncertainties. Property tests (proptest): dimension algebra identities, conversion round trips,
and "random strings never panic" for the parser and formatter.

Not covered: constants other than those listed; calendar units (month, year); auto-selected SI
prefixes for display.
