// SPDX-License-Identifier: MIT OR Apache-2.0
//! The numeric expressions found in the parameters of Simulink blocks: numbers (with `1e-3` and
//! imaginary `2i`), `[1 2; 3 4]` matrices, `a:b:c` ranges, the usual arithmetic, a few functions
//! (`sqrt`, `exp`, `sin`, `zeros`, …), the constants `pi`, `inf`, `nan` and `eps`, and the
//! variables of a workspace. Nothing is executed beyond that: an unknown name is an error that
//! says which one, so the person can set it.

use std::collections::BTreeMap;

const MAX_LEN: usize = 1 << 16;
const MAX_DEPTH: usize = 64;
const MAX_ELEMENTS: usize = 1 << 20;

/// A workspace: named real matrices, row by row.
pub type Workspace = BTreeMap<String, Vec<Vec<f64>>>;

/// A matrix of complex numbers, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Mat {
    /// Rows.
    pub rows: usize,
    /// Columns.
    pub cols: usize,
    /// Elements as `(re, im)`.
    pub data: Vec<(f64, f64)>,
}

impl Mat {
    fn scalar(re: f64, im: f64) -> Mat {
        Mat {
            rows: 1,
            cols: 1,
            data: vec![(re, im)],
        }
    }

    fn is_scalar(&self) -> bool {
        self.data.len() == 1
    }

    /// Whether every element is real.
    pub fn is_real(&self) -> bool {
        self.data.iter().all(|c| c.1 == 0.0)
    }

    /// The real parts, row-major.
    pub fn re(&self) -> Vec<f64> {
        self.data.iter().map(|c| c.0).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64, bool),
    Id(String),
    Op(char),
    DotOp(char),
    End,
}

struct Lexed {
    tok: Tok,
    space_before: bool,
}

fn lex(s: &str) -> Result<Vec<Lexed>, String> {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let mut space = false;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            space = true;
            i += 1;
            continue;
        }
        let tok = if c.is_ascii_digit()
            || (c == '.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                // `1.*x` is `1 .* x`, not `1. * x`.
                if b[i] == '.' && matches!(b.get(i + 1), Some('*' | '/' | '^')) {
                    break;
                }
                i += 1;
            }
            if i < b.len() && (b[i] == 'e' || b[i] == 'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == '+' || b[j] == '-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let text: String = b[start..i].iter().collect();
            let v: f64 = text
                .parse()
                .map_err(|_| format!("`{text}` is not a number"))?;
            let imag = i < b.len()
                && (b[i] == 'i' || b[i] == 'j')
                && !b
                    .get(i + 1)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_');
            if imag {
                i += 1;
            }
            Tok::Num(v, imag)
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            Tok::Id(b[start..i].iter().collect())
        } else if c == '.' && matches!(b.get(i + 1), Some('*' | '/' | '^')) {
            let op = b[i + 1];
            i += 2;
            Tok::DotOp(op)
        } else if "+-*/^()[],;:".contains(c) {
            i += 1;
            Tok::Op(c)
        } else {
            return Err(format!("unexpected `{c}`"));
        };
        out.push(Lexed {
            tok,
            space_before: space,
        });
        space = false;
    }
    out.push(Lexed {
        tok: Tok::End,
        space_before: space,
    });
    Ok(out)
}

struct P<'a> {
    t: Vec<Lexed>,
    i: usize,
    ws: &'a Workspace,
    in_matrix: bool,
    depth: usize,
}

type R<T> = Result<T, String>;

impl P<'_> {
    fn peek(&self) -> &Tok {
        &self.t[self.i.min(self.t.len() - 1)].tok
    }

    fn space_before(&self, at: usize) -> bool {
        self.t.get(at).is_some_and(|l| l.space_before)
    }

    fn next(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.i < self.t.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, c: char) -> bool {
        if *self.peek() == Tok::Op(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("the expression is nested too deeply".into());
        }
        Ok(())
    }

    fn range(&mut self) -> R<Mat> {
        let a = self.additive()?;
        if *self.peek() != Tok::Op(':') {
            return Ok(a);
        }
        self.i += 1;
        let b = self.additive()?;
        let (start, step, stop) = if *self.peek() == Tok::Op(':') {
            self.i += 1;
            let c = self.additive()?;
            (a, b, c)
        } else {
            (a, Mat::scalar(1.0, 0.0), b)
        };
        let get = |m: &Mat| -> R<f64> {
            match m.data.as_slice() {
                [(r, 0.0)] => Ok(*r),
                _ => Err("the bounds of a range must be real numbers".into()),
            }
        };
        let (s, d, e) = (get(&start)?, get(&step)?, get(&stop)?);
        if d == 0.0 || !(s.is_finite() && d.is_finite() && e.is_finite()) {
            return Err("a range needs finite bounds and a non-zero step".into());
        }
        let n = ((e - s) / d + 1e-10).floor();
        if n < 0.0 {
            return Ok(Mat {
                rows: 1,
                cols: 0,
                data: vec![],
            });
        }
        if n + 1.0 > MAX_ELEMENTS as f64 {
            return Err("a range is too long".into());
        }
        let n = n as usize + 1;
        Ok(Mat {
            rows: 1,
            cols: n,
            data: (0..n).map(|k| (s + d * k as f64, 0.0)).collect(),
        })
    }

    fn additive(&mut self) -> R<Mat> {
        let mut a = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Op(c @ ('+' | '-')) => *c,
                _ => return Ok(a),
            };
            // In a matrix, `[1 -2]` is two elements but `[1 - 2]` is one.
            if self.in_matrix && self.space_before(self.i) && !self.space_before(self.i + 1) {
                return Ok(a);
            }
            self.i += 1;
            let b = self.term()?;
            a = zip(&a, &b, |x, y| {
                if op == '+' {
                    (x.0 + y.0, x.1 + y.1)
                } else {
                    (x.0 - y.0, x.1 - y.1)
                }
            })?;
        }
    }

    fn term(&mut self) -> R<Mat> {
        let mut a = self.unary()?;
        loop {
            let (op, dot) = match self.peek() {
                Tok::Op(c @ ('*' | '/')) => (*c, false),
                Tok::DotOp(c @ ('*' | '/')) => (*c, true),
                _ => return Ok(a),
            };
            self.i += 1;
            let b = self.unary()?;
            if !dot && !a.is_scalar() && !b.is_scalar() {
                return Err("matrix products are not supported in parameters (use `.*`)".into());
            }
            a = zip(
                &a,
                &b,
                |x, y| if op == '*' { cmul(x, y) } else { cdiv(x, y) },
            )?;
        }
    }

    fn unary(&mut self) -> R<Mat> {
        self.deeper()?;
        let r = match self.peek() {
            Tok::Op('-') => {
                self.i += 1;
                let m = self.unary()?;
                Mat {
                    data: m.data.iter().map(|c| (-c.0, -c.1)).collect(),
                    ..m
                }
            }
            Tok::Op('+') => {
                self.i += 1;
                self.unary()?
            }
            _ => self.power()?,
        };
        self.depth -= 1;
        Ok(r)
    }

    fn power(&mut self) -> R<Mat> {
        let a = self.atom()?;
        if matches!(self.peek(), Tok::Op('^') | Tok::DotOp('^')) {
            self.i += 1;
            let b = self.unary()?;
            return zip(&a, &b, cpow);
        }
        Ok(a)
    }

    fn atom(&mut self) -> R<Mat> {
        match self.next() {
            Tok::Num(v, imag) => Ok(if imag {
                Mat::scalar(0.0, v)
            } else {
                Mat::scalar(v, 0.0)
            }),
            Tok::Op('(') => {
                self.deeper()?;
                let save = self.in_matrix;
                self.in_matrix = false;
                let m = self.range()?;
                self.in_matrix = save;
                self.depth -= 1;
                if !self.eat(')') {
                    return Err("`)` is missing".into());
                }
                Ok(m)
            }
            Tok::Op('[') => {
                self.deeper()?;
                let m = self.matrix()?;
                self.depth -= 1;
                Ok(m)
            }
            Tok::Id(name) => {
                if *self.peek() == Tok::Op('(') && !self.ws.contains_key(&name) {
                    self.i += 1;
                    let mut args = Vec::new();
                    let save = self.in_matrix;
                    self.in_matrix = false;
                    if !self.eat(')') {
                        loop {
                            args.push(self.range()?);
                            if self.eat(')') {
                                break;
                            }
                            if !self.eat(',') {
                                return Err("`,` or `)` expected in a call".into());
                            }
                        }
                    }
                    self.in_matrix = save;
                    call(&name, &args)
                } else {
                    constant(&name, self.ws)
                }
            }
            t => Err(format!("unexpected {}", describe(&t))),
        }
    }

    fn matrix(&mut self) -> R<Mat> {
        let save = self.in_matrix;
        self.in_matrix = true;
        let mut rows: Vec<Mat> = Vec::new();
        let mut row: Vec<Mat> = Vec::new();
        loop {
            match self.peek() {
                Tok::Op(']') => {
                    self.i += 1;
                    break;
                }
                Tok::Op(';') => {
                    self.i += 1;
                    rows.push(hcat(std::mem::take(&mut row))?);
                }
                Tok::Op(',') => self.i += 1,
                Tok::End => return Err("`]` is missing".into()),
                _ => row.push(self.range()?),
            }
        }
        if !row.is_empty() {
            rows.push(hcat(row)?);
        }
        self.in_matrix = save;
        vcat(rows)
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::End => "the end of the expression".into(),
        Tok::Op(c) | Tok::DotOp(c) => format!("`{c}`"),
        Tok::Id(s) => format!("`{s}`"),
        Tok::Num(v, _) => format!("`{v}`"),
    }
}

fn zip(a: &Mat, b: &Mat, f: impl Fn((f64, f64), (f64, f64)) -> (f64, f64)) -> R<Mat> {
    if a.is_scalar() {
        let x = a.data[0];
        return Ok(Mat {
            rows: b.rows,
            cols: b.cols,
            data: b.data.iter().map(|&y| f(x, y)).collect(),
        });
    }
    if b.is_scalar() {
        let y = b.data[0];
        return Ok(Mat {
            rows: a.rows,
            cols: a.cols,
            data: a.data.iter().map(|&x| f(x, y)).collect(),
        });
    }
    if a.rows != b.rows || a.cols != b.cols {
        return Err(format!(
            "the sizes {}x{} and {}x{} do not match",
            a.rows, a.cols, b.rows, b.cols
        ));
    }
    Ok(Mat {
        rows: a.rows,
        cols: a.cols,
        data: a.data.iter().zip(&b.data).map(|(&x, &y)| f(x, y)).collect(),
    })
}

fn cmul(x: (f64, f64), y: (f64, f64)) -> (f64, f64) {
    (x.0 * y.0 - x.1 * y.1, x.0 * y.1 + x.1 * y.0)
}

fn cdiv(x: (f64, f64), y: (f64, f64)) -> (f64, f64) {
    if y.1 == 0.0 {
        return (x.0 / y.0, x.1 / y.0);
    }
    let d = y.0 * y.0 + y.1 * y.1;
    ((x.0 * y.0 + x.1 * y.1) / d, (x.1 * y.0 - x.0 * y.1) / d)
}

fn cpow(x: (f64, f64), y: (f64, f64)) -> (f64, f64) {
    if x.1 == 0.0 && y.1 == 0.0 && (x.0 >= 0.0 || y.0.fract() == 0.0) {
        return (x.0.powf(y.0), 0.0);
    }
    if x == (0.0, 0.0) {
        return (0.0, 0.0);
    }
    let (r, th) = ((x.0 * x.0 + x.1 * x.1).sqrt().ln(), x.1.atan2(x.0));
    let (re, im) = (y.0 * r - y.1 * th, y.0 * th + y.1 * r);
    let m = re.exp();
    (m * im.cos(), m * im.sin())
}

fn hcat(parts: Vec<Mat>) -> R<Mat> {
    let parts: Vec<Mat> = parts
        .into_iter()
        .filter(|m| !m.data.is_empty() || m.rows > 0)
        .collect();
    let Some(first) = parts.first() else {
        return Ok(Mat {
            rows: 0,
            cols: 0,
            data: vec![],
        });
    };
    let rows = first.rows;
    if parts.iter().any(|m| m.rows != rows) {
        return Err("the pieces of a row have different heights".into());
    }
    let cols: usize = parts.iter().map(|m| m.cols).sum();
    if rows.saturating_mul(cols) > MAX_ELEMENTS {
        return Err("a matrix is too large".into());
    }
    let mut data = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for m in &parts {
            data.extend_from_slice(&m.data[r * m.cols..(r + 1) * m.cols]);
        }
    }
    Ok(Mat { rows, cols, data })
}

fn vcat(parts: Vec<Mat>) -> R<Mat> {
    let parts: Vec<Mat> = parts.into_iter().filter(|m| !m.data.is_empty()).collect();
    let Some(first) = parts.first() else {
        return Ok(Mat {
            rows: 0,
            cols: 0,
            data: vec![],
        });
    };
    let cols = first.cols;
    if parts.iter().any(|m| m.cols != cols) {
        return Err("the rows of a matrix have different lengths".into());
    }
    let rows = parts.iter().map(|m| m.rows).sum();
    let mut data = Vec::new();
    for m in &parts {
        data.extend_from_slice(&m.data);
    }
    Ok(Mat { rows, cols, data })
}

fn constant(name: &str, ws: &Workspace) -> R<Mat> {
    if let Some(rows) = ws.get(name) {
        let r = rows.len();
        let c = rows.first().map_or(0, Vec::len);
        if rows.iter().any(|x| x.len() != c) {
            return Err(format!("the variable `{name}` is not a rectangular matrix"));
        }
        return Ok(Mat {
            rows: r,
            cols: c,
            data: rows.iter().flatten().map(|&x| (x, 0.0)).collect(),
        });
    }
    Ok(Mat::scalar(
        match name {
            "pi" => std::f64::consts::PI,
            "inf" | "Inf" => f64::INFINITY,
            "nan" | "NaN" => f64::NAN,
            "eps" => f64::EPSILON,
            "i" | "j" => return Ok(Mat::scalar(0.0, 1.0)),
            "true" => 1.0,
            "false" => 0.0,
            _ => {
                return Err(format!(
                    "the variable `{name}` is not defined; give it a value in the workspace"
                ))
            }
        },
        0.0,
    ))
}

fn call(name: &str, args: &[Mat]) -> R<Mat> {
    let map = |f: fn(f64) -> f64| -> R<Mat> {
        let [a] = args else {
            return Err(format!("`{name}` takes one argument"));
        };
        if !a.is_real() {
            return Err(format!(
                "`{name}` of a complex number is not supported in parameters"
            ));
        }
        Ok(Mat {
            data: a.data.iter().map(|c| (f(c.0), 0.0)).collect(),
            ..a.clone()
        })
    };
    let dims = || -> R<(usize, usize)> {
        let g = |m: &Mat| match m.data.as_slice() {
            [(r, 0.0)] if *r >= 0.0 && r.fract() == 0.0 && *r < 1e6 => Ok(*r as usize),
            _ => Err(format!("`{name}` needs non-negative integer sizes")),
        };
        match args {
            [n] => Ok((g(n)?, g(n)?)),
            [r, c] => Ok((g(r)?, g(c)?)),
            _ => Err(format!("`{name}` takes one or two sizes")),
        }
    };
    match name {
        "sqrt" => {
            let [a] = args else {
                return Err("`sqrt` takes one argument".into());
            };
            Ok(Mat {
                data: a.data.iter().map(|&c| cpow(c, (0.5, 0.0))).collect(),
                ..a.clone()
            })
        }
        "exp" => map(f64::exp),
        "sin" => map(f64::sin),
        "cos" => map(f64::cos),
        "tan" => map(f64::tan),
        "asin" => map(f64::asin),
        "acos" => map(f64::acos),
        "atan" => map(f64::atan),
        "log" => map(f64::ln),
        "log10" => map(f64::log10),
        "floor" => map(f64::floor),
        "ceil" => map(f64::ceil),
        "round" => map(f64::round),
        "fix" => map(f64::trunc),
        "abs" => {
            let [a] = args else {
                return Err("`abs` takes one argument".into());
            };
            Ok(Mat {
                data: a.data.iter().map(|c| (c.0.hypot(c.1), 0.0)).collect(),
                ..a.clone()
            })
        }
        "zeros" | "ones" => {
            let (r, c) = dims()?;
            if r.saturating_mul(c) > MAX_ELEMENTS {
                return Err("a matrix is too large".into());
            }
            Ok(Mat {
                rows: r,
                cols: c,
                data: vec![(if name == "ones" { 1.0 } else { 0.0 }, 0.0); r * c],
            })
        }
        "eye" => {
            let (r, c) = dims()?;
            if r.saturating_mul(c) > MAX_ELEMENTS {
                return Err("a matrix is too large".into());
            }
            let mut data = vec![(0.0, 0.0); r * c];
            for k in 0..r.min(c) {
                data[k * c + k] = (1.0, 0.0);
            }
            Ok(Mat {
                rows: r,
                cols: c,
                data,
            })
        }
        "linspace" => {
            let get = |m: &Mat| -> R<f64> {
                match m.data.as_slice() {
                    [(r, 0.0)] => Ok(*r),
                    _ => Err("`linspace` takes real numbers".into()),
                }
            };
            let (a, b, n) = match args {
                [a, b] => (get(a)?, get(b)?, 100.0),
                [a, b, n] => (get(a)?, get(b)?, get(n)?),
                _ => return Err("`linspace` takes two or three arguments".into()),
            };
            if !(1.0..=MAX_ELEMENTS as f64).contains(&n) {
                return Err("`linspace` needs a sensible number of points".into());
            }
            let n = n as usize;
            Ok(Mat {
                rows: 1,
                cols: n,
                data: (0..n)
                    .map(|k| {
                        (
                            if n == 1 {
                                b
                            } else {
                                a + (b - a) * k as f64 / (n - 1) as f64
                            },
                            0.0,
                        )
                    })
                    .collect(),
            })
        }
        _ => Err(format!(
            "the function `{name}` is not supported in parameters"
        )),
    }
}

/// Evaluates `text` to a matrix.
pub fn eval(text: &str, ws: &Workspace) -> Result<Mat, String> {
    let text = text.trim();
    if text.len() > MAX_LEN {
        return Err("the expression is too long".into());
    }
    let text = text.trim_matches(|c| c == '"' || c == '\'');
    if text.is_empty() {
        return Err("the expression is empty".into());
    }
    let mut p = P {
        t: lex(text)?,
        i: 0,
        ws,
        in_matrix: false,
        depth: 0,
    };
    let m = p.range()?;
    if *p.peek() != Tok::End {
        return Err(format!("unexpected {}", describe(p.peek())));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Mat {
        eval(s, &Workspace::new()).unwrap()
    }

    #[test]
    fn numbers_and_arithmetic() {
        assert_eq!(ev("2*3+1").re(), vec![7.0]);
        assert_eq!(ev("-2^2").re(), vec![-4.0]);
        assert_eq!(ev("1e-3").re(), vec![0.001]);
        assert!((ev("2*pi").re()[0] - std::f64::consts::TAU).abs() < 1e-15);
        assert_eq!(ev("10/4").re(), vec![2.5]);
    }

    #[test]
    fn matrices_and_ranges() {
        let m = ev("[1 2; 3 4]");
        assert_eq!((m.rows, m.cols), (2, 2));
        assert_eq!(m.re(), vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(ev("[1 -2]").re(), vec![1.0, -2.0]);
        assert_eq!(ev("[1 - 2]").re(), vec![-1.0]);
        assert_eq!(ev("[-2:2]").re(), vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
        assert_eq!(ev("0:0.5:1").re(), vec![0.0, 0.5, 1.0]);
        assert_eq!(ev("[1, 2, 3]").re(), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn complex_numbers() {
        let m = ev("[-1+2i -1-2i]");
        assert_eq!(m.data, vec![(-1.0, 2.0), (-1.0, -2.0)]);
        assert_eq!(ev("2i*2i").data, vec![(-4.0, 0.0)]);
    }

    #[test]
    fn variables_and_functions() {
        let mut ws = Workspace::new();
        ws.insert("K".into(), vec![vec![5.0]]);
        ws.insert("v".into(), vec![vec![1.0, 2.0]]);
        assert_eq!(eval("K*2", &ws).unwrap().re(), vec![10.0]);
        assert_eq!(eval("[K v]", &ws).unwrap().re(), vec![5.0, 1.0, 2.0]);
        assert_eq!(eval("sqrt(16)", &ws).unwrap().re(), vec![4.0]);
        assert_eq!(eval("zeros(1,3)", &ws).unwrap().re(), vec![0.0; 3]);
        let e = eval("Kp", &ws).unwrap_err();
        assert!(e.contains("Kp"));
    }

    #[test]
    fn bad_input_is_an_error() {
        for bad in [
            "",
            "1+",
            "(1",
            "[1 2",
            "1 2",
            "foo(1)",
            "1 $ 2",
            "[1 2;3]",
            &"(".repeat(500),
            "1:0:3",
        ] {
            assert!(eval(bad, &Workspace::new()).is_err(), "{bad:.20}");
        }
    }
}
