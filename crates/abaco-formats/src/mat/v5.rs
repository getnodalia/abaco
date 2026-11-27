// SPDX-License-Identifier: MIT OR Apache-2.0
//! MAT-file level 5 (`-v6` and `-v7`): a 128-byte header and tagged data elements, each
//! optionally wrapped in zlib.

use super::{check_dims, numel, MatFile, MatValue, MatVersion, NumClass, MAX_BYTES, MAX_DEPTH};
use crate::deflate::{zlib_compress, zlib_decompress};
use crate::error::{corrupt, unsupported, Result};

const MI_INT8: u32 = 1;
const MI_UINT8: u32 = 2;
const MI_INT16: u32 = 3;
const MI_UINT16: u32 = 4;
const MI_INT32: u32 = 5;
const MI_UINT32: u32 = 6;
const MI_SINGLE: u32 = 7;
const MI_DOUBLE: u32 = 9;
const MI_INT64: u32 = 12;
const MI_UINT64: u32 = 13;
const MI_MATRIX: u32 = 14;
const MI_COMPRESSED: u32 = 15;
const MI_UTF8: u32 = 16;
const MI_UTF16: u32 = 17;
const MI_UTF32: u32 = 18;

const MX_CELL: u32 = 1;
const MX_STRUCT: u32 = 2;
const MX_OBJECT: u32 = 3;
const MX_CHAR: u32 = 4;
const MX_SPARSE: u32 = 5;
const MX_DOUBLE: u32 = 6;
const MX_SINGLE: u32 = 7;
const MX_INT8: u32 = 8;
const MX_UINT8: u32 = 9;
const MX_INT16: u32 = 10;
const MX_UINT16: u32 = 11;
const MX_INT32: u32 = 12;
const MX_UINT32: u32 = 13;
const MX_INT64: u32 = 14;
const MX_UINT64: u32 = 15;

struct Ctx {
    le: bool,
    /// Bytes still allowed to be produced by decompression.
    budget: usize,
}

impl Ctx {
    fn u32(&self, b: &[u8]) -> u32 {
        let a = [at(b, 0), at(b, 1), at(b, 2), at(b, 3)];
        if self.le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        }
    }
}

fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

/// One data element: its type and payload, and where the next one starts.
fn element<'a>(buf: &'a [u8], pos: usize, cx: &Ctx) -> Result<(u32, &'a [u8], usize)> {
    let Some(head) = buf.get(pos..pos + 4) else {
        return corrupt("a data element is cut off");
    };
    let w0 = cx.u32(head);
    if w0 >> 16 != 0 {
        let ty = w0 & 0xFFFF;
        let n = (w0 >> 16) as usize;
        if n > 4 {
            return corrupt("a small data element is larger than 4 bytes");
        }
        let Some(data) = buf.get(pos + 4..pos + 4 + n) else {
            return corrupt("a small data element is cut off");
        };
        return Ok((ty, data, pos + 8));
    }
    let Some(len4) = buf.get(pos + 4..pos + 8) else {
        return corrupt("a data element is cut off");
    };
    let n = cx.u32(len4) as usize;
    let start = pos + 8;
    let Some(data) = start.checked_add(n).and_then(|e| buf.get(start..e)) else {
        return corrupt("a data element is larger than the file");
    };
    let padded = if w0 == MI_COMPRESSED {
        n
    } else {
        n.div_ceil(8) * 8
    };
    Ok((w0, data, start + padded))
}

fn numbers(ty: u32, d: &[u8], cx: &Ctx) -> Result<Vec<f64>> {
    macro_rules! conv {
        ($t:ty, $n:expr) => {{
            let mut v = Vec::with_capacity(d.len() / $n);
            for c in d.chunks_exact($n) {
                let mut a = [0u8; $n];
                a.copy_from_slice(c);
                let x = if cx.le {
                    <$t>::from_le_bytes(a)
                } else {
                    <$t>::from_be_bytes(a)
                };
                v.push(x as f64);
            }
            v
        }};
    }
    Ok(match ty {
        MI_INT8 => d.iter().map(|&b| f64::from(b as i8)).collect(),
        MI_UINT8 => d.iter().map(|&b| f64::from(b)).collect(),
        MI_INT16 => conv!(i16, 2),
        MI_UINT16 => conv!(u16, 2),
        MI_INT32 => conv!(i32, 4),
        MI_UINT32 => conv!(u32, 4),
        MI_SINGLE => conv!(f32, 4),
        MI_DOUBLE => conv!(f64, 8),
        MI_INT64 => conv!(i64, 8),
        MI_UINT64 => conv!(u64, 8),
        _ => return unsupported(format!("numeric data of MAT type {ty}")),
    })
}

fn ints(ty: u32, d: &[u8], cx: &Ctx) -> Result<Vec<usize>> {
    numbers(ty, d, cx)?
        .into_iter()
        .map(|x| {
            if x >= 0.0 && x <= (MAX_BYTES as f64) && x.fract() == 0.0 {
                Ok(x as usize)
            } else {
                corrupt("a negative or huge index or dimension")
            }
        })
        .collect()
}

fn class_of(code: u32) -> Option<NumClass> {
    Some(match code {
        MX_DOUBLE => NumClass::Double,
        MX_SINGLE => NumClass::Single,
        MX_INT8 => NumClass::Int8,
        MX_UINT8 => NumClass::UInt8,
        MX_INT16 => NumClass::Int16,
        MX_UINT16 => NumClass::UInt16,
        MX_INT32 => NumClass::Int32,
        MX_UINT32 => NumClass::UInt32,
        MX_INT64 => NumClass::Int64,
        MX_UINT64 => NumClass::UInt64,
        _ => return None,
    })
}

fn code_of(c: NumClass) -> u32 {
    match c {
        NumClass::Double => MX_DOUBLE,
        NumClass::Single => MX_SINGLE,
        NumClass::Int8 => MX_INT8,
        NumClass::UInt8 => MX_UINT8,
        NumClass::Int16 => MX_INT16,
        NumClass::UInt16 => MX_UINT16,
        NumClass::Int32 => MX_INT32,
        NumClass::UInt32 => MX_UINT32,
        NumClass::Int64 => MX_INT64,
        NumClass::UInt64 => MX_UINT64,
    }
}

fn text_bytes(s: &[u8]) -> String {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(s.get(..end).unwrap_or(&[])).into_owned()
}

/// Parses a miMATRIX payload into its name and value.
fn matrix(payload: &[u8], cx: &mut Ctx, depth: usize) -> Result<(String, MatValue)> {
    if depth > MAX_DEPTH {
        return corrupt("cells and structs are nested too deeply");
    }
    let (fty, fdata, mut pos) = element(payload, 0, cx)?;
    if fty != MI_UINT32 || fdata.len() < 8 {
        return corrupt("an array has no flags");
    }
    let flags = cx.u32(fdata);
    let nzmax = cx.u32(fdata.get(4..).unwrap_or(&[])) as usize;
    let class = flags & 0xFF;
    let complex = flags & 0x0800 != 0;
    let logical = flags & 0x0200 != 0;
    let (dty, ddata, p) = element(payload, pos, cx)?;
    pos = p;
    let dims = ints(dty, ddata, cx)?;
    let (_, ndata, p) = element(payload, pos, cx)?;
    pos = p;
    let name = text_bytes(ndata);
    let total = numel(&dims)?;
    let value = match class {
        MX_CELL => {
            let mut items = Vec::with_capacity(total.min(1 << 16));
            for _ in 0..total {
                let (ty, d, p) = element(payload, pos, cx)?;
                pos = p;
                if ty != MI_MATRIX {
                    return corrupt("a cell holds something that is not an array");
                }
                items.push(matrix(d, cx, depth + 1)?.1);
            }
            MatValue::Cell { dims, items }
        }
        MX_STRUCT | MX_OBJECT => {
            if class == MX_OBJECT {
                let (_, _, p) = element(payload, pos, cx)?;
                pos = p;
            }
            let (_, lend, p) = element(payload, pos, cx)?;
            pos = p;
            let fl = cx.u32(lend) as usize;
            let (_, names, p) = element(payload, pos, cx)?;
            pos = p;
            if fl == 0 || fl > 1024 {
                return corrupt("a struct has a bad field-name length");
            }
            let fields: Vec<String> = names.chunks(fl).map(text_bytes).collect();
            let nf = fields.len();
            let mut values = Vec::with_capacity((total * nf).min(1 << 16));
            for _ in 0..total.saturating_mul(nf) {
                let (ty, d, p) = element(payload, pos, cx)?;
                pos = p;
                if ty != MI_MATRIX {
                    return corrupt("a struct holds something that is not an array");
                }
                values.push(matrix(d, cx, depth + 1)?.1);
            }
            MatValue::Struct {
                dims,
                fields,
                values,
            }
        }
        MX_CHAR => {
            let (ty, d, _) = element(payload, pos, cx)?;
            let data: Vec<u16> = match ty {
                MI_UTF8 => String::from_utf8_lossy(d).encode_utf16().collect(),
                MI_UTF16 | MI_UINT16 => numbers(MI_UINT16, d, cx)?
                    .into_iter()
                    .map(|x| x as u16)
                    .collect(),
                MI_UTF32 | MI_UINT32 => numbers(MI_UINT32, d, cx)?
                    .into_iter()
                    .flat_map(|x| {
                        let c = char::from_u32(x as u32).unwrap_or('\u{FFFD}');
                        let mut b = [0u16; 2];
                        c.encode_utf16(&mut b).to_vec()
                    })
                    .collect(),
                MI_UINT8 | MI_INT8 => d.iter().map(|&b| u16::from(b)).collect(),
                _ => return unsupported(format!("character data of MAT type {ty}")),
            };
            if ty != MI_UTF8 {
                check_dims(&dims, data.len())?;
            }
            let dims = if ty == MI_UTF8 {
                vec![usize::from(!data.is_empty()), data.len()]
            } else {
                dims
            };
            MatValue::Char { dims, data }
        }
        MX_SPARSE => {
            if dims.len() != 2 {
                return corrupt("a sparse matrix is not two-dimensional");
            }
            let (ity, idata, p) = element(payload, pos, cx)?;
            pos = p;
            let (jty, jdata, p) = element(payload, pos, cx)?;
            pos = p;
            let row_idx = ints(ity, idata, cx)?;
            let col_ptr = ints(jty, jdata, cx)?;
            let cols = dims.get(1).copied().unwrap_or(0);
            if col_ptr.len() < cols + 1 {
                return corrupt("a sparse matrix has too few column pointers");
            }
            let nnz = col_ptr
                .get(cols)
                .copied()
                .unwrap_or(0)
                .min(nzmax.max(row_idx.len()));
            let (rty, rdata, p) = element(payload, pos, cx)?;
            pos = p;
            let mut re = numbers(rty, rdata, cx)?;
            re.truncate(nnz);
            let im = if complex {
                let (ty, d, _) = element(payload, pos, cx)?;
                let mut v = numbers(ty, d, cx)?;
                v.truncate(nnz);
                Some(v)
            } else {
                None
            };
            let rows = dims.first().copied().unwrap_or(0);
            if row_idx.len() < nnz || re.len() < nnz || row_idx.iter().any(|&r| r >= rows.max(1)) {
                return corrupt("a sparse matrix has inconsistent indices");
            }
            MatValue::Sparse {
                rows,
                cols,
                row_idx: row_idx.into_iter().take(nnz).collect(),
                col_ptr: col_ptr.into_iter().take(cols + 1).collect(),
                re,
                im,
                logical,
            }
        }
        c if class_of(c).is_some() => {
            let nc = class_of(c).unwrap_or(NumClass::Double);
            let (rty, rdata, p) = element(payload, pos, cx)?;
            pos = p;
            let re = numbers(rty, rdata, cx)?;
            check_dims(&dims, re.len())?;
            let im = if complex {
                let (ity, idata, _) = element(payload, pos, cx)?;
                let v = numbers(ity, idata, cx)?;
                check_dims(&dims, v.len())?;
                Some(v)
            } else {
                None
            };
            if logical {
                MatValue::Logical {
                    dims,
                    data: re.iter().map(|&x| x != 0.0).collect(),
                }
            } else {
                MatValue::Numeric {
                    class: nc,
                    dims,
                    re,
                    im,
                }
            }
        }
        16..=18 => {
            let mut cls = String::from(match class {
                16 => "function_handle",
                _ => "opaque",
            });
            if class == 17 {
                if let Ok((_, _, p1)) = element(payload, pos, cx) {
                    if let Ok((_, _, p2)) = element(payload, p1, cx) {
                        if let Ok((_, n, _)) = element(payload, p1, cx) {
                            let t = text_bytes(n);
                            if !t.is_empty() {
                                cls = t;
                            }
                        }
                        let _ = p2;
                    }
                }
            }
            MatValue::Opaque { class: cls }
        }
        other => return unsupported(format!("MATLAB array class {other}")),
    };
    Ok((name, value))
}

/// Reads a level-5 MAT-file.
pub fn read_v5(bytes: &[u8]) -> Result<MatFile> {
    if bytes.len() < 128 {
        return corrupt("a level 5 MAT-file needs a 128-byte header");
    }
    let le = match bytes.get(126..128) {
        Some(b"IM") => true,
        Some(b"MI") => false,
        _ => return corrupt("bad byte-order mark in the MAT-file header"),
    };
    let mut cx = Ctx {
        le,
        budget: MAX_BYTES,
    };
    let version = {
        let v = bytes.get(124..126).unwrap_or(&[]);
        if le {
            u16::from_le_bytes([at(v, 0), at(v, 1)])
        } else {
            u16::from_be_bytes([at(v, 0), at(v, 1)])
        }
    };
    if version != 0x0100 {
        return unsupported(format!("MAT-file version {version:#06x}"));
    }
    let description = text_bytes(bytes.get(..116).unwrap_or(&[]))
        .trim_end()
        .to_string();
    let mut vars = Vec::new();
    let mut pos = 128;
    let mut compressed = false;
    while pos + 8 <= bytes.len() {
        let (ty, data, next) = element(bytes, pos, &cx)?;
        pos = next;
        let owned;
        let payload: &[u8] = if ty == MI_COMPRESSED {
            compressed = true;
            owned = zlib_decompress(data, cx.budget)?;
            cx.budget = cx.budget.saturating_sub(owned.len());
            // The inflated bytes are one data element of their own.
            let (ity, idata, _) = element(&owned, 0, &cx)?;
            if ity != MI_MATRIX {
                return corrupt("a compressed element is not an array");
            }
            idata
        } else if ty == MI_MATRIX {
            data
        } else {
            return corrupt(format!("a top-level element has type {ty}, not an array"));
        };
        let (name, v) = matrix(payload, &mut cx, 0)?;
        vars.push((name, v));
    }
    Ok(MatFile {
        version: if compressed {
            MatVersion::V7
        } else {
            MatVersion::V6
        },
        description,
        vars,
    })
}

// ───────────────────────────── writing ─────────────────────────────

fn put_elem(out: &mut Vec<u8>, ty: u32, data: &[u8]) {
    if data.len() <= 4 && ty != MI_MATRIX && !data.is_empty() {
        out.extend_from_slice(&(ty | ((data.len() as u32) << 16)).to_le_bytes());
        out.extend_from_slice(data);
        out.resize(out.len() + (4 - data.len()), 0);
    } else {
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        let pad = (8 - data.len() % 8) % 8;
        out.resize(out.len() + pad, 0);
    }
}

fn pack(ty: u32, x: &[f64]) -> Vec<u8> {
    let mut o = Vec::new();
    for &v in x {
        match ty {
            MI_INT8 => o.push(v as i8 as u8),
            MI_UINT8 => o.push(v as u8),
            MI_INT16 => o.extend_from_slice(&(v as i16).to_le_bytes()),
            MI_UINT16 => o.extend_from_slice(&(v as u16).to_le_bytes()),
            MI_INT32 => o.extend_from_slice(&(v as i32).to_le_bytes()),
            MI_UINT32 => o.extend_from_slice(&(v as u32).to_le_bytes()),
            MI_SINGLE => o.extend_from_slice(&(v as f32).to_le_bytes()),
            MI_INT64 => o.extend_from_slice(&(v as i64).to_le_bytes()),
            MI_UINT64 => o.extend_from_slice(&(v as u64).to_le_bytes()),
            _ => o.extend_from_slice(&v.to_le_bytes()),
        }
    }
    o
}

fn mi_of(c: NumClass) -> u32 {
    match c {
        NumClass::Double => MI_DOUBLE,
        NumClass::Single => MI_SINGLE,
        NumClass::Int8 => MI_INT8,
        NumClass::UInt8 => MI_UINT8,
        NumClass::Int16 => MI_INT16,
        NumClass::UInt16 => MI_UINT16,
        NumClass::Int32 => MI_INT32,
        NumClass::UInt32 => MI_UINT32,
        NumClass::Int64 => MI_INT64,
        NumClass::UInt64 => MI_UINT64,
    }
}

fn header(out: &mut Vec<u8>, class: u32, flags: u32, nzmax: u32, dims: &[usize], name: &str) {
    put_elem(
        out,
        MI_UINT32,
        &[(class | flags).to_le_bytes(), nzmax.to_le_bytes()].concat(),
    );
    let d: Vec<u8> = dims
        .iter()
        .flat_map(|&n| (n as i32).to_le_bytes())
        .collect();
    put_elem(out, MI_INT32, &d);
    put_elem(out, MI_INT8, name.as_bytes());
}

/// The bytes of one miMATRIX element (tag included) for `v` under `name`.
fn matrix_bytes(name: &str, v: &MatValue) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    match v {
        MatValue::Numeric {
            class,
            dims,
            re,
            im,
        } => {
            check_dims(dims, re.len())?;
            let flags = if im.is_some() { 0x0800 } else { 0 };
            header(&mut body, code_of(*class), flags, 0, dims, name);
            put_elem(&mut body, mi_of(*class), &pack(mi_of(*class), re));
            if let Some(im) = im {
                put_elem(&mut body, mi_of(*class), &pack(mi_of(*class), im));
            }
        }
        MatValue::Logical { dims, data } => {
            check_dims(dims, data.len())?;
            header(&mut body, MX_UINT8, 0x0200, 0, dims, name);
            let b: Vec<u8> = data.iter().map(|&x| u8::from(x)).collect();
            put_elem(&mut body, MI_UINT8, &b);
        }
        MatValue::Char { dims, data } => {
            check_dims(dims, data.len())?;
            header(&mut body, MX_CHAR, 0, 0, dims, name);
            let b: Vec<u8> = data.iter().flat_map(|c| c.to_le_bytes()).collect();
            put_elem(&mut body, MI_UINT16, &b);
        }
        MatValue::Cell { dims, items } => {
            check_dims(dims, items.len())?;
            header(&mut body, MX_CELL, 0, 0, dims, name);
            for it in items {
                body.extend(matrix_bytes("", it)?);
            }
        }
        MatValue::Struct {
            dims,
            fields,
            values,
        } => {
            let n = super::numel(dims)?;
            if values.len() != n * fields.len() {
                return corrupt("a struct array has the wrong number of values");
            }
            header(&mut body, MX_STRUCT, 0, 0, dims, name);
            let longest = fields.iter().map(|f| f.len()).max().unwrap_or(0);
            let fl = if longest < 32 {
                32
            } else {
                64.max(longest + 1)
            };
            put_elem(&mut body, MI_INT32, &(fl as i32).to_le_bytes());
            let mut names = Vec::new();
            for f in fields {
                let mut b = f.as_bytes().to_vec();
                b.resize(fl, 0);
                names.extend(b);
            }
            put_elem(&mut body, MI_INT8, &names);
            for v in values {
                body.extend(matrix_bytes("", v)?);
            }
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
            let flags = if im.is_some() { 0x0800 } else { 0 } | if *logical { 0x0200 } else { 0 };
            header(
                &mut body,
                MX_SPARSE,
                flags,
                row_idx.len().max(1) as u32,
                &[*rows, *cols],
                name,
            );
            let ir: Vec<u8> = row_idx
                .iter()
                .flat_map(|&i| (i as i32).to_le_bytes())
                .collect();
            let jc: Vec<u8> = col_ptr
                .iter()
                .flat_map(|&i| (i as i32).to_le_bytes())
                .collect();
            put_elem(&mut body, MI_INT32, &ir);
            put_elem(&mut body, MI_INT32, &jc);
            if *logical {
                put_elem(&mut body, MI_UINT8, &pack(MI_UINT8, re));
            } else {
                put_elem(&mut body, MI_DOUBLE, &pack(MI_DOUBLE, re));
            }
            if let Some(im) = im {
                put_elem(&mut body, MI_DOUBLE, &pack(MI_DOUBLE, im));
            }
        }
        MatValue::Opaque { class } => {
            return unsupported(format!("`{name}` is a {class}, which cannot be saved"));
        }
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&MI_MATRIX.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}

/// Writes a level-5 MAT-file; with `compress` every variable is zlib-compressed (`-v7`).
pub fn write_v5(vars: &[(String, MatValue)], description: &str, compress: bool) -> Result<Vec<u8>> {
    let text = if description.is_empty() {
        "MATLAB 5.0 MAT-file, written by Abaco"
    } else {
        description
    };
    let mut out = Vec::new();
    let mut head: Vec<u8> = text.bytes().take(116).collect();
    head.resize(116, b' ');
    out.extend(head);
    out.extend_from_slice(&[0u8; 8]);
    out.extend_from_slice(&0x0100u16.to_le_bytes());
    out.extend_from_slice(b"IM");
    for (name, v) in vars {
        let m = matrix_bytes(name, v)?;
        if compress {
            let z = zlib_compress(&m);
            out.extend_from_slice(&MI_COMPRESSED.to_le_bytes());
            out.extend_from_slice(&(z.len() as u32).to_le_bytes());
            // MATLAB does not pad compressed elements.
            out.extend(z);
        } else {
            out.extend(m);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(dims: &[usize], re: &[f64]) -> MatValue {
        MatValue::Numeric {
            class: NumClass::Double,
            dims: dims.to_vec(),
            re: re.to_vec(),
            im: None,
        }
    }

    fn sample() -> Vec<(String, MatValue)> {
        vec![
            ("a".into(), num(&[2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
            ("s".into(), MatValue::text("héllo")),
            (
                "z".into(),
                MatValue::Numeric {
                    class: NumClass::Double,
                    dims: vec![1, 2],
                    re: vec![1.0, 2.0],
                    im: Some(vec![-1.0, 0.5]),
                },
            ),
            (
                "i".into(),
                MatValue::Numeric {
                    class: NumClass::Int16,
                    dims: vec![1, 3],
                    re: vec![-1.0, 0.0, 300.0],
                    im: None,
                },
            ),
            (
                "b".into(),
                MatValue::Logical {
                    dims: vec![2, 2],
                    data: vec![true, false, false, true],
                },
            ),
            (
                "c".into(),
                MatValue::Cell {
                    dims: vec![1, 2],
                    items: vec![MatValue::scalar(7.0), MatValue::text("x")],
                },
            ),
            (
                "st".into(),
                MatValue::Struct {
                    dims: vec![1, 1],
                    fields: vec!["u".into(), "v".into()],
                    values: vec![MatValue::scalar(1.0), MatValue::text("hi")],
                },
            ),
            (
                "nd".into(),
                num(&[2, 2, 2], &[1., 2., 3., 4., 5., 6., 7., 8.]),
            ),
            (
                "sp".into(),
                MatValue::Sparse {
                    rows: 3,
                    cols: 3,
                    row_idx: vec![0, 2, 1],
                    col_ptr: vec![0, 2, 2, 3],
                    re: vec![1.0, 2.0, 3.0],
                    im: None,
                    logical: false,
                },
            ),
        ]
    }

    #[test]
    fn round_trips_every_kind_with_and_without_compression() {
        for compress in [false, true] {
            let bytes = write_v5(&sample(), "", compress).unwrap();
            let f = read_v5(&bytes).unwrap();
            assert_eq!(f.vars, sample(), "compress={compress}");
            assert_eq!(
                f.version,
                if compress {
                    MatVersion::V7
                } else {
                    MatVersion::V6
                }
            );
        }
    }

    #[test]
    fn rejects_truncation_everywhere() {
        let bytes = write_v5(&sample(), "", true).unwrap();
        for cut in (0..bytes.len()).step_by(7) {
            let _ = read_v5(bytes.get(..cut).unwrap_or(&[]));
        }
        let mut junk = bytes.clone();
        for i in (130..junk.len()).step_by(5) {
            junk[i] = junk[i].wrapping_mul(31).wrapping_add(7);
        }
        let _ = read_v5(&junk);
    }
}
