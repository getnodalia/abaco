// SPDX-License-Identifier: MIT OR Apache-2.0
//! A writer for the plain subset of HDF5 that MATLAB reads: a version 0 superblock, version 1
//! object headers, groups kept as symbol tables (local heap, B-tree, symbol nodes) and
//! contiguous datasets. Object references and variable-length attribute values are supported
//! because MATLAB's cell arrays, struct arrays and `MATLAB_fields` need them.
//!
//! Every size is independent of the addresses written, so the file is laid out twice: once to
//! learn where everything goes, once to write the addresses.

use super::Dtype;
use crate::error::{limit, Result};

const UNDEF: u64 = u64::MAX;
const LEAF_ENTRIES: usize = 8; // 2 * the group leaf node K of 4
const TREE_CHILDREN: usize = 32; // 2 * the group internal node K of 16
const SNOD_SIZE: usize = 8 + LEAF_ENTRIES * 40;
const TREE_SIZE: usize = 24 + TREE_CHILDREN * 8 + (TREE_CHILDREN + 1) * 8;
const SUPERBLOCK: usize = 96;

/// The value of an attribute.
#[derive(Clone, Debug)]
pub enum AttrData {
    /// Raw element bytes.
    Raw(Vec<u8>),
    /// One variable-length sequence of bytes per element.
    Vlen(Vec<Vec<u8>>),
}

/// An attribute to write.
#[derive(Clone, Debug)]
pub struct Attribute {
    /// Name.
    pub name: String,
    /// Element type.
    pub dtype: Dtype,
    /// Dimensions (empty for a scalar).
    pub dims: Vec<usize>,
    /// Value.
    pub data: AttrData,
}

impl Attribute {
    /// A string attribute.
    pub fn string(name: &str, s: &str) -> Attribute {
        let b = s.as_bytes().to_vec();
        Attribute {
            name: name.into(),
            dtype: Dtype::Str {
                size: b.len().max(1),
            },
            dims: Vec::new(),
            data: AttrData::Raw(if b.is_empty() { vec![0] } else { b }),
        }
    }

    /// A scalar `uint32`.
    pub fn u32(name: &str, v: u32) -> Attribute {
        Attribute {
            name: name.into(),
            dtype: Dtype::int(4, false),
            dims: Vec::new(),
            data: AttrData::Raw(v.to_le_bytes().to_vec()),
        }
    }

    /// A scalar `int32`.
    pub fn i32(name: &str, v: i32) -> Attribute {
        Attribute {
            name: name.into(),
            dtype: Dtype::int(4, true),
            dims: Vec::new(),
            data: AttrData::Raw(v.to_le_bytes().to_vec()),
        }
    }

    /// A scalar `uint64`.
    pub fn u64(name: &str, v: u64) -> Attribute {
        Attribute {
            name: name.into(),
            dtype: Dtype::int(8, false),
            dims: Vec::new(),
            data: AttrData::Raw(v.to_le_bytes().to_vec()),
        }
    }

    /// A list of strings, each a variable-length sequence of one-byte characters.
    pub fn strings(name: &str, items: &[String]) -> Attribute {
        Attribute {
            name: name.into(),
            dtype: Dtype::Vlen {
                base: Box::new(Dtype::Str { size: 1 }),
                is_string: false,
            },
            dims: vec![items.len()],
            data: AttrData::Vlen(items.iter().map(|s| s.as_bytes().to_vec()).collect()),
        }
    }
}

enum Body {
    Group(Vec<(String, usize)>),
    Dataset {
        dtype: Dtype,
        dims: Vec<usize>,
        raw: Vec<u8>,
        refs: Option<Vec<usize>>,
    },
}

struct Node {
    body: Body,
    attrs: Vec<Attribute>,
}

/// An HDF5 file being built. Node 0 is the root group.
pub struct Writer {
    nodes: Vec<Node>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    /// A file holding an empty root group.
    pub fn new() -> Writer {
        Writer {
            nodes: vec![Node {
                body: Body::Group(Vec::new()),
                attrs: Vec::new(),
            }],
        }
    }

    /// The root group.
    pub fn root(&self) -> usize {
        0
    }

    /// Adds a group and returns its id (it is not yet a member of anything).
    pub fn group(&mut self, attrs: Vec<Attribute>) -> usize {
        self.nodes.push(Node {
            body: Body::Group(Vec::new()),
            attrs,
        });
        self.nodes.len() - 1
    }

    /// Adds a dataset of `dtype` with `dims` (HDF5 order) and its raw bytes.
    pub fn dataset(
        &mut self,
        dtype: Dtype,
        dims: Vec<usize>,
        raw: Vec<u8>,
        attrs: Vec<Attribute>,
    ) -> usize {
        self.nodes.push(Node {
            body: Body::Dataset {
                dtype,
                dims,
                raw,
                refs: None,
            },
            attrs,
        });
        self.nodes.len() - 1
    }

    /// Adds a dataset of object references to other nodes.
    pub fn ref_dataset(
        &mut self,
        dims: Vec<usize>,
        targets: Vec<usize>,
        attrs: Vec<Attribute>,
    ) -> usize {
        self.nodes.push(Node {
            body: Body::Dataset {
                dtype: Dtype::Ref { size: 8 },
                dims,
                raw: Vec::new(),
                refs: Some(targets),
            },
            attrs,
        });
        self.nodes.len() - 1
    }

    /// Makes `child` a member of `parent` under `name`.
    pub fn link(&mut self, parent: usize, name: &str, child: usize) {
        if let Some(Node {
            body: Body::Group(m),
            ..
        }) = self.nodes.get_mut(parent)
        {
            m.push((name.to_string(), child));
        }
    }

    /// Serialises the file after `userblock` (empty, or a multiple of 512 bytes).
    pub fn finish(&self, userblock: &[u8]) -> Result<Vec<u8>> {
        let zero = Layout {
            addrs: vec![0; self.nodes.len()],
            gheap: 0,
        };
        let (_, first) = self.pass(userblock, &zero)?;
        let (bytes, _) = self.pass(userblock, &first)?;
        Ok(bytes)
    }

    fn pass(&self, userblock: &[u8], prev: &Layout) -> Result<(Vec<u8>, Layout)> {
        let ub = userblock.len();
        let mut out = userblock.to_vec();
        out.resize(ub + SUPERBLOCK, 0);
        let mut addrs = vec![0u64; self.nodes.len()];
        let mut heap_objs: Vec<Vec<u8>> = Vec::new();
        for (id, node) in self.nodes.iter().enumerate() {
            align8(&mut out);
            let at = rel(&out, ub);
            if let Some(a) = addrs.get_mut(id) {
                *a = at;
            }
            match &node.body {
                Body::Dataset {
                    dtype,
                    dims,
                    raw,
                    refs,
                } => {
                    let data: Vec<u8> = match refs {
                        Some(t) => t
                            .iter()
                            .flat_map(|&i| prev.addrs.get(i).copied().unwrap_or(0).to_le_bytes())
                            .collect(),
                        None => raw.clone(),
                    };
                    let mut msgs = Vec::new();
                    let mut count = 0u16;
                    put_msg(&mut msgs, 0x01, &dataspace(dims))?;
                    put_msg(&mut msgs, 0x03, &datatype(dtype))?;
                    put_msg(&mut msgs, 0x05, &[2, 2, 2, 0])?;
                    count += 3;
                    for a in &node.attrs {
                        put_msg(&mut msgs, 0x0C, &attribute(a, prev.gheap, &mut heap_objs)?)?;
                        count += 1;
                    }
                    let header_len = 16 + msgs.len() + 32;
                    let data_addr = if data.is_empty() {
                        UNDEF
                    } else {
                        at + header_len as u64
                    };
                    let mut layout = vec![3u8, 1];
                    layout.extend_from_slice(&data_addr.to_le_bytes());
                    layout.extend_from_slice(&(data.len() as u64).to_le_bytes());
                    put_msg(&mut msgs, 0x08, &layout)?;
                    count += 1;
                    out.extend(header(&msgs, count));
                    out.extend(data);
                }
                Body::Group(members) => {
                    let mut attr_msgs = Vec::new();
                    let mut count = 1u16;
                    for a in &node.attrs {
                        put_msg(
                            &mut attr_msgs,
                            0x0C,
                            &attribute(a, prev.gheap, &mut heap_objs)?,
                        )?;
                        count += 1;
                    }
                    let header_len = 16 + 24 + attr_msgs.len();
                    let heap_addr = at + header_len as u64;
                    let mut sorted: Vec<&(String, usize)> = members.iter().collect();
                    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
                    // The local heap: an empty name first, then each name padded to 8.
                    let mut seg = vec![0u8; 8];
                    let mut offs = Vec::new();
                    for (name, _) in &sorted {
                        offs.push(seg.len() as u64);
                        seg.extend_from_slice(name.as_bytes());
                        seg.push(0);
                        while !seg.len().is_multiple_of(8) {
                            seg.push(0);
                        }
                    }
                    let seg_addr = heap_addr + 32;
                    let snod_start = seg_addr + seg.len() as u64;
                    let n_snods = sorted.len().div_ceil(LEAF_ENTRIES).max(1);
                    // Tree levels above the symbol nodes.
                    let mut level_counts = Vec::new();
                    let mut n = n_snods;
                    loop {
                        let trees = n.div_ceil(TREE_CHILDREN);
                        level_counts.push(trees);
                        if trees == 1 {
                            break;
                        }
                        n = trees;
                    }
                    let tree_start = snod_start + (n_snods * SNOD_SIZE) as u64;
                    let total_trees: usize = level_counts.iter().sum();
                    let root_tree = tree_start + ((total_trees - 1) * TREE_SIZE) as u64;
                    let mut msgs = Vec::new();
                    let mut st = root_tree.to_le_bytes().to_vec();
                    st.extend_from_slice(&heap_addr.to_le_bytes());
                    put_msg(&mut msgs, 0x11, &st)?;
                    msgs.extend(attr_msgs);
                    out.extend(header(&msgs, count));
                    // Local heap.
                    out.extend_from_slice(b"HEAP");
                    out.extend_from_slice(&[0, 0, 0, 0]);
                    out.extend_from_slice(&(seg.len() as u64).to_le_bytes());
                    out.extend_from_slice(&1u64.to_le_bytes());
                    out.extend_from_slice(&seg_addr.to_le_bytes());
                    out.extend_from_slice(&seg);
                    // Symbol nodes.
                    let mut level: Vec<(u64, u64)> = Vec::new();
                    for k in 0..n_snods {
                        let lo = k * LEAF_ENTRIES;
                        let hi = (lo + LEAF_ENTRIES).min(sorted.len());
                        let mut node_bytes = Vec::with_capacity(SNOD_SIZE);
                        node_bytes.extend_from_slice(b"SNOD");
                        node_bytes.push(1);
                        node_bytes.push(0);
                        node_bytes.extend_from_slice(&((hi - lo.min(hi)) as u16).to_le_bytes());
                        let mut last = 0u64;
                        for i in lo..hi {
                            let child = sorted.get(i).map(|e| e.1).unwrap_or(0);
                            let off = offs.get(i).copied().unwrap_or(0);
                            last = off;
                            node_bytes.extend_from_slice(&off.to_le_bytes());
                            node_bytes.extend_from_slice(
                                &prev.addrs.get(child).copied().unwrap_or(0).to_le_bytes(),
                            );
                            node_bytes.extend_from_slice(&[0u8; 24]);
                        }
                        node_bytes.resize(SNOD_SIZE, 0);
                        level.push((snod_start + (k * SNOD_SIZE) as u64, last));
                        out.extend(node_bytes);
                    }
                    // B-tree levels.
                    let mut lvl = 0u8;
                    loop {
                        let mut next = Vec::new();
                        for chunk in level.chunks(TREE_CHILDREN) {
                            let mut t = Vec::with_capacity(TREE_SIZE);
                            t.extend_from_slice(b"TREE");
                            t.push(0);
                            t.push(lvl);
                            t.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
                            t.extend_from_slice(&UNDEF.to_le_bytes());
                            t.extend_from_slice(&UNDEF.to_le_bytes());
                            t.extend_from_slice(&0u64.to_le_bytes());
                            let mut last = 0;
                            for (addr, max) in chunk {
                                t.extend_from_slice(&addr.to_le_bytes());
                                t.extend_from_slice(&max.to_le_bytes());
                                last = *max;
                            }
                            t.resize(TREE_SIZE, 0);
                            next.push((rel(&out, ub), last));
                            out.extend(t);
                        }
                        if next.len() <= 1 {
                            break;
                        }
                        level = next;
                        lvl += 1;
                    }
                }
            }
        }
        // The global heap holding variable-length attribute values.
        let mut gheap = 0u64;
        if !heap_objs.is_empty() {
            if heap_objs.len() > 0xFFFE {
                return limit("too many variable-length attribute values");
            }
            align8(&mut out);
            gheap = rel(&out, ub);
            let mut body = Vec::new();
            for (i, o) in heap_objs.iter().enumerate() {
                body.extend_from_slice(&((i + 1) as u16).to_le_bytes());
                body.extend_from_slice(&1u16.to_le_bytes());
                body.extend_from_slice(&[0u8; 4]);
                body.extend_from_slice(&(o.len() as u64).to_le_bytes());
                body.extend_from_slice(o);
                while body.len() % 8 != 0 {
                    body.push(0);
                }
            }
            let size = (body.len() + 16 + 16).max(4096);
            out.extend_from_slice(b"GCOL");
            out.extend_from_slice(&[1, 0, 0, 0]);
            out.extend_from_slice(&(size as u64).to_le_bytes());
            let used = body.len();
            out.extend(body);
            // Object 0 is the free space: its size counts its own 16-byte header.
            out.extend_from_slice(&[0u8; 8]);
            out.extend_from_slice(&((size - 16 - used) as u64).to_le_bytes());
            out.resize(ub + gheap as usize + size, 0);
        }
        // The superblock, now that the end of the file is known.
        let eof = out.len() as u64;
        let root = addrs.first().copied().unwrap_or(0);
        let mut sb = Vec::with_capacity(SUPERBLOCK);
        sb.extend_from_slice(b"\x89HDF\r\n\x1a\n");
        sb.extend_from_slice(&[0, 0, 0, 0, 0, 8, 8, 0]);
        sb.extend_from_slice(&4u16.to_le_bytes());
        sb.extend_from_slice(&16u16.to_le_bytes());
        sb.extend_from_slice(&0u32.to_le_bytes());
        sb.extend_from_slice(&(ub as u64).to_le_bytes());
        sb.extend_from_slice(&UNDEF.to_le_bytes());
        sb.extend_from_slice(&eof.to_le_bytes());
        sb.extend_from_slice(&UNDEF.to_le_bytes());
        sb.extend_from_slice(&0u64.to_le_bytes());
        sb.extend_from_slice(&root.to_le_bytes());
        sb.extend_from_slice(&[0u8; 8 + 16]);
        if let Some(dst) = out.get_mut(ub..ub + SUPERBLOCK) {
            dst.copy_from_slice(&sb);
        }
        Ok((out, Layout { addrs, gheap }))
    }
}

struct Layout {
    addrs: Vec<u64>,
    gheap: u64,
}

fn rel(out: &[u8], ub: usize) -> u64 {
    (out.len() - ub) as u64
}

fn align8(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(8) {
        out.push(0);
    }
}

fn padded(mut v: Vec<u8>) -> Vec<u8> {
    while !v.len().is_multiple_of(8) {
        v.push(0);
    }
    v
}

fn put_msg(out: &mut Vec<u8>, ty: u16, data: &[u8]) -> Result<()> {
    let n = data.len().div_ceil(8) * 8;
    if n > 0xFFFF {
        return limit("an HDF5 header message is too large (too many fields or members)");
    }
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(data);
    out.resize(out.len() + (n - data.len()), 0);
    Ok(())
}

fn header(msgs: &[u8], count: u16) -> Vec<u8> {
    let mut h = vec![1u8, 0];
    h.extend_from_slice(&count.to_le_bytes());
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&(msgs.len() as u32).to_le_bytes());
    h.extend_from_slice(&[0u8; 4]);
    h.extend_from_slice(msgs);
    h
}

fn dataspace(dims: &[usize]) -> Vec<u8> {
    let mut d = vec![1u8, dims.len() as u8, 0, 0, 0, 0, 0, 0];
    for &x in dims {
        d.extend_from_slice(&(x as u64).to_le_bytes());
    }
    d
}

fn datatype(t: &Dtype) -> Vec<u8> {
    let mut d = Vec::new();
    match t {
        Dtype::Fixed { size, signed, le } => {
            d.push(0x10);
            d.push(u8::from(!le) | if *signed { 0x08 } else { 0 });
            d.extend_from_slice(&[0, 0]);
            d.extend_from_slice(&(*size as u32).to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&((*size * 8) as u16).to_le_bytes());
        }
        Dtype::Float { size, le } => {
            let (sign, ebits, ebias, mbits) = if *size == 4 {
                (31u8, 8u8, 127u32, 23u8)
            } else {
                (63, 11, 1023, 52)
            };
            d.push(0x11);
            d.push(0x20 | u8::from(!le));
            d.push(sign);
            d.push(0);
            d.extend_from_slice(&(*size as u32).to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&((*size * 8) as u16).to_le_bytes());
            d.extend_from_slice(&[mbits, ebits, 0, mbits]);
            d.extend_from_slice(&ebias.to_le_bytes());
        }
        Dtype::Str { size } => {
            d.push(0x13);
            d.push(1); // null padded, ASCII
            d.extend_from_slice(&[0, 0]);
            d.extend_from_slice(&(*size as u32).to_le_bytes());
        }
        Dtype::Ref { size } => {
            d.push(0x17);
            d.extend_from_slice(&[0, 0, 0]);
            d.extend_from_slice(&(*size as u32).to_le_bytes());
        }
        Dtype::Vlen { base, is_string } => {
            d.push(0x19);
            d.push(u8::from(*is_string));
            d.extend_from_slice(&[0, 0]);
            d.extend_from_slice(&16u32.to_le_bytes());
            d.extend(datatype(base));
        }
        Dtype::Compound { size, members } => {
            d.push(0x16);
            d.extend_from_slice(&(members.len() as u16).to_le_bytes());
            d.push(0);
            d.extend_from_slice(&(*size as u32).to_le_bytes());
            for m in members {
                let mut name = m.name.as_bytes().to_vec();
                name.push(0);
                d.extend(padded(name));
                d.extend_from_slice(&(m.offset as u32).to_le_bytes());
                d.extend_from_slice(&[0u8; 28]);
                d.extend(datatype(&m.dtype));
            }
        }
    }
    d
}

fn attribute(a: &Attribute, gheap: u64, heap: &mut Vec<Vec<u8>>) -> Result<Vec<u8>> {
    let mut name = a.name.as_bytes().to_vec();
    name.push(0);
    let nsz = name.len();
    let t = datatype(&a.dtype);
    let s = dataspace(&a.dims);
    let data: Vec<u8> = match &a.data {
        AttrData::Raw(r) => r.clone(),
        AttrData::Vlen(items) => {
            let mut v = Vec::new();
            for it in items {
                heap.push(it.clone());
                v.extend_from_slice(&(it.len() as u32).to_le_bytes());
                v.extend_from_slice(&gheap.to_le_bytes());
                v.extend_from_slice(&(heap.len() as u32).to_le_bytes());
            }
            v
        }
    };
    let mut out = vec![1u8, 0];
    out.extend_from_slice(&(nsz as u16).to_le_bytes());
    out.extend_from_slice(&(t.len() as u16).to_le_bytes());
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend(padded(name));
    out.extend(padded(t));
    out.extend(padded(s));
    out.extend(data);
    Ok(out)
}
