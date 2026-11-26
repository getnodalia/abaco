# Ábaco Script

Ábaco Script is the matrix language of Ábaco, written in Rust with no external numerical engine. It
follows MATLAB's syntax and semantics closely, so most MATLAB code for the supported subset runs
unchanged. It lives in `crates/abaco-script`; the `abaco script`, `abaco repl` and `abaco functions`
commands expose it.

```text
abaco script --code 'A = [4 -2; 1 1]; disp(eig(A)'')'
abaco script analysis.m
abaco repl
abaco functions
```

The units-aware expression language of maths documents (`abaco-expr`) is a separate language; the two
coexist. Units inside scripts are planned.

## Language

* Matrices and column-major 2-D arrays of real or complex numbers, logicals, character rows, cell arrays
  and scalar structs; function handles (`@sin`, `@(x) x.^2` capturing the variables it uses).
* Operators: `+ - * / \ ^ .* ./ .\ .^ ' .'`, comparisons, `& | ~ && ||`, ranges `a:s:b`, implicit
  expansion (broadcasting) for element-wise operations.
* Indexing and assignment with MATLAB's rules: `A(i, j)`, `A(:)`, `A(end+1) = x`, logical masks,
  deletion `A(2, :) = []`, growth on assignment, `c{k}`, `s.field`, `s.(name)`, `[a, b] = f(x)`,
  `[~, k] = max(v)`.
* `if/elseif/else`, `for`, `while`, `switch` (including cell cases), `break`, `continue`, `return`,
  `try/catch err`, `global`; `%` and `#` comments, `%{ ... %}` blocks, `...` continuation, command
  syntax (`hold on`, `format long`, `close all`).
* Functions defined in a script (`function [a, b] = f(x, varargin)`), recursion, `nargin`, `nargout`,
  `varargin`, `varargout`.
* Errors carry an identifier and a message that says what to do, and `catch err` gives
  `err.message`, `err.identifier` and `err.stack`.

## Library (more than 430 functions; `abaco functions` lists them)

| Area | Functions |
|---|---|
| Creation | `zeros ones eye rand randn randi randperm rng linspace logspace colon meshgrid ndgrid magic toeplitz vander hilb diag triu tril kron repmat reshape cat horzcat vertcat circshift flip* rot90` |
| Elementwise maths | trigonometric and hyperbolic (and inverses, degree variants), `exp log log2 log10 sqrt abs sign floor ceil round fix mod rem hypot atan2 nthroot gamma lgamma erf erfc factorial nchoosek gcd lcm isprime primes factor`, complex parts |
| Reductions | `sum prod mean median mode var std range min max cumsum cumprod cummax cummin diff dot cross trapz cumtrapz any all find nnz sort sortrows unique histc cov corrcoef prctile` |
| Linear algebra | `det inv \ linsolve trace rank pinv null orth cond rcond norm svd eig chol lu qr expm issymmetric` |
| Polynomials | `roots poly polyval polyfit polyder polyint conv deconv` |
| Fourier and filtering | `fft ifft fft2 ifft2 fftshift ifftshift nextpow2 filter filtfilt freqz xcorr findpeaks detrend unwrap sinc` |
| Filter design and windows | `butter cheby1 fir1 hamming hann bartlett blackman kaiser rectwin` |
| Interpolation | `interp1 interp2 spline pchip ppval mkpp` |
| Statistics | `skewness kurtosis quantile iqr mad rms zscore rescale bounds corr histcounts movmean movsum movmedian movmax movmin movstd` |
| Distributions and special functions | `normpdf normcdf norminv normrnd exppdf expcdf expinv unifpdf unifcdf chi2pdf chi2cdf tpdf tcdf fcdf binopdf binocdf poisspdf poisscdf erfinv erfcinv gammainc betainc beta betaln` |
| Calculus | `integral quad quadgk integral2 fzero` |
| Optimisation | `fminsearch fminunc fminbnd fsolve lsqnonlin lsqcurvefit linprog optimset optimget` |
| Differential equations | `ode45 ode23 ode113 ode23s ode15s ode23t odeset odeget` |
| Control systems | `tf zpk ss tfdata zpkdata ssdata tf2ss ss2tf pole zero pzmap damp dcgain isstable evalfr freqresp series parallel feedback pid c2d ctrb obsv step impulse initial lsim stepinfo bode nyquist margin rlocus place lqr dlqr care dare lyap dlyap` |
| Text | `sprintf printf fprintf disp display num2str int2str mat2str str2double str2num upper lower strtrim strrep strsplit strjoin strcat strtok strcmp strcmpi strncmp contains startsWith endsWith strfind regexp regexpi regexprep dec2bin bin2dec dec2hex hex2dec` |
| Cells and structs | `cell struct fieldnames isfield rmfield getfield setfield struct2cell cell2struct num2cell cell2mat deal cellfun arrayfun structfun isequal isequaln ismember union intersect setdiff setxor` |
| Files | `readmatrix writematrix csvread csvwrite dlmread dlmwrite fileread isfile save load` |
| Indices | `sub2ind ind2sub` |
| System | `error warning assert rethrow MException lasterr tic toc clock now cputime eval evalc exist isvarname who format bitand bitor bitxor bitshift intmax intmin` |
| Plots (recorded) | `plot plot3 scatter stem stairs bar hist semilogx semilogy loglog surf mesh contour imagesc fplot title xlabel ylabel zlabel legend grid axis xlim ylim hold figure close clf cla subplot` |

Plot commands do not draw: they record series and surfaces in `Interp::figures` and emit an
`Event::Figure`. The app draws them; the CLI ignores them.

## Safety

Scripts are untrusted input. Every run is bounded and stops with an error, never a crash:

* source size, expression height (300), bracket nesting, native stack use, call depth (256), steps
  (200 million), output (4 MB) and array size (20 million elements) are limits (`Limits`);
* a run can be cancelled from another thread (`Interp::cancel`);
* hosts that run scripts on a thread with a big stack raise `Limits::max_stack` (the CLI does);
* reading and writing files is off unless the host allows it (`Limits::files`; the app and the
  CLI do, with the person's own scripts). A file read is capped at 64 MB, a file written is replaced
  atomically, and relative paths are relative to the folder the program was started in;
* the test suite fuzzes the parser with arbitrary text, calls every built-in with junk arguments and
  checks that deep expressions end in a limit error on a 2 MB stack.

## Verification

Numerical functions are checked against closed forms and published values (see
`crates/abaco-script/tests/library.rs`): known linear systems, `Q R = A`, `P A = L U`, `U S Vᵀ = A`,
`A V = V D`, `expm` of a rotation generator, FFT against the definition for every length up to 40,
Butterworth coefficients against `scipy.signal.butter`, spline exactness on cubics, distributions against
table values (`crates/abaco-script/tests/science.rs`), the Rosenbrock function, Robertson's stiff
chemistry (`tests/optode.rs`), a textbook linear program, a falling ball stopped by an event, magic squares of every order class, `integral` of
`e^(−x²)` over the real line, `fzero` of `cos x = x`, special-function values, and printf formatting.

## Limits, stated plainly

* Arrays are two-dimensional. There are no N-d arrays and no struct arrays (a struct is scalar).
* There are no integer types: `int8 … uint64` round and saturate and `single` rounds to 32 bits, but
  values are stored and displayed as doubles. `intmax`/`intmin` return the class bounds as doubles.
* `eig` of a complex matrix and complex least squares are not supported; `svd` always returns the
  economy-size `U` and `V`.
* `fft` accepts any length (Bluestein for non-powers of two) up to 2²⁴ points. `interp1` has no `makima`,
  `spline` has no clamped end slopes, and `butter`/`cheby1` return transfer-function coefficients only.
* Optimisation: `fminsearch` is Nelder–Mead, `fminunc` BFGS with finite-difference gradients, `fsolve`
  and the `lsq*` functions Levenberg–Marquardt, `linprog` a two-phase simplex. There are no bounds or
  constraints for the nonlinear solvers yet (`lb`/`ub`, `fmincon`, `quadprog`, `intlinprog` are missing).
* ODEs: `ode45`, `ode23` and `ode113` all run the Dormand–Prince 5(4) pair, and `ode23s`, `ode15s` and
  `ode23t` all run a Rosenbrock (2,3) solver; the names are accepted so MATLAB code runs, but they are not
  the same algorithms. Output comes from the continuous extension at the times in `tspan` (or at every
  step for a two-element `tspan`, without MATLAB's `Refine`). `Events` (terminal and direction),
  backward integration and a constant `Mass` matrix (index-1 DAEs, singular allowed) work; a vector
  `AbsTol`, a time-dependent `Mass`, `deval`, `ode15i` and `dde23` do not.
* Control systems: a model is a struct (`tf`, `zpk` or `ss`, continuous or with a sample time). There is no
  operator overloading (`G1*G2`, `G/(1+G)`) and no `tf('s')`: use `series`, `parallel` and `feedback`, which
  need single-input single-output models and return a transfer function. `step`, `impulse`, `lsim`, `initial`,
  `stepinfo`, `bode`, `nyquist` and `margin` accept SISO models; `lsim` needs uniformly spaced times. `place` is
  single-input (Ackermann), `lyap` is limited to 24 states, and models stop at 400 states. `margin` reports the
  gain margin as a ratio, not in dB. There are no delays, no `ss` descriptor forms, no `minreal`, `balreal`
  or `lqg`/`kalman`.
* There are no character matrices: `char` and `num2str` of a matrix join rows with a newline.
* Files: `readmatrix`, `writematrix`, `csvread`, `csvwrite`, `dlmread`, `dlmwrite` (delimited text; headers
  are skipped, empty or non-numeric fields read as NaN; numbers are written so they read back exactly),
  `fileread`, and `save`/`load`. `save(file, 'a', 'b')` writes the named variables (all when none are
  named) to a text file of Ábaco's own (`.abd`, added when the name has no extension; it is not a MAT file
  and MATLAB cannot read it). It holds numbers, complex numbers, logicals, text, cells, structs and named
  handles; an anonymous function holds what it captured, so it is skipped with a warning. `load` restores
  such a file (`S = load(file)` returns a struct instead) or reads a text file of numbers into a variable
  named after the file. Both are function calls only: `save file x` is not command syntax. There is no
  `fopen`/`fgetl`/`fwrite`, no `readtable`, no MAT, Excel or image files and no network functions;
  `input` and `keyboard` are refused. `pause` never blocks.
* No units, classes, packages or `containers.Map`.
* Random streams are xoshiro256++; they are reproducible with `rng(seed)` but do not match MATLAB's.

## In the app

`abaco-app` runs Ábaco Script in its second workspace (View ▸ *Script and command window*, `Ctrl+J`;
`Ctrl+Shift+J` goes back to the maths document; `abaco-app file.m` opens a script there).

* **Editor**: `F5` runs the whole script, `Ctrl+Enter` the selection or, with none, the line the cursor
  is on, `Ctrl+.` stops a run. A failure names the line in the editor's header and prints the message
  with its function trace in the command window.
* **Command window**: type a command and press Enter; the up and down arrows walk the history.
  Output, warnings and errors keep their own colours. `clc` clears it; the window keeps its last 5,000
  lines.
* **Workspace**: every variable with its class, size and a one-line summary; selecting one shows what
  `disp` would print for it, and the bin button forgets it.
* **Figures**: one tab per figure. 2-D axes draw lines, markers, stems, stairs, bars and histograms
  with ticks, grid, legend, log axes and `axis equal`; 3-D axes draw `surf`/`mesh` and `plot3`, and
  dragging turns the view.
* The interpreter lives on its own thread for the whole session, so variables survive between runs. A
  run is stopped by a flag the interpreter checks at every step.

## Conditioning

`A\b`, `linsolve` and `inv` estimate the reciprocal condition number of the matrix (a 1-norm
estimate from the LU factors). When it falls below machine epsilon the answer is still returned, but
the statement is followed by the warning *Matrix is close to singular or badly scaled. Results may be
inaccurate. RCOND = …*, as MATLAB does; `rcond(A)` returns the estimate. An exactly singular matrix is
an error. Verified: `rcond(eye(n)) = 1`, `rcond` of an exactly singular matrix is `0`, and
`rcond(hilb(6))` is within 20 % of the published 1/2.9070e7 (a 1-norm estimate need not match the exact
value).
