// SPDX-License-Identifier: MIT OR Apache-2.0
//! MAT-file level 7.3: HDF5 with MATLAB's conventions on top.
//!
//! A variable is a dataset or a group of the root, tagged with the attribute `MATLAB_class`.
//! Arrays are stored with their dimensions reversed (HDF5 is row-major, MATLAB column-major), so
//! the bytes are the same either way. Complex numbers are a compound of `real` and `imag`; empty
//! arrays are a `uint64` dataset of their dimensions flagged `MATLAB_empty`; a scalar struct is a
//! group of its fields; a struct array and a cell array hold object references into the group
//! `#refs#`; a sparse matrix is a group of `data`, `ir` and `jc`.

use super::{check_dims, numel, MatFile, MatValue, MatVersion, NumClass, MAX_DEPTH};
use crate::error::{corrupt, limit, unsupported, Result};
use crate::hdf5::read::{read_uint, Attr, Dataset, File, Kind};
use crate::hdf5::write::{Attribute, Writer};
use crate::hdf5::{Dtype, Member};

const MAX_OBJECTS: usize = 4_000_000;

struct Ctx<'a> {
    f: File<'a>,
    objects: usize,
    stack: Vec<u64>,
}

/// Reads a `-v7.3` MAT-file.
pub fn read_v73(bytes: &[u8]) -> Result<MatFile> {
    let f = File::open(bytes)?;
    let description = if bytes.get(..8) == Some(b"\x89HDF\r\n\x1a\n") {
        String::new()
    } else {
        let head = bytes.get(..116).unwrap_or(&[]);
        String::from_utf8_lossy(head)
            .trim_end_matches(['\0', ' '])
            .to_string()
    };
    let root = f.root();
    let members = f.members(root)?;
    let mut ctx = Ctx {
        f,
        objects: 0,
        stack: Vec::new(),
    };
    let mut vars = Vec::new();
    for (name, addr) in members {
        if name.starts_with('#') {
            continue;
        }
        let v = read_object(&mut ctx, addr, 0)?;
        vars.push((name, v));
    }
    Ok(MatFile {
        version: MatVersion::V73,
        description,
        vars,
    })
}

fn attr<'b>(attrs: &'b [Attr], name: &str) -> Option<&'b Attr> {
    attrs.iter().find(|a| a.name == name)
}

fn attr_text(attrs: &[Attr], name: &str) -> Option<String> {
    let a = attr(attrs, name)?;
    match &a.dtype {
        Dtype::Str { .. } => {
            let end = a.data.iter().position(|&b| b == 0).unwrap_or(a.data.len());
            Some(String::from_utf8_lossy(a.data.get(..end).unwrap_or(&[])).into_owned())
        }
        Dtype::Fixed { size: 2, le, .. } => {
            let u: Vec<u16> = a
                .data
                .chunks_exact(2)
                .map(|c| read_uint(c, 2, *le) as u16)
                .take_while(|&c| c != 0)
                .collect();
            Some(String::from_utf16_lossy(&u))
        }
        _ => None,
    }
}

fn attr_uint(attrs: &[Attr], name: &str) -> Option<u64> {
    let a = attr(attrs, name)?;
    match &a.dtype {
        Dtype::Fixed { size, le, .. } if a.data.len() >= *size => {
            Some(read_uint(&a.data, *size, *le))
        }
        _ => None,
    }
}

fn field_names(ctx: &Ctx, attrs: &[Attr]) -> Result<Option<Vec<String>>> {
    let Some(a) = attr(attrs, "MATLAB_fields") else {
        return Ok(None);
    };
    let Dtype::Vlen { base, .. } = &a.dtype else {
        return Ok(None);
    };
    let n = a.dims.iter().product::<usize>().min(a.data.len() / 16);
    let mut out = Vec::new();
    for i in 0..n {
        let e = a.data.get(i * 16..i * 16 + 16).unwrap_or(&[]);
        let get = |k: usize| e.get(k).copied().unwrap_or(0);
        let addr = u64::from_le_bytes([
            get(4),
            get(5),
            get(6),
            get(7),
            get(8),
            get(9),
            get(10),
            get(11),
        ]);
        let index = u32::from_le_bytes([get(12), get(13), get(14), get(15)]);
        let bytes = ctx.f.global_heap_object(addr, index)?;
        let name = if base.size() == 2 {
            let u: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&u)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        out.push(name);
    }
    Ok(Some(out))
}

fn mat_dims(hdims: &[usize]) -> Vec<usize> {
    let mut d: Vec<usize> = hdims.iter().rev().copied().collect();
    match d.len() {
        0 => d = vec![1, 1],
        1 => d.push(1),
        _ => {}
    }
    d
}

fn decode(dt: &Dtype, b: &[u8]) -> f64 {
    match dt {
        Dtype::Fixed { size, signed, le } => {
            let u = read_uint(b, *size, *le);
            if *signed && *size < 8 && *size > 0 {
                let shift = 64 - 8 * *size as u32;
                (((u << shift) as i64) >> shift) as f64
            } else if *signed {
                u as i64 as f64
            } else {
                u as f64
            }
        }
        Dtype::Float { size, le } => {
            let u = read_uint(b, *size, *le);
            if *size == 4 {
                f64::from(f32::from_bits(u as u32))
            } else {
                f64::from_bits(u)
            }
        }
        _ => 0.0,
    }
}

fn numbers(ds: &Dataset, n: usize) -> Result<(Vec<f64>, Option<Vec<f64>>)> {
    let sz = ds.dtype.size().max(1);
    match &ds.dtype {
        Dtype::Fixed { .. } | Dtype::Float { .. } => {
            let v = (0..n)
                .map(|i| decode(&ds.dtype, ds.raw.get(i * sz..(i + 1) * sz).unwrap_or(&[])))
                .collect();
            Ok((v, None))
        }
        Dtype::Compound { members, .. } => {
            let find = |name: &str| {
                members
                    .iter()
                    .find(|m: &&Member| m.name.eq_ignore_ascii_case(name))
            };
            let (Some(r), Some(i)) = (find("real"), find("imag")) else {
                return unsupported("a compound dataset that is not complex");
            };
            let part = |m: &Member| -> Vec<f64> {
                (0..n)
                    .map(|k| {
                        let o = k * sz + m.offset;
                        decode(&m.dtype, ds.raw.get(o..o + m.dtype.size()).unwrap_or(&[]))
                    })
                    .collect()
            };
            Ok((part(r), Some(part(i))))
        }
        _ => unsupported("a dataset of a type that is not a number"),
    }
}

fn class_of_dtype(dt: &Dtype) -> NumClass {
    match dt {
        Dtype::Float { size: 4, .. } => NumClass::Single,
        Dtype::Fixed { size, signed, .. } => match (*size, *signed) {
            (1, true) => NumClass::Int8,
            (1, false) => NumClass::UInt8,
            (2, true) => NumClass::Int16,
            (2, false) => NumClass::UInt16,
            (4, true) => NumClass::Int32,
            (4, false) => NumClass::UInt32,
            (8, true) => NumClass::Int64,
            _ => NumClass::UInt64,
        },
        Dtype::Compound { members, .. } => members
            .first()
            .map_or(NumClass::Double, |m| class_of_dtype(&m.dtype)),
        _ => NumClass::Double,
    }
}

fn empty_of(class: Option<&str>, dims: Vec<usize>, fields: Option<Vec<String>>) -> MatValue {
    match class {
        Some("logical") => MatValue::Logical {
            dims,
            data: Vec::new(),
        },
        Some("char") => MatValue::Char {
            dims,
            data: Vec::new(),
        },
        Some("cell") => MatValue::Cell {
            dims,
            items: Vec::new(),
        },
        Some("struct") => MatValue::Struct {
            dims,
            fields: fields.unwrap_or_default(),
            values: Vec::new(),
        },
        Some(c) => match NumClass::from_name(c) {
            Some(class) => MatValue::Numeric {
                class,
                dims,
                re: Vec::new(),
                im: None,
            },
            None => MatValue::Opaque {
                class: c.to_string(),
            },
        },
        None => MatValue::Numeric {
            class: NumClass::Double,
            dims,
            re: Vec::new(),
            im: None,
        },
    }
}

fn read_object(ctx: &mut Ctx, addr: u64, depth: usize) -> Result<MatValue> {
    if depth > MAX_DEPTH {
        return limit("cells and structs are nested too deeply");
    }
    ctx.objects += 1;
    if ctx.objects > MAX_OBJECTS {
        return limit("the file holds too many objects");
    }
    if ctx.stack.contains(&addr) {
        return corrupt("an object refers to itself");
    }
    ctx.stack.push(addr);
    let r = read_object_inner(ctx, addr, depth);
    ctx.stack.pop();
    r
}

fn read_object_inner(ctx: &mut Ctx, addr: u64, depth: usize) -> Result<MatValue> {
    let attrs = ctx.f.attrs(addr)?;
    let class = attr_text(&attrs, "MATLAB_class");
    match ctx.f.kind(addr)? {
        Kind::Dataset => read_dataset(ctx, addr, &attrs, class, depth),
        Kind::Group => read_group(ctx, addr, &attrs, class, depth),
    }
}

fn read_dataset(
    ctx: &mut Ctx,
    addr: u64,
    attrs: &[Attr],
    class: Option<String>,
    depth: usize,
) -> Result<MatValue> {
    let ds = ctx.f.dataset(addr)?;
    if attr_uint(attrs, "MATLAB_empty").unwrap_or(0) != 0 {
        let sz = ds.dtype.size().max(1);
        let dims: Vec<usize> = ds
            .raw
            .chunks_exact(sz)
            .map(|c| decode(&ds.dtype, c) as usize)
            .collect();
        let dims = if dims.len() < 2 { vec![0, 0] } else { dims };
        numel(&dims)?;
        let fields = field_names(ctx, attrs)?;
        return Ok(empty_of(class.as_deref(), dims, fields));
    }
    let dims = mat_dims(&ds.dims);
    let n = numel(&dims)?;
    match class.as_deref() {
        Some("cell") => {
            let mut items = Vec::new();
            for i in 0..n {
                let a = read_uint(ds.raw.get(i * 8..i * 8 + 8).unwrap_or(&[]), 8, true);
                items.push(read_object(ctx, a, depth + 1)?);
            }
            Ok(MatValue::Cell { dims, items })
        }
        Some("char") => Ok(MatValue::Char {
            dims,
            data: units(&ds, n),
        }),
        Some("logical") => {
            let (re, _) = numbers(&ds, n)?;
            Ok(MatValue::Logical {
                dims,
                data: re.iter().map(|&x| x != 0.0).collect(),
            })
        }
        Some(c) if NumClass::from_name(c).is_some() => {
            let (re, im) = numbers(&ds, n)?;
            Ok(MatValue::Numeric {
                class: NumClass::from_name(c).unwrap_or(NumClass::Double),
                dims,
                re,
                im,
            })
        }
        Some(c) => Ok(MatValue::Opaque {
            class: c.to_string(),
        }),
        None => match &ds.dtype {
            Dtype::Str { size } => {
                let text: Vec<u16> = ds
                    .raw
                    .iter()
                    .take(n * *size)
                    .map(|&b| u16::from(b))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .take_while(|&c| c != 0)
                    .collect();
                Ok(MatValue::Char {
                    dims: vec![usize::from(!text.is_empty()), text.len()],
                    data: text,
                })
            }
            Dtype::Ref { .. } => {
                let mut items = Vec::new();
                for i in 0..n {
                    let a = read_uint(ds.raw.get(i * 8..i * 8 + 8).unwrap_or(&[]), 8, true);
                    items.push(read_object(ctx, a, depth + 1)?);
                }
                Ok(MatValue::Cell { dims, items })
            }
            dt => {
                let (re, im) = numbers(&ds, n)?;
                Ok(MatValue::Numeric {
                    class: class_of_dtype(dt),
                    dims,
                    re,
                    im,
                })
            }
        },
    }
}

fn units(ds: &Dataset, n: usize) -> Vec<u16> {
    let sz = ds.dtype.size().max(1);
    let le = matches!(
        ds.dtype,
        Dtype::Fixed { le: true, .. } | Dtype::Float { le: true, .. }
    );
    (0..n)
        .map(|i| read_uint(ds.raw.get(i * sz..(i + 1) * sz).unwrap_or(&[]), sz, le) as u16)
        .collect()
}

fn read_group(
    ctx: &mut Ctx,
    addr: u64,
    attrs: &[Attr],
    class: Option<String>,
    depth: usize,
) -> Result<MatValue> {
    let members = ctx.f.members(addr)?;
    let find = |name: &str| members.iter().find(|(n, _)| n == name).map(|(_, a)| *a);
    if let Some(rows) = attr_uint(attrs, "MATLAB_sparse") {
        let (Some(d), Some(ir), Some(jc)) = (find("data"), find("ir"), find("jc")) else {
            return corrupt("a sparse matrix is missing data, ir or jc");
        };
        let data = ctx.f.dataset(d)?;
        let ir = ctx.f.dataset(ir)?;
        let jc = ctx.f.dataset(jc)?;
        let nnz = data.dims.iter().product::<usize>();
        let (re, im) = numbers(&data, nnz)?;
        let idx = |ds: &Dataset| -> Vec<usize> {
            let n = ds.dims.iter().product::<usize>();
            let sz = ds.dtype.size().max(1);
            (0..n)
                .map(|i| {
                    decode(&ds.dtype, ds.raw.get(i * sz..(i + 1) * sz).unwrap_or(&[])) as usize
                })
                .collect()
        };
        let row_idx = idx(&ir);
        let col_ptr = idx(&jc);
        let rows =
            usize::try_from(rows).map_or_else(|_| limit("a sparse matrix is too large"), Ok)?;
        if col_ptr.is_empty() || row_idx.len() < nnz || col_ptr.iter().any(|&p| p > nnz) {
            return corrupt("a sparse matrix has inconsistent indices");
        }
        if row_idx.iter().any(|&r| r >= rows.max(1)) {
            return corrupt("a sparse matrix has a row index out of range");
        }
        return Ok(MatValue::Sparse {
            rows,
            cols: col_ptr.len() - 1,
            row_idx: row_idx.into_iter().take(nnz).collect(),
            col_ptr,
            re,
            im,
            logical: class.as_deref() == Some("logical"),
        });
    }
    if class.as_deref() == Some("struct") || class.is_none() {
        let names = match field_names(ctx, attrs)? {
            Some(n) => n,
            None => members
                .iter()
                .filter(|(n, _)| !n.starts_with('#'))
                .map(|(n, _)| n.clone())
                .collect(),
        };
        let mut dims = vec![1, 1];
        let mut cols: Vec<Vec<MatValue>> = Vec::new();
        let mut array = false;
        for name in &names {
            let Some(a) = find(name) else {
                return corrupt(format!("the struct field `{name}` is missing"));
            };
            let fattrs = ctx.f.attrs(a)?;
            let is_ref_field = ctx.f.kind(a)? == Kind::Dataset
                && attr(&fattrs, "MATLAB_class").is_none()
                && matches!(ctx.f.dataset(a)?.dtype, Dtype::Ref { .. });
            if is_ref_field {
                array = true;
                let ds = ctx.f.dataset(a)?;
                dims = mat_dims(&ds.dims);
                let n = numel(&dims)?;
                let mut col = Vec::new();
                for i in 0..n {
                    let t = read_uint(ds.raw.get(i * 8..i * 8 + 8).unwrap_or(&[]), 8, true);
                    col.push(read_object(ctx, t, depth + 1)?);
                }
                cols.push(col);
            } else {
                cols.push(vec![read_object(ctx, a, depth + 1)?]);
            }
        }
        let count = if array { numel(&dims)? } else { 1 };
        let mut values = Vec::new();
        for e in 0..count {
            for col in &cols {
                values.push(col.get(e).cloned().unwrap_or(MatValue::scalar(0.0)));
            }
        }
        return Ok(MatValue::Struct {
            dims,
            fields: names,
            values,
        });
    }
    Ok(MatValue::Opaque {
        class: class.unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------------------------
// Writing

struct Out {
    w: Writer,
    refs: Option<usize>,
    next: usize,
}

fn dtype_of(class: NumClass) -> Dtype {
    match class {
        NumClass::Double => Dtype::float(8),
        NumClass::Single => Dtype::float(4),
        NumClass::Int8 => Dtype::int(1, true),
        NumClass::UInt8 => Dtype::int(1, false),
        NumClass::Int16 => Dtype::int(2, true),
        NumClass::UInt16 => Dtype::int(2, false),
        NumClass::Int32 => Dtype::int(4, true),
        NumClass::UInt32 => Dtype::int(4, false),
        NumClass::Int64 => Dtype::int(8, true),
        NumClass::UInt64 => Dtype::int(8, false),
    }
}

fn pack_one(class: NumClass, x: f64, out: &mut Vec<u8>) {
    match class {
        NumClass::Double => out.extend_from_slice(&x.to_le_bytes()),
        NumClass::Single => out.extend_from_slice(&(x as f32).to_le_bytes()),
        NumClass::Int8 => out.extend_from_slice(&(x as i8).to_le_bytes()),
        NumClass::UInt8 => out.extend_from_slice(&(x as u8).to_le_bytes()),
        NumClass::Int16 => out.extend_from_slice(&(x as i16).to_le_bytes()),
        NumClass::UInt16 => out.extend_from_slice(&(x as u16).to_le_bytes()),
        NumClass::Int32 => out.extend_from_slice(&(x as i32).to_le_bytes()),
        NumClass::UInt32 => out.extend_from_slice(&(x as u32).to_le_bytes()),
        NumClass::Int64 => out.extend_from_slice(&(x as i64).to_le_bytes()),
        NumClass::UInt64 => out.extend_from_slice(&(x as u64).to_le_bytes()),
    }
}

fn rev(dims: &[usize]) -> Vec<usize> {
    dims.iter().rev().copied().collect()
}

fn class_attr(c: &str) -> Attribute {
    Attribute::string("MATLAB_class", c)
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        return unsupported(format!("`{name}` cannot be a name in a -v7.3 file"));
    }
    Ok(())
}

fn complex_type(class: NumClass) -> Dtype {
    let e = class.size();
    Dtype::Compound {
        size: 2 * e,
        members: vec![
            Member {
                name: "real".into(),
                offset: 0,
                dtype: dtype_of(class),
            },
            Member {
                name: "imag".into(),
                offset: e,
                dtype: dtype_of(class),
            },
        ],
    }
}

fn numeric_node(
    o: &mut Out,
    class: NumClass,
    dims: &[usize],
    re: &[f64],
    im: Option<&[f64]>,
    cls: &str,
) -> usize {
    let mut raw = Vec::new();
    let dt = match im {
        Some(im) => {
            for (r, i) in re.iter().zip(im) {
                pack_one(class, *r, &mut raw);
                pack_one(class, *i, &mut raw);
            }
            complex_type(class)
        }
        None => {
            for &r in re {
                pack_one(class, r, &mut raw);
            }
            dtype_of(class)
        }
    };
    o.w.dataset(dt, rev(dims), raw, vec![class_attr(cls)])
}

fn empty_node(o: &mut Out, cls: &str, dims: &[usize], extra: Vec<Attribute>) -> usize {
    let mut raw = Vec::new();
    for &d in dims {
        raw.extend_from_slice(&(d as u64).to_le_bytes());
    }
    let mut attrs = vec![class_attr(cls), Attribute::u32("MATLAB_empty", 1)];
    attrs.extend(extra);
    o.w.dataset(Dtype::int(8, false), vec![dims.len()], raw, attrs)
}

fn refs_group(o: &mut Out) -> usize {
    if let Some(r) = o.refs {
        return r;
    }
    let g = o.w.group(Vec::new());
    o.refs = Some(g);
    g
}

fn ref_object(o: &mut Out, v: &MatValue, depth: usize) -> Result<usize> {
    let id = build(o, v, depth + 1)?;
    let g = refs_group(o);
    o.next += 1;
    let name = format!("{:x}", o.next);
    o.w.link(g, &name, id);
    Ok(id)
}

fn build(o: &mut Out, v: &MatValue, depth: usize) -> Result<usize> {
    if depth > MAX_DEPTH {
        return limit("cells and structs are nested too deeply");
    }
    match v {
        MatValue::Numeric {
            class,
            dims,
            re,
            im,
        } => {
            check_dims(dims, re.len())?;
            if re.is_empty() {
                return Ok(empty_node(o, class.name(), dims, Vec::new()));
            }
            Ok(numeric_node(
                o,
                *class,
                dims,
                re,
                im.as_deref(),
                class.name(),
            ))
        }
        MatValue::Logical { dims, data } => {
            check_dims(dims, data.len())?;
            if data.is_empty() {
                return Ok(empty_node(o, "logical", dims, Vec::new()));
            }
            let raw = data.iter().map(|&b| u8::from(b)).collect();
            Ok(o.w.dataset(
                Dtype::int(1, false),
                rev(dims),
                raw,
                vec![class_attr("logical")],
            ))
        }
        MatValue::Char { dims, data } => {
            check_dims(dims, data.len())?;
            if data.is_empty() {
                return Ok(empty_node(o, "char", dims, Vec::new()));
            }
            let raw = data.iter().flat_map(|u| u.to_le_bytes()).collect();
            Ok(o.w.dataset(
                Dtype::int(2, false),
                rev(dims),
                raw,
                vec![class_attr("char"), Attribute::i32("MATLAB_int_decode", 2)],
            ))
        }
        MatValue::Cell { dims, items } => {
            check_dims(dims, items.len())?;
            if items.is_empty() {
                return Ok(empty_node(o, "cell", dims, Vec::new()));
            }
            let mut targets = Vec::new();
            for it in items {
                targets.push(ref_object(o, it, depth)?);
            }
            Ok(o.w.ref_dataset(rev(dims), targets, vec![class_attr("cell")]))
        }
        MatValue::Struct {
            dims,
            fields,
            values,
        } => {
            let n = numel(dims)?;
            if fields.len().checked_mul(n) != Some(values.len()) {
                return corrupt("a struct array does not hold one value per field and element");
            }
            for f in fields {
                check_name(f)?;
            }
            let fattr = || Attribute::strings("MATLAB_fields", fields);
            if n == 0 {
                return Ok(empty_node(
                    o,
                    "struct",
                    dims,
                    if fields.is_empty() {
                        Vec::new()
                    } else {
                        vec![fattr()]
                    },
                ));
            }
            let mut attrs = vec![class_attr("struct")];
            if !fields.is_empty() {
                attrs.push(fattr());
            }
            let g = o.w.group(attrs);
            let nf = fields.len();
            for (fi, name) in fields.iter().enumerate() {
                if n == 1 {
                    let child = build(o, &values[fi], depth + 1)?;
                    o.w.link(g, name, child);
                } else {
                    let mut targets = Vec::new();
                    for e in 0..n {
                        let val = values.get(e * nf + fi).ok_or_else(|| {
                            crate::error::FormatError::Corrupt("a struct value is missing".into())
                        })?;
                        targets.push(ref_object(o, val, depth)?);
                    }
                    let ds = o.w.ref_dataset(rev(dims), targets, Vec::new());
                    o.w.link(g, name, ds);
                }
            }
            Ok(g)
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
            if col_ptr.len() != cols + 1 || re.len() != row_idx.len() {
                return corrupt("a sparse matrix has inconsistent sizes");
            }
            let cls = if *logical { "logical" } else { "double" };
            let g = o.w.group(vec![
                class_attr(cls),
                Attribute::u64("MATLAB_sparse", *rows as u64),
            ]);
            let n = re.len();
            let data = if *logical {
                o.w.dataset(
                    Dtype::int(1, false),
                    vec![n],
                    re.iter().map(|&x| u8::from(x != 0.0)).collect(),
                    Vec::new(),
                )
            } else {
                let mut raw = Vec::new();
                let dt = match im {
                    Some(im) => {
                        for (r, i) in re.iter().zip(im) {
                            pack_one(NumClass::Double, *r, &mut raw);
                            pack_one(NumClass::Double, *i, &mut raw);
                        }
                        complex_type(NumClass::Double)
                    }
                    None => {
                        for &r in re {
                            pack_one(NumClass::Double, r, &mut raw);
                        }
                        Dtype::float(8)
                    }
                };
                o.w.dataset(dt, vec![n], raw, Vec::new())
            };
            let ir = o.w.dataset(
                Dtype::int(8, false),
                vec![n],
                row_idx
                    .iter()
                    .flat_map(|&i| (i as u64).to_le_bytes())
                    .collect(),
                Vec::new(),
            );
            let jc = o.w.dataset(
                Dtype::int(8, false),
                vec![col_ptr.len()],
                col_ptr
                    .iter()
                    .flat_map(|&i| (i as u64).to_le_bytes())
                    .collect(),
                Vec::new(),
            );
            o.w.link(g, "data", data);
            o.w.link(g, "ir", ir);
            o.w.link(g, "jc", jc);
            Ok(g)
        }
        MatValue::Opaque { class } => {
            unsupported(format!("a value of class {class} cannot be saved"))
        }
    }
}

/// Writes a `-v7.3` MAT-file.
pub fn write_v73(vars: &[(String, MatValue)], description: &str) -> Result<Vec<u8>> {
    let mut o = Out {
        w: Writer::new(),
        refs: None,
        next: 0,
    };
    let root = o.w.root();
    for (name, v) in vars {
        check_name(name)?;
        let id = build(&mut o, v, 0)?;
        o.w.link(root, name, id);
    }
    if let Some(r) = o.refs {
        o.w.link(root, "#refs#", r);
    }
    let text = if description.is_empty() {
        "MATLAB 7.3 MAT-file, written by Abaco, HDF5 schema 1.00 ."
    } else {
        description
    };
    let mut user: Vec<u8> = text.bytes().take(116).collect();
    user.resize(116, b' ');
    user.extend_from_slice(&[0u8; 8]);
    user.extend_from_slice(&0x0200u16.to_le_bytes());
    user.extend_from_slice(b"IM");
    user.resize(512, 0);
    o.w.finish(&user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbl(dims: &[usize], re: &[f64]) -> MatValue {
        MatValue::Numeric {
            class: NumClass::Double,
            dims: dims.to_vec(),
            re: re.to_vec(),
            im: None,
        }
    }

    fn sample() -> Vec<(String, MatValue)> {
        vec![
            ("a".into(), dbl(&[2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
            ("x".into(), MatValue::scalar(2.5)),
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
            ("s".into(), MatValue::text("hello")),
            (
                "b".into(),
                MatValue::Logical {
                    dims: vec![2, 2],
                    data: vec![true, false, false, true],
                },
            ),
            ("e".into(), dbl(&[0, 3], &[])),
            (
                "c".into(),
                MatValue::Cell {
                    dims: vec![1, 3],
                    items: vec![
                        MatValue::scalar(1.0),
                        MatValue::text("two"),
                        dbl(&[1, 2], &[3.0, 4.0]),
                    ],
                },
            ),
            (
                "st".into(),
                MatValue::Struct {
                    dims: vec![1, 1],
                    fields: vec!["p".into(), "q".into()],
                    values: vec![MatValue::scalar(1.0), MatValue::text("hi")],
                },
            ),
            (
                "sa".into(),
                MatValue::Struct {
                    dims: vec![1, 2],
                    fields: vec!["u".into()],
                    values: vec![MatValue::scalar(10.0), MatValue::scalar(20.0)],
                },
            ),
            (
                "sp".into(),
                MatValue::Sparse {
                    rows: 3,
                    cols: 3,
                    row_idx: vec![0, 2, 1],
                    col_ptr: vec![0, 1, 2, 3],
                    re: vec![1.0, 2.0, 3.0],
                    im: None,
                    logical: false,
                },
            ),
        ]
    }

    #[test]
    fn round_trips() {
        let mut vars = sample();
        let bytes = write_v73(&vars, "").unwrap();
        let back = read_v73(&bytes).unwrap();
        // HDF5 groups are ordered by name, as in the files MATLAB writes.
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(back.vars.len(), vars.len());
        for (want, got) in vars.iter().zip(&back.vars) {
            assert_eq!(want.0, got.0);
            assert_eq!(want.1, got.1, "variable {}", want.0);
        }
    }

    #[test]
    fn many_variables_use_a_deep_tree() {
        let vars: Vec<(String, MatValue)> = (0..700)
            .map(|i| (format!("v{i:04}"), MatValue::scalar(i as f64)))
            .collect();
        let bytes = write_v73(&vars, "").unwrap();
        let back = read_v73(&bytes).unwrap();
        assert_eq!(back.vars.len(), 700);
        assert_eq!(back.vars[699].1, MatValue::scalar(699.0));
    }

    #[test]
    fn truncated_files_fail_cleanly() {
        let bytes = write_v73(&sample(), "").unwrap();
        for cut in (0..bytes.len()).step_by(97) {
            let _ = read_v73(&bytes[..cut]);
        }
    }
}
