// SPDX-License-Identifier: MIT OR Apache-2.0
//! DEFLATE (RFC 1951), zlib (RFC 1950), Adler-32 and CRC-32, written for this crate.
//!
//! [`inflate`] reads every kind of block (stored, fixed and dynamic Huffman) with a cap on the
//! output size, so a hostile stream cannot grow without bound. [`deflate`] writes LZ77 with
//! dynamic-free fixed Huffman codes (or stored blocks when that is smaller), which every reader
//! of the format, MATLAB's included, accepts.

use crate::error::{corrupt, limit, Result};

const MAX_BITS: usize = 15;

/// Reads bits least significant first.
struct Bits<'a> {
    buf: &'a [u8],
    pos: usize,
    bit: u32,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn new(buf: &'a [u8]) -> Bits<'a> {
        Bits {
            buf,
            pos: 0,
            bit: 0,
            cnt: 0,
        }
    }

    fn need(&mut self, n: u32) -> Result<u32> {
        while self.cnt < n {
            let Some(&b) = self.buf.get(self.pos) else {
                return corrupt("deflate stream ends early");
            };
            self.pos += 1;
            self.bit |= u32::from(b) << self.cnt;
            self.cnt += 8;
        }
        let v = if n == 32 {
            self.bit
        } else {
            self.bit & ((1u32 << n) - 1)
        };
        self.bit = if n == 32 { 0 } else { self.bit >> n };
        self.cnt -= n;
        Ok(v)
    }

    fn align(&mut self) {
        self.bit = 0;
        self.cnt = 0;
    }
}

/// A canonical Huffman code, decoded as in zlib's `puff`.
struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            if let Some(c) = count.get_mut(usize::from(l)) {
                *c += 1;
            }
        }
        let mut left: i32 = 1;
        for len in 1..=MAX_BITS {
            left <<= 1;
            left -= i32::from(count.get(len).copied().unwrap_or(0));
            if left < 0 {
                return corrupt("over-subscribed Huffman code");
            }
        }
        let mut offs = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            let prev = offs.get(len).copied().unwrap_or(0);
            let c = count.get(len).copied().unwrap_or(0);
            if let Some(o) = offs.get_mut(len + 1) {
                *o = prev + c;
            }
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let o = offs.get_mut(usize::from(l)).map(|o| {
                    let v = *o;
                    *o += 1;
                    v
                });
                if let Some(slot) = o.and_then(|o| symbol.get_mut(usize::from(o))) {
                    *slot = sym as u16;
                }
            }
        }
        Ok(Huffman { count, symbol })
    }

    fn decode(&self, b: &mut Bits) -> Result<u16> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=MAX_BITS {
            code |= b.need(1)? as i32;
            let count = i32::from(self.count.get(len).copied().unwrap_or(0));
            if code - count < first {
                return match self.symbol.get((index + (code - first)) as usize) {
                    Some(&s) => Ok(s),
                    None => corrupt("bad Huffman symbol"),
                };
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        corrupt("bad Huffman code")
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman, max: usize) -> Result<()> {
    loop {
        let sym = lit.decode(b)?;
        if sym < 256 {
            if out.len() >= max {
                return limit("inflated data is larger than the allowed size");
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let i = usize::from(sym - 257);
            let (Some(&base), Some(&extra)) = (LEN_BASE.get(i), LEN_EXTRA.get(i)) else {
                return corrupt("bad length symbol");
            };
            let len = usize::from(base) + b.need(u32::from(extra))? as usize;
            let ds = usize::from(dist.decode(b)?);
            let (Some(&dbase), Some(&dextra)) = (DIST_BASE.get(ds), DIST_EXTRA.get(ds)) else {
                return corrupt("bad distance symbol");
            };
            let d = usize::from(dbase) + b.need(u32::from(dextra))? as usize;
            if d > out.len() {
                return corrupt("distance reaches before the start of the data");
            }
            if out.len() + len > max {
                return limit("inflated data is larger than the allowed size");
            }
            let start = out.len() - d;
            for k in 0..len {
                let byte = out.get(start + k).copied().unwrap_or(0);
                out.push(byte);
            }
        }
    }
}

/// Decompresses a raw DEFLATE stream; the output may not exceed `max` bytes.
pub fn inflate(data: &[u8], max: usize) -> Result<Vec<u8>> {
    inflate_prefix(data, max).map(|(v, _)| v)
}

/// Like [`inflate`], also returning how many input bytes the stream used.
pub fn inflate_prefix(data: &[u8], max: usize) -> Result<(Vec<u8>, usize)> {
    let mut b = Bits::new(data);
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = b.need(1)?;
        let kind = b.need(2)?;
        match kind {
            0 => {
                b.align();
                let len = b.pos.checked_add(4).and_then(|e| data.get(b.pos..e));
                let Some(h) = len else {
                    return corrupt("stored block header is cut off");
                };
                let n = usize::from(u16::from_le_bytes([h[0], h[1]]));
                let nn = u16::from_le_bytes([h[2], h[3]]);
                if nn != !(n as u16) {
                    return corrupt("stored block length check failed");
                }
                b.pos += 4;
                let Some(chunk) = data.get(b.pos..b.pos + n) else {
                    return corrupt("stored block is cut off");
                };
                if out.len() + n > max {
                    return limit("inflated data is larger than the allowed size");
                }
                out.extend_from_slice(chunk);
                b.pos += n;
            }
            1 => {
                let mut l = [8u8; 288];
                for x in l.iter_mut().take(256).skip(144) {
                    *x = 9;
                }
                for x in l.iter_mut().take(280).skip(256) {
                    *x = 7;
                }
                let lit = Huffman::new(&l)?;
                let dist = Huffman::new(&[5u8; 30])?;
                codes(&mut b, &mut out, &lit, &dist, max)?;
            }
            2 => {
                let nlen = b.need(5)? as usize + 257;
                let ndist = b.need(5)? as usize + 1;
                let ncode = b.need(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return corrupt("too many Huffman codes");
                }
                const ORDER: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let mut cl = [0u8; 19];
                for &o in ORDER.iter().take(ncode) {
                    if let Some(s) = cl.get_mut(o) {
                        *s = b.need(3)? as u8;
                    }
                }
                let clh = Huffman::new(&cl)?;
                let mut lengths = vec![0u8; nlen + ndist];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = clh.decode(&mut b)?;
                    if sym < 16 {
                        if let Some(s) = lengths.get_mut(i) {
                            *s = sym as u8;
                        }
                        i += 1;
                    } else {
                        let (val, rep) = match sym {
                            16 => {
                                if i == 0 {
                                    return corrupt("repeat with no previous length");
                                }
                                (
                                    lengths.get(i - 1).copied().unwrap_or(0),
                                    3 + b.need(2)? as usize,
                                )
                            }
                            17 => (0, 3 + b.need(3)? as usize),
                            _ => (0, 11 + b.need(7)? as usize),
                        };
                        if i + rep > nlen + ndist {
                            return corrupt("too many code lengths");
                        }
                        for _ in 0..rep {
                            if let Some(s) = lengths.get_mut(i) {
                                *s = val;
                            }
                            i += 1;
                        }
                    }
                }
                if lengths.get(256).copied().unwrap_or(0) == 0 {
                    return corrupt("no end-of-block code");
                }
                let lit = Huffman::new(lengths.get(..nlen).unwrap_or(&[]))?;
                let dist = Huffman::new(lengths.get(nlen..).unwrap_or(&[]))?;
                codes(&mut b, &mut out, &lit, &dist, max)?;
            }
            _ => return corrupt("bad block type"),
        }
        if last == 1 {
            break;
        }
    }
    Ok((out, b.pos))
}

/// Adler-32 of `data`.
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// CRC-32 (the one ZIP and PNG use) of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, t) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &x in data {
        c = table
            .get(((c ^ u32::from(x)) & 0xFF) as usize)
            .copied()
            .unwrap_or(0)
            ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Decompresses a zlib stream (header, DEFLATE, Adler-32), checking the checksum.
pub fn zlib_decompress(data: &[u8], max: usize) -> Result<Vec<u8>> {
    let (Some(&cmf), Some(&flg)) = (data.first(), data.get(1)) else {
        return corrupt("zlib stream is too short");
    };
    if cmf & 0x0F != 8 || (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return corrupt("not a zlib stream");
    }
    if flg & 0x20 != 0 {
        return corrupt("zlib preset dictionaries are not supported");
    }
    let body = data.get(2..).unwrap_or(&[]);
    let (out, used) = inflate_prefix(body, max)?;
    if let Some(t) = body.get(used..used + 4) {
        let want = u32::from_be_bytes([t[0], t[1], t[2], t[3]]);
        if want != adler32(&out) {
            return corrupt("zlib checksum mismatch");
        }
    }
    Ok(out)
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, bits: u32) {
        self.acc |= u64::from(v) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Huffman codes go in most significant bit first.
    fn put_code(&mut self, code: u32, bits: u32) {
        let mut rev = 0;
        for i in 0..bits {
            if code & (1 << i) != 0 {
                rev |= 1 << (bits - 1 - i);
            }
        }
        self.put(rev, bits);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

fn put_lit(w: &mut BitWriter, sym: u32) {
    match sym {
        0..=143 => w.put_code(0x30 + sym, 8),
        144..=255 => w.put_code(0x190 + (sym - 144), 9),
        256..=279 => w.put_code(sym - 256, 7),
        _ => w.put_code(0xC0 + (sym - 280), 8),
    }
}

/// Compresses `data` into a raw DEFLATE stream.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    const HASH: usize = 1 << 15;
    const WINDOW: usize = 32768;
    const CHAIN: usize = 48;
    let mut w = BitWriter {
        out: Vec::with_capacity(data.len() / 2 + 16),
        acc: 0,
        n: 0,
    };
    w.put(1, 1);
    w.put(1, 2);
    let mut head = vec![usize::MAX; HASH];
    let mut prev = vec![usize::MAX; data.len()];
    let hash = |i: usize| -> usize {
        let a = usize::from(data.get(i).copied().unwrap_or(0));
        let b = usize::from(data.get(i + 1).copied().unwrap_or(0));
        let c = usize::from(data.get(i + 2).copied().unwrap_or(0));
        ((a << 10) ^ (b << 5) ^ c) & (HASH - 1)
    };
    let mut i = 0;
    while i < data.len() {
        let mut best_len = 0;
        let mut best_dist = 0;
        if i + 3 <= data.len() {
            let h = hash(i);
            let mut cand = head.get(h).copied().unwrap_or(usize::MAX);
            let mut steps = 0;
            while cand != usize::MAX && steps < CHAIN && i - cand <= WINDOW {
                let mut l = 0;
                while l < 258 && i + l < data.len() && data.get(cand + l) == data.get(i + l) {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = i - cand;
                    if l == 258 {
                        break;
                    }
                }
                cand = prev.get(cand).copied().unwrap_or(usize::MAX);
                steps += 1;
            }
        }
        let advance;
        if best_len >= 3 {
            let li = LEN_BASE
                .iter()
                .rposition(|&b| usize::from(b) <= best_len)
                .unwrap_or(0);
            put_lit(&mut w, 257 + li as u32);
            let extra = LEN_EXTRA.get(li).copied().unwrap_or(0);
            if extra > 0 {
                w.put(
                    (best_len - usize::from(LEN_BASE.get(li).copied().unwrap_or(0))) as u32,
                    u32::from(extra),
                );
            }
            let di = DIST_BASE
                .iter()
                .rposition(|&b| usize::from(b) <= best_dist)
                .unwrap_or(0);
            w.put_code(di as u32, 5);
            let dextra = DIST_EXTRA.get(di).copied().unwrap_or(0);
            if dextra > 0 {
                w.put(
                    (best_dist - usize::from(DIST_BASE.get(di).copied().unwrap_or(0))) as u32,
                    u32::from(dextra),
                );
            }
            advance = best_len;
        } else {
            put_lit(&mut w, u32::from(data.get(i).copied().unwrap_or(0)));
            advance = 1;
        }
        for k in 0..advance {
            if i + k + 3 <= data.len() {
                let h = hash(i + k);
                if let (Some(p), Some(hd)) = (prev.get_mut(i + k), head.get_mut(h)) {
                    *p = *hd;
                    *hd = i + k;
                }
            }
        }
        i += advance;
    }
    put_lit(&mut w, 256);
    let coded = w.finish();
    if coded.len() > data.len() + data.len() / 16000 * 5 + 5 {
        return stored(data);
    }
    coded
}

fn stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 5 * (data.len() / 65535 + 1));
    let mut chunks = data.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        out.push(last);
        let n = c.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(!n).to_le_bytes());
        out.extend_from_slice(c);
    }
    out
}

/// Compresses `data` into a zlib stream.
pub fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x9C];
    out.extend(deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(n: usize) -> Vec<u8> {
        let mut s = 12345u32;
        (0..n)
            .map(|i| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
                if i % 7 < 4 {
                    (i % 13) as u8
                } else {
                    (s >> 24) as u8
                }
            })
            .collect()
    }

    #[test]
    fn round_trip_many_sizes() {
        for n in [0, 1, 2, 3, 100, 257, 4096, 70_000] {
            let d = sample(n);
            let z = zlib_compress(&d);
            assert_eq!(zlib_decompress(&z, 1 << 24).unwrap(), d, "n={n}");
        }
    }

    #[test]
    fn repetitive_data_shrinks() {
        let d = vec![7u8; 100_000];
        assert!(zlib_compress(&d).len() < 2_000);
    }

    #[test]
    fn stored_blocks_round_trip() {
        let d = sample(200_000);
        assert_eq!(inflate(&stored(&d), 1 << 24).unwrap(), d);
    }

    #[test]
    fn known_stream_from_zlib() {
        // zlib.compress(b"hello hello hello hello", 9) with dynamic-free output.
        let z = [
            0x78, 0xDA, 0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0xC8, 0x40, 0x27, 0x01, 0x68, 0x03,
            0x08, 0xB1,
        ];
        assert_eq!(
            zlib_decompress(&z, 100).unwrap(),
            b"hello hello hello hello"
        );
    }

    #[test]
    fn checks_and_limits() {
        let z = zlib_compress(b"abcabcabcabc");
        let mut bad = z.clone();
        let n = bad.len();
        bad[n - 1] ^= 1;
        assert!(zlib_decompress(&bad, 100).is_err());
        assert!(zlib_decompress(&z, 5).is_err());
        assert!(zlib_decompress(&z[..z.len() - 6], 100).is_err());
        assert!(inflate(&[0xFF, 0xFF, 0xFF], 100).is_err());
    }

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
