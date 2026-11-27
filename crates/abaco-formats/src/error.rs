// SPDX-License-Identifier: MIT OR Apache-2.0
//! The error type shared by every reader and writer in this crate.

/// A file could not be read or written.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FormatError {
    /// The bytes are not valid for the format (truncated, corrupt, wrong magic).
    #[error("{0}")]
    Corrupt(String),
    /// The file is valid but uses something this crate does not implement.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A size or depth limit was exceeded; files are never trusted.
    #[error("limit exceeded: {0}")]
    Limit(String),
}

/// A result with [`FormatError`].
pub type Result<T> = std::result::Result<T, FormatError>;

/// A corruption error.
pub fn corrupt<T>(msg: impl Into<String>) -> Result<T> {
    Err(FormatError::Corrupt(msg.into()))
}

/// An unsupported-feature error.
pub fn unsupported<T>(msg: impl Into<String>) -> Result<T> {
    Err(FormatError::Unsupported(msg.into()))
}

/// A limit error.
pub fn limit<T>(msg: impl Into<String>) -> Result<T> {
    Err(FormatError::Limit(msg.into()))
}

/// A bounds-checked little reader over a byte slice.
#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    /// The bytes.
    pub buf: &'a [u8],
    /// The next position.
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    /// A cursor at the start of `buf`.
    pub fn new(buf: &'a [u8]) -> Cursor<'a> {
        Cursor { buf, pos: 0 }
    }

    /// Bytes left.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// The next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n);
        match end.and_then(|e| self.buf.get(self.pos..e)) {
            Some(s) => {
                self.pos += n;
                Ok(s)
            }
            None => corrupt("unexpected end of file"),
        }
    }

    /// One byte.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    /// A little-endian u16.
    pub fn u16_le(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([at(b, 0), at(b, 1)]))
    }

    /// A little-endian u32.
    pub fn u32_le(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([at(b, 0), at(b, 1), at(b, 2), at(b, 3)]))
    }

    /// A little-endian u64.
    pub fn u64_le(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }
}

/// `b[i]`, or 0 when out of range (callers have already checked the length).
pub fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}
