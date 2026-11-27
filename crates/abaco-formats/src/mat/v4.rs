// SPDX-License-Identifier: MIT OR Apache-2.0
//! MAT-file level 4 (`-v4`): a flat list of two-dimensional numeric, text or sparse matrices.

use super::{MatFile, MatValue, MatVersion, NumClass, MAX_BYTES};
use crate::error::{corrupt, unsupported, Cursor, Result};

fn convert(p: u32, d: &[u8], le: bool) -> Result<Vec<f64>> {
    macro_rules! conv {
        ($t:ty, $n:expr) => {{
            d.chunks_exact($n)
                .map(|c| {
                    let mut a = [0u8; $n];
                    a.copy_from_slice(c);
                    (if le {
                        <$t>::from_le_bytes(a)
                    } else {
                        <$t>::from_be_bytes(a)
                    }) as f64
                })
                .collect()
        }};
    }
    Ok(match p {
        0 => conv!(f64, 8),
        1 => conv!(f32, 4),
        2 => conv!(i32, 4),
        3 => conv!(i16, 2),
        4 => conv!(u16, 2),
        5 => d.iter().map(|&b| f64::from(b)).collect(),
        _ => return unsupported(format!("level 4 precision {p}")),
    })
}

fn psize(p: u32) -> usize {
    match p {
        0 => 8,
        1 | 2 => 4,
        3 | 4 => 2,
        _ => 1,
    }
}

/// Reads a level-4 MAT-file.
pub fn read_v4(bytes: &[u8]) -> Result<MatFile> {
    let mut cur = Cursor::new(bytes);
    let mut vars = Vec::new();
    while cur.remaining() > 0 {
        let start = cur.pos;
        let head = cur.take(20)?;
        let rd = |i: usize, le: bool| -> u32 {
            let a = [
                head.get(i).copied().unwrap_or(0),
                head.get(i + 1).copied().unwrap_or(0),
                head.get(i + 2).copied().unwrap_or(0),
                head.get(i + 3).copied().unwrap_or(0),
            ];
            if le {
                u32::from_le_bytes(a)
            } else {
                u32::from_be_bytes(a)
            }
        };
        let valid = |m: u32| {
            m < 5000
                && (m / 1000) <= 4
                && (m / 100).is_multiple_of(10)
                && (m / 10) % 10 <= 5
                && m % 10 <= 2
        };
        let le = if valid(rd(0, true)) {
            true
        } else if valid(rd(0, false)) {
            false
        } else {
            return if start == 0 {
                corrupt("this is not a MAT-file")
            } else {
                corrupt("a level 4 matrix header is not valid")
            };
        };
        let mopt = rd(0, le);
        if mopt / 1000 > 1 {
            return unsupported("level 4 files from VAX or Cray machines");
        }
        let (prec, kind) = ((mopt / 10) % 10, mopt % 10);
        let (rows, cols, imagf, namlen) = (
            rd(4, le) as usize,
            rd(8, le) as usize,
            rd(12, le),
            rd(16, le) as usize,
        );
        if namlen == 0 || namlen > 256 {
            return corrupt("a level 4 matrix has a bad name length");
        }
        let nb = cur.take(namlen)?;
        let end = nb.iter().position(|&b| b == 0).unwrap_or(nb.len());
        let name = String::from_utf8_lossy(nb.get(..end).unwrap_or(&[])).into_owned();
        let n = rows
            .checked_mul(cols)
            .filter(|&n| n.saturating_mul(8) <= MAX_BYTES)
            .map_or_else(|| corrupt("a level 4 matrix is too large"), Ok)?;
        let w = psize(prec);
        let re = convert(prec, cur.take(n * w)?, le)?;
        let im = if imagf != 0 {
            Some(convert(prec, cur.take(n * w)?, le)?)
        } else {
            None
        };
        let value = match kind {
            0 => MatValue::Numeric {
                class: NumClass::Double,
                dims: vec![rows, cols],
                re,
                im,
            },
            1 => MatValue::Char {
                dims: vec![rows, cols],
                data: re.iter().map(|&x| x as u16).collect(),
            },
            _ => {
                // Sparse: (nnz+1) rows of (i, j, re[, im]); the last row holds the size.
                let nnz = rows.saturating_sub(1);
                let (mut r, mut c) = (0usize, 0usize);
                let mut trip: Vec<(usize, usize, f64, f64)> = Vec::with_capacity(nnz);
                for k in 0..nnz {
                    let i = re.get(k).copied().unwrap_or(1.0);
                    let j = re.get(rows + k).copied().unwrap_or(1.0);
                    let v = re.get(2 * rows + k).copied().unwrap_or(0.0);
                    let vi = re.get(3 * rows + k).copied().unwrap_or(0.0);
                    if i < 1.0 || j < 1.0 || !i.is_finite() || !j.is_finite() {
                        return corrupt("a level 4 sparse matrix has a bad index");
                    }
                    trip.push((i as usize - 1, j as usize - 1, v, vi));
                }
                if rows > 0 {
                    r = re.get(rows - 1).copied().unwrap_or(0.0) as usize;
                    c = re.get(2 * rows - 1).copied().unwrap_or(0.0) as usize;
                }
                if trip.iter().any(|t| t.0 >= r.max(1) || t.1 >= c.max(1)) {
                    return corrupt("a level 4 sparse matrix has an index outside its size");
                }
                trip.sort_by_key(|t| (t.1, t.0));
                let mut col_ptr = vec![0usize; c + 1];
                for t in &trip {
                    if let Some(p) = col_ptr.get_mut(t.1 + 1) {
                        *p += 1;
                    }
                }
                for j in 0..c {
                    let prev = col_ptr.get(j).copied().unwrap_or(0);
                    if let Some(p) = col_ptr.get_mut(j + 1) {
                        *p += prev;
                    }
                }
                let complex = cols >= 4;
                MatValue::Sparse {
                    rows: r,
                    cols: c,
                    row_idx: trip.iter().map(|t| t.0).collect(),
                    col_ptr,
                    re: trip.iter().map(|t| t.2).collect(),
                    im: complex.then(|| trip.iter().map(|t| t.3).collect()),
                    logical: false,
                }
            }
        };
        vars.push((name, value));
    }
    Ok(MatFile {
        version: MatVersion::V4,
        description: String::new(),
        vars,
    })
}

fn matrix(
    out: &mut Vec<u8>,
    name: &str,
    rows: usize,
    cols: usize,
    kind: u32,
    re: &[f64],
    im: Option<&[f64]>,
) {
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(rows as u32).to_le_bytes());
    out.extend_from_slice(&(cols as u32).to_le_bytes());
    out.extend_from_slice(&u32::from(im.is_some()).to_le_bytes());
    out.extend_from_slice(&((name.len() + 1) as u32).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    for &x in re {
        out.extend_from_slice(&x.to_le_bytes());
    }
    if let Some(im) = im {
        for &x in im {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
}

/// Writes a level-4 MAT-file. Only two-dimensional numeric, logical, text and sparse values
/// can be stored; anything else is an error naming the variable. Integers and singles are
/// written as doubles.
pub fn write_v4(vars: &[(String, MatValue)]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for (name, v) in vars {
        match v {
            MatValue::Numeric { dims, re, im, .. } if dims.len() == 2 => {
                matrix(&mut out, name, dims[0], dims[1], 0, re, im.as_deref());
            }
            MatValue::Logical { dims, data } if dims.len() == 2 => {
                let re: Vec<f64> = data.iter().map(|&b| f64::from(u8::from(b))).collect();
                matrix(&mut out, name, dims[0], dims[1], 0, &re, None);
            }
            MatValue::Char { dims, data } if dims.len() == 2 => {
                let re: Vec<f64> = data.iter().map(|&c| f64::from(c)).collect();
                matrix(&mut out, name, dims[0], dims[1], 1, &re, None);
            }
            MatValue::Sparse {
                rows,
                cols,
                row_idx,
                col_ptr,
                re,
                im,
                ..
            } => {
                let nnz = row_idx.len();
                let width = if im.is_some() { 4 } else { 3 };
                let mut d = vec![0.0; (nnz + 1) * width];
                let r = nnz + 1;
                let mut k = 0;
                for j in 0..*cols {
                    let (a, b) = (
                        col_ptr.get(j).copied().unwrap_or(0),
                        col_ptr.get(j + 1).copied().unwrap_or(0),
                    );
                    for p in a..b {
                        d[k] = row_idx.get(p).copied().unwrap_or(0) as f64 + 1.0;
                        d[r + k] = j as f64 + 1.0;
                        d[2 * r + k] = re.get(p).copied().unwrap_or(0.0);
                        if let Some(im) = im {
                            d[3 * r + k] = im.get(p).copied().unwrap_or(0.0);
                        }
                        k += 1;
                    }
                }
                d[nnz] = *rows as f64;
                d[r + nnz] = *cols as f64;
                matrix(&mut out, name, r, width, 2, &d, None);
            }
            other => {
                return unsupported(format!(
                    "`{name}` ({}) cannot be stored in a level 4 MAT-file",
                    describe(other)
                ))
            }
        }
    }
    Ok(out)
}

fn describe(v: &MatValue) -> &'static str {
    match v {
        MatValue::Cell { .. } => "a cell array",
        MatValue::Struct { .. } => "a struct",
        MatValue::Opaque { .. } => "an object",
        _ => "an array with more than two dimensions",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let vars = vec![
            (
                "a".to_string(),
                MatValue::Numeric {
                    class: NumClass::Double,
                    dims: vec![2, 2],
                    re: vec![1.0, 2.0, 3.0, 4.0],
                    im: Some(vec![0.0, 1.0, 0.0, -1.0]),
                },
            ),
            ("t".to_string(), MatValue::text("abc")),
            (
                "sp".to_string(),
                MatValue::Sparse {
                    rows: 3,
                    cols: 2,
                    row_idx: vec![2, 0],
                    col_ptr: vec![0, 1, 2],
                    re: vec![5.0, 6.0],
                    im: None,
                    logical: false,
                },
            ),
        ];
        let f = read_v4(&write_v4(&vars).unwrap()).unwrap();
        assert_eq!(f.vars, vars);
        assert!(write_v4(&[(
            "c".into(),
            MatValue::Cell {
                dims: vec![1, 1],
                items: vec![MatValue::scalar(1.0)]
            }
        )])
        .is_err());
    }
}
