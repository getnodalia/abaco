// SPDX-License-Identifier: MIT OR Apache-2.0
//! Between the values of a MAT-file ([`abaco_formats::mat`]) and the values of the interpreter.
//!
//! The interpreter holds `double`, complex, logical, text, cells, scalar structs and function
//! handles, in two dimensions. A MAT-file can hold more: other numeric classes, N-d arrays,
//! struct arrays, sparse matrices, objects. Whatever does not fit is converted the nearest way
//! and **said**: every conversion that loses something adds a warning, and what cannot be
//! converted at all is left out with a warning. Nothing is dropped silently.

use crate::ops::MAX_ELEMENTS;
use crate::value::{Arr, StructV, Value};
use abaco_formats::mat::{MatValue, NumClass};
use num_complex::Complex64;

/// The deepest nesting of cells and structs.
const MAX_DEPTH: usize = 64;

/// Two dimensions from a MAT dimension list (trailing 1s are ignored).
fn dims2(dims: &[usize]) -> Option<(usize, usize)> {
    match dims {
        [r, c] => Some((*r, *c)),
        [r, c, rest @ ..] if rest.iter().all(|d| *d == 1) => Some((*r, *c)),
        _ => None,
    }
}

fn checked_len(r: usize, c: usize) -> Option<usize> {
    r.checked_mul(c).filter(|n| *n <= MAX_ELEMENTS)
}

/// A MAT variable as an interpreter value; `None` (with a warning) when it cannot be held.
pub(super) fn to_value(v: &MatValue, name: &str, warns: &mut Vec<String>) -> Option<Value> {
    convert_in(v, name, warns, 0)
}

fn convert_in(v: &MatValue, name: &str, warns: &mut Vec<String>, depth: usize) -> Option<Value> {
    if depth > MAX_DEPTH {
        warns.push(format!("`{name}` is nested too deeply and was left out"));
        return None;
    }
    let shape = |dims: &[usize], warns: &mut Vec<String>| -> Option<(usize, usize)> {
        let d = dims2(dims);
        if d.is_none() {
            warns.push(format!(
                "`{name}` has {} dimensions; only arrays of two dimensions are supported, so it was left out",
                dims.len()
            ));
        }
        d
    };
    match v {
        MatValue::Numeric {
            class,
            dims,
            re,
            im,
        } => {
            let (r, c) = shape(dims, warns)?;
            if checked_len(r, c)? != re.len() {
                warns.push(format!("`{name}` is inconsistent and was left out"));
                return None;
            }
            if *class != NumClass::Double {
                warns.push(format!(
                    "`{name}` is {}; it is held as double",
                    class.name()
                ));
            }
            Some(match im {
                Some(im) => {
                    let d = re
                        .iter()
                        .zip(im)
                        .map(|(a, b)| Complex64::new(*a, *b))
                        .collect();
                    Value::cplx(Arr::new(r, c, d))
                }
                None => Value::Num(Arr::new(r, c, re.clone())),
            })
        }
        MatValue::Logical { dims, data } => {
            let (r, c) = shape(dims, warns)?;
            (checked_len(r, c)? == data.len()).then(|| Value::Bool(Arr::new(r, c, data.clone())))
        }
        MatValue::Char { dims, data } => {
            let (r, c) = shape(dims, warns)?;
            if checked_len(r, c)? != data.len() {
                return None;
            }
            if r <= 1 {
                return Some(Value::Str(String::from_utf16_lossy(data)));
            }
            warns.push(format!(
                "`{name}` is a character matrix; it is held as a cell column of its rows"
            ));
            let rows = (0..r)
                .map(|i| {
                    let units: Vec<u16> = (0..c)
                        .filter_map(|j| data.get(j * r + i).copied())
                        .collect();
                    Value::Str(String::from_utf16_lossy(&units))
                })
                .collect();
            Some(Value::Cell(Arr::col(rows)))
        }
        MatValue::Cell { dims, items } => {
            let (r, c) = shape(dims, warns)?;
            if checked_len(r, c)? != items.len() {
                return None;
            }
            let mut out = Vec::with_capacity(items.len());
            for (k, it) in items.iter().enumerate() {
                let inner = format!("{name}{{{}}}", k + 1);
                out.push(convert_in(it, &inner, warns, depth + 1).unwrap_or_else(Value::empty));
            }
            Some(Value::Cell(Arr::new(r, c, out)))
        }
        MatValue::Struct {
            dims,
            fields,
            values,
        } => {
            let (r, c) = shape(dims, warns)?;
            let n = checked_len(r, c)?;
            if n.checked_mul(fields.len()) != Some(values.len()) {
                return None;
            }
            let mut elems = Vec::with_capacity(n);
            for e in 0..n {
                let mut s = StructV::default();
                for (f, fname) in fields.iter().enumerate() {
                    let inner = format!("{name}.{fname}");
                    let val = values
                        .get(e * fields.len() + f)
                        .and_then(|x| convert_in(x, &inner, warns, depth + 1))
                        .unwrap_or_else(Value::empty);
                    s.set(fname, val);
                }
                elems.push(Value::Struct(Box::new(s)));
            }
            if n == 1 {
                return elems.pop();
            }
            warns.push(format!(
                "`{name}` is a struct array; it is held as a {r}x{c} cell array of structs"
            ));
            Some(Value::Cell(Arr::new(r, c, elems)))
        }
        MatValue::Sparse {
            rows,
            cols,
            row_idx,
            col_ptr,
            re,
            im,
            logical,
        } => {
            let n = checked_len(*rows, *cols);
            let Some(n) = n else {
                warns.push(format!(
                    "`{name}` is a sparse matrix too large to hold in full and was left out"
                ));
                return None;
            };
            warns.push(format!("`{name}` is sparse; it is held as a full matrix"));
            let mut dre = vec![0.0; n];
            let mut dim = im.as_ref().map(|_| vec![0.0; n]);
            for j in 0..*cols {
                let (Some(&lo), Some(&hi)) = (col_ptr.get(j), col_ptr.get(j + 1)) else {
                    return None;
                };
                for k in lo..hi {
                    let (Some(&i), Some(&x)) = (row_idx.get(k), re.get(k)) else {
                        return None;
                    };
                    if i >= *rows {
                        return None;
                    }
                    if let Some(slot) = dre.get_mut(j * rows + i) {
                        *slot = x;
                    }
                    if let (Some(d), Some(imv)) = (dim.as_mut(), im.as_ref()) {
                        if let (Some(slot), Some(y)) = (d.get_mut(j * rows + i), imv.get(k)) {
                            *slot = *y;
                        }
                    }
                }
            }
            if *logical {
                return Some(Value::Bool(Arr::new(
                    *rows,
                    *cols,
                    dre.iter().map(|x| *x != 0.0).collect(),
                )));
            }
            Some(match dim {
                Some(dim) => Value::cplx(Arr::new(
                    *rows,
                    *cols,
                    dre.iter()
                        .zip(&dim)
                        .map(|(a, b)| Complex64::new(*a, *b))
                        .collect(),
                )),
                None => Value::Num(Arr::new(*rows, *cols, dre)),
            })
        }
        MatValue::Opaque { class } => {
            warns.push(format!(
                "`{name}` is a `{class}` (a function handle, an object or a string array), which Ábaco cannot hold; it was left out"
            ));
            None
        }
    }
}

/// An interpreter value as a MAT variable; `None` (with a warning) when MAT-files cannot hold it.
pub(super) fn from_value(v: &Value, name: &str, warns: &mut Vec<String>) -> Option<MatValue> {
    convert_out(v, name, warns, 0)
}

fn convert_out(v: &Value, name: &str, warns: &mut Vec<String>, depth: usize) -> Option<MatValue> {
    if depth > MAX_DEPTH {
        warns.push(format!("`{name}` is nested too deeply and was not saved"));
        return None;
    }
    Some(match v {
        Value::Num(a) => MatValue::Numeric {
            class: NumClass::Double,
            dims: vec![a.r, a.c],
            re: a.d.clone(),
            im: None,
        },
        Value::Cplx(a) => MatValue::Numeric {
            class: NumClass::Double,
            dims: vec![a.r, a.c],
            re: a.d.iter().map(|z| z.re).collect(),
            im: Some(a.d.iter().map(|z| z.im).collect()),
        },
        Value::Bool(a) => MatValue::Logical {
            dims: vec![a.r, a.c],
            data: a.d.clone(),
        },
        Value::Str(s) => MatValue::text(s),
        Value::Cell(a) => {
            let mut items = Vec::with_capacity(a.d.len());
            for (k, x) in a.d.iter().enumerate() {
                let inner = format!("{name}{{{}}}", k + 1);
                items.push(convert_out(x, &inner, warns, depth + 1).unwrap_or_else(|| {
                    // Keeps the cell's shape; the warning says what became of the element.
                    MatValue::Numeric {
                        class: NumClass::Double,
                        dims: vec![0, 0],
                        re: Vec::new(),
                        im: None,
                    }
                }));
            }
            MatValue::Cell {
                dims: vec![a.r, a.c],
                items,
            }
        }
        Value::Struct(s) => {
            let mut values = Vec::with_capacity(s.fields.len());
            for (f, x) in &s.fields {
                let inner = format!("{name}.{f}");
                values.push(convert_out(x, &inner, warns, depth + 1).unwrap_or_else(|| {
                    MatValue::Numeric {
                        class: NumClass::Double,
                        dims: vec![0, 0],
                        re: Vec::new(),
                        im: None,
                    }
                }));
            }
            MatValue::Struct {
                dims: vec![1, 1],
                fields: s.fields.iter().map(|(f, _)| f.clone()).collect(),
                values,
            }
        }
        Value::Fn(_) => {
            warns.push(format!(
                "`{name}` is a function handle; MAT-files cannot hold one written by Ábaco, so it was not saved"
            ));
            return None;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let mut s = StructV::default();
        s.set("a", Value::num(1.5));
        s.set("t", Value::text("héllo"));
        let vals = vec![
            Value::mat(2, 2, vec![1.0, 3.0, 2.0, 4.0]),
            Value::Bool(Arr::row(vec![true, false])),
            Value::text(""),
            Value::text("abc"),
            Value::cell_row(vec![Value::num(1.0), Value::text("x")]),
            Value::Struct(Box::new(s)),
            Value::cscalar(Complex64::new(1.0, -2.0)),
        ];
        for v in vals {
            let mut w = Vec::new();
            let m = from_value(&v, "x", &mut w).unwrap();
            let back = to_value(&m, "x", &mut w).unwrap();
            assert_eq!(back, v);
            assert!(w.is_empty(), "{w:?}");
        }
    }

    #[test]
    fn what_does_not_fit_is_reported() {
        let mut w = Vec::new();
        let m = MatValue::Numeric {
            class: NumClass::Int8,
            dims: vec![1, 2],
            re: vec![1.0, 2.0],
            im: None,
        };
        assert_eq!(to_value(&m, "k", &mut w), Some(Value::row(vec![1.0, 2.0])));
        assert!(w[0].contains("int8"));
        let nd = MatValue::Numeric {
            class: NumClass::Double,
            dims: vec![2, 2, 2],
            re: vec![0.0; 8],
            im: None,
        };
        assert_eq!(to_value(&nd, "nd", &mut w), None);
        assert!(w.last().is_some_and(|m| m.contains("3 dimensions")));
        let sp = MatValue::Sparse {
            rows: 2,
            cols: 2,
            row_idx: vec![1, 0],
            col_ptr: vec![0, 1, 2],
            re: vec![5.0, 7.0],
            im: None,
            logical: false,
        };
        assert_eq!(
            to_value(&sp, "sp", &mut w),
            Some(Value::mat(2, 2, vec![0.0, 5.0, 7.0, 0.0]))
        );
        let op = MatValue::Opaque {
            class: "function_handle".into(),
        };
        assert_eq!(to_value(&op, "f", &mut w), None);
        let f = Value::Fn(std::sync::Arc::new(crate::value::FnValue::Named(
            "sin".into(),
        )));
        assert_eq!(from_value(&f, "f", &mut w), None);
    }

    #[test]
    fn damaged_shapes_are_left_out_not_panics() {
        let mut w = Vec::new();
        let bad = [
            MatValue::Numeric {
                class: NumClass::Double,
                dims: vec![3, 3],
                re: vec![1.0],
                im: None,
            },
            MatValue::Sparse {
                rows: 2,
                cols: 2,
                row_idx: vec![9],
                col_ptr: vec![0, 1, 1],
                re: vec![1.0],
                im: None,
                logical: false,
            },
            MatValue::Sparse {
                rows: usize::MAX,
                cols: 3,
                row_idx: vec![],
                col_ptr: vec![0; 4],
                re: vec![],
                im: None,
                logical: false,
            },
            MatValue::Cell {
                dims: vec![2, 2],
                items: vec![],
            },
        ];
        for b in bad {
            assert_eq!(to_value(&b, "b", &mut w), None);
        }
    }
}
