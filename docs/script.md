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

## Library (more than 350 functions; `abaco functions` lists them)

| Area | Functions |
|---|---|
| Creation | `zeros ones eye rand randn randi randperm rng linspace logspace colon meshgrid ndgrid magic toeplitz vander hilb diag triu tril kron repmat reshape cat horzcat vertcat circshift flip* rot90` |
| Elementwise maths | trigonometric and hyperbolic (and inverses, degree variants), `exp log log2 log10 sqrt abs sign floor ceil round fix mod rem hypot atan2 nthroot gamma lgamma erf erfc factorial nchoosek gcd lcm isprime primes factor`, complex parts |
| Reductions | `sum prod mean median mode var std range min max cumsum cumprod cummax cummin diff dot cross trapz cumtrapz any all find nnz sort sortrows unique histc cov corrcoef prctile` |
| Linear algebra | `det inv \ linsolve trace rank pinv null orth cond norm svd eig chol lu qr expm issymmetric` |
| Polynomials | `roots poly polyval polyfit polyder polyint conv deconv` |
| Calculus | `integral quad quadgk integral2 fzero` |
| Text | `sprintf printf fprintf disp display num2str int2str mat2str str2double str2num upper lower strtrim strrep strsplit strjoin strcat strtok strcmp strcmpi strncmp contains startsWith endsWith strfind regexp regexpi regexprep dec2bin bin2dec dec2hex hex2dec` |
| Cells and structs | `cell struct fieldnames isfield rmfield getfield setfield struct2cell cell2struct num2cell cell2mat deal cellfun arrayfun structfun isequal ismember union intersect setdiff setxor` |
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
* the test suite fuzzes the parser with arbitrary text, calls every built-in with junk arguments and
  checks that deep expressions end in a limit error on a 2 MB stack.

## Verification

Numerical functions are checked against closed forms and published values (see
`crates/abaco-script/tests/library.rs`): known linear systems, `Q R = A`, `P A = L U`, `U S Vᵀ = A`,
`A V = V D`, `expm` of a rotation generator, magic squares of every order class, `integral` of
`e^(−x²)` over the real line, `fzero` of `cos x = x`, special-function values, and printf formatting.

## Limits, stated plainly

* Arrays are two-dimensional. There are no N-d arrays and no struct arrays (a struct is scalar).
* There are no integer types: `int8 … uint64` round and saturate and `single` rounds to 32 bits, but
  values are stored and displayed as doubles. `intmax`/`intmin` return the class bounds as doubles.
* `eig` of a complex matrix and complex least squares are not supported; `svd` always returns the
  economy-size `U` and `V`.
* There are no character matrices: `char` and `num2str` of a matrix join rows with a newline.
* No file or network functions yet (`fopen`, `readmatrix`, `save`, `load`); `input` and `keyboard`
  are refused. `pause` never blocks.
* No units, classes, packages or `containers.Map`.
* Random streams are xoshiro256++; they are reproducible with `rng(seed)` but do not match MATLAB's.
