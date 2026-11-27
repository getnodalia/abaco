// SPDX-License-Identifier: MIT OR Apache-2.0
//! A reader for the parts of HDF5 that MATLAB and common tools write: superblocks 0 to 3,
//! object headers 1 and 2, groups by symbol table or by link messages, datasets that are
//! contiguous, compact or chunked (deflate and shuffle), attributes, object references and
//! variable-length strings.

use super::{Dtype, Member};
use crate::deflate::inflate;
use crate::error::{at, corrupt, limit, unsupported, Result};
use crate::mat::MAX_BYTES;
use std::collections::BTreeMap;

const UNDEF: u64 = u64::MAX;
const MAX_MESSAGES: usize = 100_000;
const MAX_LINKS: usize = 1_000_000;

/// One header message.
#[derive(Clone, Debug)]
pub struct Message {
    /// The message type.
    pub ty: u16,
    /// Its payload.
    pub data: Vec<u8>,
}

/// An attribute.
#[derive(Clone, Debug)]
pub struct Attr {
    /// Name.
    pub name: String,
    /// Element type.
    pub dtype: Dtype,
    /// Dimensions (empty for a scalar).
    pub dims: Vec<usize>,
    /// Raw bytes.
    pub data: Vec<u8>,
}

/// A dataset read into memory.
#[derive(Clone, Debug)]
pub struct Dataset {
    /// Element type.
    pub dtype: Dtype,
    /// Dimensions in HDF5 order (slowest first).
    pub dims: Vec<usize>,
    /// Raw bytes, `numel * dtype.size()` of them.
    pub raw: Vec<u8>,
}

/// What an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A group.
    Group,
    /// A dataset.
    Dataset,
}

type HeapObjects = Vec<(u32, Vec<u8>)>;

/// An open HDF5 file.
pub struct File<'a> {
    buf: &'a [u8],
    base: usize,
    root: u64,
    heaps: std::cell::RefCell<BTreeMap<u64, HeapObjects>>,
}

fn r64(b: &[u8], i: usize) -> u64 {
    let mut a = [0u8; 8];
    for (k, x) in a.iter_mut().enumerate() {
        *x = at(b, i + k);
    }
    u64::from_le_bytes(a)
}

fn r32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([at(b, i), at(b, i + 1), at(b, i + 2), at(b, i + 3)])
}

fn r16(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([at(b, i), at(b, i + 1)])
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    String::from_utf8_lossy(b.get(..end).unwrap_or(&[])).into_owned()
}

impl<'a> File<'a> {
    /// Opens `buf` as an HDF5 file (a user block before the signature is allowed).
    pub fn open(buf: &'a [u8]) -> Result<File<'a>> {
        const SIG: &[u8] = b"\x89HDF\r\n\x1a\n";
        let mut base = None;
        let mut p = 0usize;
        while p + 8 <= buf.len() {
            if buf.get(p..p + 8) == Some(SIG) {
                base = Some(p);
                break;
            }
            p = if p == 0 { 512 } else { p * 2 };
        }
        let Some(base) = base else {
            return corrupt("this is not an HDF5 file");
        };
        let sb = buf.get(base..).unwrap_or(&[]);
        let ver = at(sb, 8);
        let root = match ver {
            0 | 1 => {
                if at(sb, 13) != 8 || at(sb, 14) != 8 {
                    return unsupported("HDF5 files with offsets or lengths that are not 8 bytes");
                }
                let skip = if ver == 1 { 4 } else { 0 };
                // Root symbol table entry follows the four addresses.
                let entry = 24 + skip + 32;
                r64(sb, entry + 8)
            }
            2 | 3 => {
                if at(sb, 9) != 8 || at(sb, 10) != 8 {
                    return unsupported("HDF5 files with offsets or lengths that are not 8 bytes");
                }
                r64(sb, 12 + 8 + 8 + 8)
            }
            v => return unsupported(format!("HDF5 superblock version {v}")),
        };
        Ok(File {
            buf,
            base,
            root,
            heaps: Default::default(),
        })
    }

    /// The address of the root group.
    pub fn root(&self) -> u64 {
        self.root
    }

    fn slice(&self, addr: u64, len: usize) -> Result<&'a [u8]> {
        if addr == UNDEF {
            return corrupt("an undefined address was followed");
        }
        let start = usize::try_from(addr)
            .ok()
            .and_then(|a| a.checked_add(self.base));
        match start
            .and_then(|s| s.checked_add(len).map(|e| (s, e)))
            .and_then(|(s, e)| self.buf.get(s..e))
        {
            Some(s) => Ok(s),
            None => corrupt("an HDF5 address is outside the file"),
        }
    }

    fn tail(&self, addr: u64) -> Result<&'a [u8]> {
        if addr == UNDEF {
            return corrupt("an undefined address was followed");
        }
        let start = usize::try_from(addr)
            .ok()
            .and_then(|a| a.checked_add(self.base));
        match start.and_then(|s| self.buf.get(s..)) {
            Some(s) => Ok(s),
            None => corrupt("an HDF5 address is outside the file"),
        }
    }

    /// The messages of the object header at `addr`.
    pub fn header(&self, addr: u64) -> Result<Vec<Message>> {
        let h = self.tail(addr)?;
        let mut out = Vec::new();
        if h.get(..4) == Some(b"OHDR") {
            let flags = at(h, 5);
            let mut p = 6;
            if flags & 0x20 != 0 {
                p += 16;
            }
            if flags & 0x10 != 0 {
                p += 4;
            }
            let sz = 1usize << (flags & 3);
            let mut n = 0usize;
            for i in 0..sz {
                n |= usize::from(at(h, p + i)) << (8 * i);
            }
            p += sz;
            let body = h
                .get(p..p + n)
                .map_or_else(|| corrupt("a header is cut off"), Ok)?;
            self.v2_messages(body, flags, &mut out, 0)?;
        } else {
            if at(h, 0) != 1 {
                return corrupt("an object header has an unknown version");
            }
            let n = usize::from(r16(h, 2));
            let size = r32(h, 8) as usize;
            let body = h
                .get(16..16 + size)
                .map_or_else(|| corrupt("a header is cut off"), Ok)?;
            self.v1_messages(body, n, &mut out, 0)?;
        }
        Ok(out)
    }

    fn v1_messages(
        &self,
        body: &[u8],
        mut left: usize,
        out: &mut Vec<Message>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 {
            return limit("header continuations are nested too deeply");
        }
        let mut p = 0;
        while p + 8 <= body.len() && left > 0 {
            let ty = r16(body, p);
            let size = usize::from(r16(body, p + 2));
            let data = body
                .get(p + 8..p + 8 + size)
                .map_or_else(|| corrupt("a header message is cut off"), Ok)?;
            p += 8 + size;
            left = left.saturating_sub(1);
            if out.len() > MAX_MESSAGES {
                return limit("too many header messages");
            }
            if ty == 0x10 {
                let (off, len) = (r64(data, 0), r64(data, 8) as usize);
                let blk = self.slice(off, len)?;
                self.v1_messages(blk, usize::MAX, out, depth + 1)?;
            } else if ty != 0 {
                out.push(Message {
                    ty,
                    data: data.to_vec(),
                });
            }
        }
        Ok(())
    }

    fn v2_messages(
        &self,
        body: &[u8],
        hflags: u8,
        out: &mut Vec<Message>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 {
            return limit("header continuations are nested too deeply");
        }
        let track = hflags & 0x04 != 0;
        let mut p = 0;
        let end = body.len().saturating_sub(4);
        while p + 4 <= end {
            let ty = u16::from(at(body, p));
            let size = usize::from(r16(body, p + 1));
            p += 4 + if track { 2 } else { 0 };
            let data = body
                .get(p..p + size)
                .map_or_else(|| corrupt("a header message is cut off"), Ok)?;
            p += size;
            if out.len() > MAX_MESSAGES {
                return limit("too many header messages");
            }
            if ty == 0x10 {
                let (off, len) = (r64(data, 0), r64(data, 8) as usize);
                let blk = self.slice(off, len)?;
                if blk.get(..4) != Some(b"OCHK") {
                    return corrupt("a header continuation has no signature");
                }
                self.v2_messages(blk.get(4..).unwrap_or(&[]), hflags, out, depth + 1)?;
            } else if ty != 0 {
                out.push(Message {
                    ty,
                    data: data.to_vec(),
                });
            }
        }
        Ok(())
    }

    /// Whether the object at `addr` is a group or a dataset.
    pub fn kind(&self, addr: u64) -> Result<Kind> {
        let msgs = self.header(addr)?;
        if msgs.iter().any(|m| m.ty == 0x08) {
            Ok(Kind::Dataset)
        } else {
            Ok(Kind::Group)
        }
    }

    /// The attributes of the object at `addr`.
    pub fn attrs(&self, addr: u64) -> Result<Vec<Attr>> {
        let mut out = Vec::new();
        for m in self.header(addr)? {
            if m.ty == 0x0C {
                if let Some(a) = self.attribute(&m.data)? {
                    out.push(a);
                }
            }
        }
        Ok(out)
    }

    fn attribute(&self, d: &[u8]) -> Result<Option<Attr>> {
        let ver = at(d, 0);
        if !(1..=3).contains(&ver) {
            return Ok(None);
        }
        let (nsz, tsz, ssz) = (
            usize::from(r16(d, 2)),
            usize::from(r16(d, 4)),
            usize::from(r16(d, 6)),
        );
        let mut p = if ver == 3 { 9 } else { 8 };
        let pad = |n: usize| if ver == 1 { n.div_ceil(8) * 8 } else { n };
        let name = cstr(d.get(p..p + nsz).unwrap_or(&[]));
        p += pad(nsz);
        let (dtype, _) = parse_dtype(d.get(p..p + tsz).unwrap_or(&[]))?;
        p += pad(tsz);
        let dims = parse_dataspace(d.get(p..p + ssz).unwrap_or(&[]))?;
        p += pad(ssz);
        let n: usize = dims.iter().product();
        let need = n.saturating_mul(dtype.size());
        if need > MAX_BYTES {
            return limit("an attribute is too large");
        }
        let data = d.get(p..).unwrap_or(&[]);
        let data = data.get(..need.min(data.len())).unwrap_or(&[]).to_vec();
        Ok(Some(Attr {
            name,
            dtype,
            dims,
            data,
        }))
    }

    /// The members `(name, address)` of the group at `addr`, in storage order.
    pub fn members(&self, addr: u64) -> Result<Vec<(String, u64)>> {
        let msgs = self.header(addr)?;
        let mut out = Vec::new();
        for m in &msgs {
            match m.ty {
                0x11 => {
                    let (bt, heap) = (r64(&m.data, 0), r64(&m.data, 8));
                    self.walk_group_tree(bt, heap, &mut out, 0)?;
                }
                0x06 => {
                    if let Some(l) = link_message(&m.data)? {
                        out.push(l);
                    }
                }
                0x02 => {
                    // Link info: dense storage is not supported.
                    let flags = at(&m.data, 1);
                    let off = if flags & 1 != 0 { 10 } else { 2 };
                    let heap = r64(&m.data, off);
                    if heap != UNDEF && heap != 0 {
                        return unsupported("HDF5 groups with densely stored links");
                    }
                }
                _ => {}
            }
            if out.len() > MAX_LINKS {
                return limit("a group has too many members");
            }
        }
        Ok(out)
    }

    fn heap_name(&self, heap: u64, off: u64) -> Result<String> {
        let h = self.slice(heap, 32)?;
        if h.get(..4) != Some(b"HEAP") {
            return corrupt("a local heap has no signature");
        }
        let data = r64(h, 24);
        let seg = self.tail(data)?;
        let start = usize::try_from(off).unwrap_or(usize::MAX);
        Ok(cstr(seg.get(start..).unwrap_or(&[])))
    }

    fn walk_group_tree(
        &self,
        node: u64,
        heap: u64,
        out: &mut Vec<(String, u64)>,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return limit("a group B-tree is too deep");
        }
        let t = self.slice(node, 24)?;
        if t.get(..4) != Some(b"TREE") {
            return corrupt("a B-tree node has no signature");
        }
        let level = at(t, 5);
        let n = usize::from(r16(t, 6));
        let body = self.slice(node + 24, 8 + n * 16)?;
        for i in 0..n {
            let child = r64(body, 8 + i * 16);
            if level > 0 {
                self.walk_group_tree(child, heap, out, depth + 1)?;
            } else {
                let sn = self.slice(child, 8)?;
                if sn.get(..4) != Some(b"SNOD") {
                    return corrupt("a symbol node has no signature");
                }
                let cnt = usize::from(r16(sn, 6));
                let ents = self.slice(child + 8, cnt * 40)?;
                for e in 0..cnt {
                    let name = self.heap_name(heap, r64(ents, e * 40))?;
                    out.push((name, r64(ents, e * 40 + 8)));
                }
            }
            if out.len() > MAX_LINKS {
                return limit("a group has too many members");
            }
        }
        Ok(())
    }

    /// Reads the dataset at `addr` fully.
    pub fn dataset(&self, addr: u64) -> Result<Dataset> {
        let msgs = self.header(addr)?;
        let mut dtype = None;
        let mut dims = None;
        let mut layout = None;
        let mut filters: Vec<(u16, Vec<u32>)> = Vec::new();
        for m in &msgs {
            match m.ty {
                0x01 => dims = Some(parse_dataspace(&m.data)?),
                0x03 => dtype = Some(parse_dtype(&m.data)?.0),
                0x08 => layout = Some(m.data.clone()),
                0x0B => filters = parse_filters(&m.data)?,
                _ => {}
            }
        }
        let (Some(dtype), Some(dims), Some(layout)) = (dtype, dims, layout) else {
            return corrupt("a dataset is missing its type, shape or layout");
        };
        let n = dims
            .iter()
            .try_fold(1usize, |a, &d| a.checked_mul(d))
            .filter(|&n| n.saturating_mul(dtype.size().max(1)) <= MAX_BYTES)
            .map_or_else(|| limit("a dataset is too large"), Ok)?;
        let total = n * dtype.size();
        let raw = match (at(&layout, 0), at(&layout, 1)) {
            (3, 0) => {
                let sz = usize::from(r16(&layout, 2));
                layout.get(4..4 + sz).unwrap_or(&[]).to_vec()
            }
            (3, 1) => {
                let (a, s) = (r64(&layout, 2), r64(&layout, 10) as usize);
                if a == UNDEF || total == 0 {
                    vec![0u8; total]
                } else {
                    self.slice(a, s.min(total))?.to_vec()
                }
            }
            (3, 2) => self.chunked(&layout, &dims, dtype.size(), &filters, total)?,
            (v, c) => return unsupported(format!("HDF5 data layout version {v} class {c}")),
        };
        let mut raw = raw;
        raw.resize(total, 0);
        Ok(Dataset { dtype, dims, raw })
    }

    fn chunked(
        &self,
        layout: &[u8],
        dims: &[usize],
        esize: usize,
        filters: &[(u16, Vec<u32>)],
        total: usize,
    ) -> Result<Vec<u8>> {
        let nd = usize::from(at(layout, 2));
        let bt = r64(layout, 3);
        let mut cdims = Vec::new();
        for i in 0..nd {
            cdims.push(r32(layout, 11 + 4 * i) as usize);
        }
        cdims.pop();
        if nd != dims.len() + 1 || cdims.contains(&0) {
            return corrupt("a chunked dataset has inconsistent dimensions");
        }
        let mut out = vec![0u8; total];
        if bt == UNDEF || total == 0 {
            return Ok(out);
        }
        let mut chunks = Vec::new();
        self.chunk_tree(bt, nd, &mut chunks, 0)?;
        let chunk_elems: usize = cdims.iter().product();
        let chunk_bytes = chunk_elems.saturating_mul(esize);
        if chunk_bytes > MAX_BYTES {
            return limit("a chunk is too large");
        }
        for (offs, size, mask, addr) in chunks {
            let mut data = self.slice(addr, size)?.to_vec();
            for (i, (id, params)) in filters.iter().enumerate().rev() {
                if mask & (1 << i) != 0 {
                    continue;
                }
                data = match id {
                    1 => inflate(data.get(2..).unwrap_or(&[]), chunk_bytes.max(1))?,
                    2 => unshuffle(
                        &data,
                        params.first().copied().unwrap_or(esize as u32) as usize,
                    ),
                    other => return unsupported(format!("HDF5 filter {other}")),
                };
            }
            // Copy the chunk's elements to their place, clipping at the dataset edge.
            let rank = dims.len();
            let mut idx = vec![0usize; rank];
            for e in 0..chunk_elems {
                let mut rem = e;
                for d in (0..rank).rev() {
                    idx[d] = rem % cdims[d];
                    rem /= cdims[d];
                }
                let mut pos = 0usize;
                let mut inside = true;
                for d in 0..rank {
                    let g = offs.get(d).copied().unwrap_or(0) as usize + idx[d];
                    if g >= dims[d] {
                        inside = false;
                        break;
                    }
                    pos = pos * dims[d] + g;
                }
                if inside {
                    if let (Some(dst), Some(src)) = (
                        out.get_mut(pos * esize..(pos + 1) * esize),
                        data.get(e * esize..(e + 1) * esize),
                    ) {
                        dst.copy_from_slice(src);
                    }
                }
            }
        }
        Ok(out)
    }

    #[allow(clippy::type_complexity)]
    fn chunk_tree(
        &self,
        node: u64,
        nd: usize,
        out: &mut Vec<(Vec<u64>, usize, u32, u64)>,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return limit("a chunk B-tree is too deep");
        }
        let t = self.slice(node, 24)?;
        if t.get(..4) != Some(b"TREE") {
            return corrupt("a chunk B-tree node has no signature");
        }
        let level = at(t, 5);
        let n = usize::from(r16(t, 6));
        let key = 8 + 8 * nd;
        let body = self.slice(node + 24, n * (key + 8) + key)?;
        for i in 0..n {
            let kp = i * (key + 8);
            let size = r32(body, kp) as usize;
            let mask = r32(body, kp + 4);
            let offs: Vec<u64> = (0..nd - 1).map(|d| r64(body, kp + 8 + 8 * d)).collect();
            let child = r64(body, kp + key);
            if level > 0 {
                self.chunk_tree(child, nd, out, depth + 1)?;
            } else {
                out.push((offs, size, mask, child));
            }
            if out.len() > MAX_LINKS {
                return limit("a dataset has too many chunks");
            }
        }
        Ok(())
    }

    /// The bytes of the variable-length object `(heap address, index)`.
    pub fn global_heap_object(&self, addr: u64, index: u32) -> Result<Vec<u8>> {
        let mut heaps = self.heaps.borrow_mut();
        if let std::collections::btree_map::Entry::Vacant(e) = heaps.entry(addr) {
            let tail = self.tail(addr)?;
            if tail.get(..4) != Some(b"GCOL") {
                return corrupt("a global heap has no signature");
            }
            let size = (r64(tail, 8) as usize).min(tail.len());
            let mut objs = Vec::new();
            let mut p = 16;
            while p + 16 <= size {
                let idx = u32::from(r16(tail, p));
                let sz = r64(tail, p + 8) as usize;
                if idx == 0 {
                    break;
                }
                let d = tail.get(p + 16..p + 16 + sz).unwrap_or(&[]).to_vec();
                objs.push((idx, d));
                p += 16 + sz.div_ceil(8) * 8;
            }
            e.insert(objs);
        }
        Ok(heaps
            .get(&addr)
            .and_then(|o| o.iter().find(|(i, _)| *i == index))
            .map(|(_, d)| d.clone())
            .unwrap_or_default())
    }
}

fn unshuffle(data: &[u8], size: usize) -> Vec<u8> {
    if size <= 1 || data.is_empty() {
        return data.to_vec();
    }
    let n = data.len() / size;
    let mut out = vec![0u8; data.len()];
    for i in 0..n {
        for b in 0..size {
            if let (Some(d), Some(s)) = (out.get_mut(i * size + b), data.get(b * n + i)) {
                *d = *s;
            }
        }
    }
    let tail = n * size;
    if let (Some(d), Some(s)) = (out.get_mut(tail..), data.get(tail..)) {
        d.copy_from_slice(s);
    }
    out
}

fn link_message(d: &[u8]) -> Result<Option<(String, u64)>> {
    let flags = at(d, 1);
    let mut p = 2;
    let ty = if flags & 0x08 != 0 {
        p += 1;
        at(d, 2)
    } else {
        0
    };
    if flags & 0x04 != 0 {
        p += 8;
    }
    if flags & 0x10 != 0 {
        p += 1;
    }
    let sz = 1usize << (flags & 3);
    let mut n = 0usize;
    for i in 0..sz {
        n |= usize::from(at(d, p + i)) << (8 * i);
    }
    p += sz;
    let name = String::from_utf8_lossy(d.get(p..p + n).unwrap_or(&[])).into_owned();
    p += n;
    if ty == 0 {
        Ok(Some((name, r64(d, p))))
    } else {
        Ok(None)
    }
}

fn parse_filters(d: &[u8]) -> Result<Vec<(u16, Vec<u32>)>> {
    let ver = at(d, 0);
    let n = usize::from(at(d, 1));
    let mut p = if ver == 1 { 8 } else { 2 };
    let mut out = Vec::new();
    for _ in 0..n {
        let id = r16(d, p);
        p += 2;
        let nlen = if ver == 1 || id >= 256 {
            let v = usize::from(r16(d, p));
            p += 2;
            v
        } else {
            0
        };
        p += 2; // flags
        let nv = usize::from(r16(d, p));
        p += 2;
        p += if ver == 1 { nlen.div_ceil(8) * 8 } else { nlen };
        let mut vals = Vec::new();
        for _ in 0..nv {
            vals.push(r32(d, p));
            p += 4;
        }
        if ver == 1 && nv % 2 == 1 {
            p += 4;
        }
        out.push((id, vals));
    }
    Ok(out)
}

/// Parses a dataspace message into its dimensions (empty for a scalar).
pub fn parse_dataspace(d: &[u8]) -> Result<Vec<usize>> {
    let ver = at(d, 0);
    let rank = usize::from(at(d, 1));
    if rank > 32 {
        return corrupt("a dataspace has too many dimensions");
    }
    let (start, null) = match ver {
        1 => (8, false),
        2 => (4, at(d, 3) == 2),
        v => return unsupported(format!("dataspace version {v}")),
    };
    if null {
        return Ok(vec![0]);
    }
    let mut dims = Vec::new();
    for i in 0..rank {
        let v = r64(d, start + 8 * i);
        dims.push(usize::try_from(v).map_or_else(|_| limit("a dimension is too large"), Ok)?);
    }
    Ok(dims)
}

/// Parses a datatype message; returns the type and the number of bytes it used.
pub fn parse_dtype(d: &[u8]) -> Result<(Dtype, usize)> {
    let class = at(d, 0) & 0x0F;
    let ver = at(d, 0) >> 4;
    let bf = [at(d, 1), at(d, 2), at(d, 3)];
    let size = r32(d, 4) as usize;
    match class {
        0 => Ok((
            Dtype::Fixed {
                size,
                signed: bf[0] & 0x08 != 0,
                le: bf[0] & 1 == 0,
            },
            12,
        )),
        1 => Ok((
            Dtype::Float {
                size,
                le: bf[0] & 1 == 0,
            },
            20,
        )),
        3 => Ok((Dtype::Str { size }, 8)),
        6 => {
            let nm = usize::from(r16(&bf, 0));
            let mut p = 8;
            let mut members = Vec::new();
            for _ in 0..nm {
                let end = d
                    .get(p..)
                    .and_then(|s| s.iter().position(|&b| b == 0))
                    .map_or_else(|| corrupt("a compound member has no name"), Ok)?;
                let name = String::from_utf8_lossy(d.get(p..p + end).unwrap_or(&[])).into_owned();
                p += if ver == 3 {
                    end + 1
                } else {
                    (end + 1).div_ceil(8) * 8
                };
                let off = if ver == 3 {
                    let w = if size <= 0xFF {
                        1
                    } else if size <= 0xFFFF {
                        2
                    } else if size <= 0xFF_FFFF {
                        3
                    } else {
                        4
                    };
                    let mut v = 0usize;
                    for i in 0..w {
                        v |= usize::from(at(d, p + i)) << (8 * i);
                    }
                    p += w;
                    v
                } else {
                    let v = r32(d, p) as usize;
                    p += 4;
                    v
                };
                if ver == 1 {
                    p += 28;
                }
                let (dt, used) = parse_dtype(d.get(p..).unwrap_or(&[]))?;
                p += used;
                members.push(Member {
                    name,
                    offset: off,
                    dtype: dt,
                });
            }
            Ok((Dtype::Compound { size, members }, p))
        }
        7 => Ok((Dtype::Ref { size }, 8)),
        9 => {
            let is_string = bf[0] & 0x0F == 1;
            let (base, used) = parse_dtype(d.get(8..).unwrap_or(&[]))?;
            Ok((
                Dtype::Vlen {
                    base: Box::new(base),
                    is_string,
                },
                8 + used,
            ))
        }
        c => unsupported(format!("HDF5 datatype class {c}")),
    }
}

/// A cursor-free helper so callers can read little-endian integers of 1 to 8 bytes.
pub fn read_uint(b: &[u8], size: usize, le: bool) -> u64 {
    let mut v = 0u64;
    for i in 0..size.min(8) {
        let byte = if le { at(b, i) } else { at(b, size - 1 - i) };
        v |= u64::from(byte) << (8 * i);
    }
    v
}
