// SPDX-License-Identifier: MIT OR Apache-2.0
//! ZIP archives (the container of `.slx` files): reading of stored and deflated entries with
//! checksums and size limits, and writing.

use crate::deflate::{crc32, deflate, inflate};
use crate::error::{at, corrupt, limit, unsupported, Result};

/// The most entries an archive may hold.
pub const MAX_ENTRIES: usize = 100_000;
/// The most one entry may expand to.
pub const MAX_ENTRY_BYTES: usize = 256 << 20;

/// One entry of an archive.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Path inside the archive.
    pub name: String,
    method: u16,
    crc: u32,
    csize: usize,
    size: usize,
    offset: usize,
}

/// A ZIP archive held in memory.
pub struct Archive<'a> {
    buf: &'a [u8],
    entries: Vec<Entry>,
}

fn r16(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([at(b, i), at(b, i + 1)])
}

fn r32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([at(b, i), at(b, i + 1), at(b, i + 2), at(b, i + 3)])
}

impl<'a> Archive<'a> {
    /// Opens `buf` as a ZIP archive.
    pub fn open(buf: &'a [u8]) -> Result<Archive<'a>> {
        let lo = buf.len().saturating_sub(22 + 65535);
        let mut eocd = None;
        if buf.len() >= 22 {
            let mut i = buf.len() - 22;
            loop {
                if buf.get(i..i + 4) == Some(b"PK\x05\x06") {
                    eocd = Some(i);
                    break;
                }
                if i <= lo {
                    break;
                }
                i -= 1;
            }
        }
        let Some(e) = eocd else {
            return corrupt("this is not a ZIP archive");
        };
        let count = usize::from(r16(buf, e + 10));
        let cd_size = r32(buf, e + 12) as usize;
        let cd_off = r32(buf, e + 16) as usize;
        if count == 0xFFFF || cd_off == 0xFFFF_FFFF {
            return unsupported("ZIP64 archives");
        }
        if count > MAX_ENTRIES {
            return limit("the archive has too many entries");
        }
        let cd = cd_off
            .checked_add(cd_size)
            .and_then(|end| buf.get(cd_off..end))
            .map_or_else(|| corrupt("the central directory is outside the file"), Ok)?;
        let mut entries = Vec::new();
        let mut p = 0usize;
        for _ in 0..count {
            if cd.get(p..p + 4) != Some(b"PK\x01\x02") || p + 46 > cd.len() {
                return corrupt("a central directory entry is damaged");
            }
            let flags = r16(cd, p + 8);
            let method = r16(cd, p + 10);
            let crc = r32(cd, p + 16);
            let csize = r32(cd, p + 20) as usize;
            let size = r32(cd, p + 24) as usize;
            let nlen = usize::from(r16(cd, p + 28));
            let elen = usize::from(r16(cd, p + 30));
            let clen = usize::from(r16(cd, p + 32));
            let offset = r32(cd, p + 42) as usize;
            let name_bytes = cd
                .get(p + 46..p + 46 + nlen)
                .map_or_else(|| corrupt("an entry name is cut off"), Ok)?;
            if flags & 1 != 0 {
                return unsupported("encrypted ZIP entries");
            }
            entries.push(Entry {
                name: String::from_utf8_lossy(name_bytes).into_owned(),
                method,
                crc,
                csize,
                size,
                offset,
            });
            p += 46 + nlen + elen + clen;
        }
        Ok(Archive { buf, entries })
    }

    /// The entries, in archive order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether an entry with this name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    /// The uncompressed bytes of the entry `name`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let Some(e) = self.entries.iter().find(|e| e.name == name) else {
            return corrupt(format!("the archive has no entry `{name}`"));
        };
        self.extract(e)
    }

    /// The uncompressed bytes of `e`.
    pub fn extract(&self, e: &Entry) -> Result<Vec<u8>> {
        if e.size > MAX_ENTRY_BYTES {
            return limit(format!("the entry `{}` is too large", e.name));
        }
        let h = self
            .buf
            .get(e.offset..e.offset + 30)
            .filter(|h| h.get(..4) == Some(b"PK\x03\x04"))
            .map_or_else(|| corrupt("a local header is damaged"), Ok)?;
        let start = e.offset + 30 + usize::from(r16(h, 26)) + usize::from(r16(h, 28));
        let data = start
            .checked_add(e.csize)
            .and_then(|end| self.buf.get(start..end))
            .map_or_else(|| corrupt("an entry is cut off"), Ok)?;
        let out = match e.method {
            0 => data.to_vec(),
            8 => inflate(data, e.size)?,
            m => return unsupported(format!("ZIP compression method {m}")),
        };
        if out.len() != e.size {
            return corrupt(format!("the entry `{}` has the wrong size", e.name));
        }
        if crc32(&out) != e.crc {
            return corrupt(format!("the entry `{}` fails its checksum", e.name));
        }
        Ok(out)
    }
}

/// Builds a ZIP archive.
#[derive(Default)]
pub struct Writer {
    body: Vec<u8>,
    central: Vec<u8>,
    count: usize,
}

impl Writer {
    /// An empty archive.
    pub fn new() -> Writer {
        Writer::default()
    }

    /// Adds a file, deflated when that makes it smaller.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<()> {
        if self.count >= 0xFFFE || data.len() >= 0xFFFF_FFFF || self.body.len() >= 0xFFFF_FFF0 {
            return limit("the archive is too large");
        }
        let crc = crc32(data);
        let packed = deflate(data);
        let (method, payload): (u16, &[u8]) = if packed.len() < data.len() {
            (8, &packed)
        } else {
            (0, data)
        };
        let flags: u16 = if name.is_ascii() { 0 } else { 0x0800 };
        let offset = self.body.len() as u32;
        let mut local = Vec::new();
        local.extend_from_slice(b"PK\x03\x04");
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&flags.to_le_bytes());
        local.extend_from_slice(&method.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes()); // time
        local.extend_from_slice(&0x0021u16.to_le_bytes()); // date: 1980-01-01
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        self.body.extend(local);
        self.body.extend_from_slice(payload);
        let c = &mut self.central;
        c.extend_from_slice(b"PK\x01\x02");
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&flags.to_le_bytes());
        c.extend_from_slice(&method.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0x0021u16.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0u8; 2 + 2 + 2 + 2 + 4]);
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        self.count += 1;
        Ok(())
    }

    /// The finished archive.
    pub fn finish(self) -> Vec<u8> {
        let mut out = self.body;
        let cd_off = out.len() as u32;
        let cd_len = self.central.len() as u32;
        out.extend(self.central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&(self.count as u16).to_le_bytes());
        out.extend_from_slice(&(self.count as u16).to_le_bytes());
        out.extend_from_slice(&cd_len.to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = Writer::new();
        let big: Vec<u8> = (0..5000u32).flat_map(|i| (i % 17).to_le_bytes()).collect();
        w.add("a.txt", b"hello").unwrap();
        w.add("dir/b.bin", &big).unwrap();
        w.add("ñ.txt", b"").unwrap();
        let bytes = w.finish();
        let z = Archive::open(&bytes).unwrap();
        assert_eq!(z.entries().len(), 3);
        assert_eq!(z.read("a.txt").unwrap(), b"hello");
        assert_eq!(z.read("dir/b.bin").unwrap(), big);
        assert_eq!(z.read("ñ.txt").unwrap(), b"");
        assert!(z.read("missing").is_err());
    }

    #[test]
    fn damage_is_an_error_not_a_panic() {
        let mut w = Writer::new();
        w.add("a", &vec![7u8; 4000]).unwrap();
        let bytes = w.finish();
        for cut in 0..bytes.len() {
            if let Ok(z) = Archive::open(&bytes[..cut]) {
                let _ = z.read("a");
            }
        }
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0xFF;
            if let Ok(z) = Archive::open(&b) {
                let _ = z.read("a");
            }
        }
    }
}
