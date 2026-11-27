// SPDX-License-Identifier: MIT OR Apache-2.0
//! MAT-files: the formats MATLAB saves workspaces in.
//!
//! * **Level 4** (`-v4`): a flat list of matrices, little or big endian.
//! * **Level 5** (`-v6`, `-v7`): tagged data elements, optionally zlib-compressed.
//! * **Level 7.3** (`-v7.3`): an HDF5 file with MATLAB's conventions on top.
//!
//! [`read`] recognises which of them it was given; [`write`] produces any of them. Values go
//! through [`MatValue`], which is independent of any interpreter: numbers keep their class
//! (`int8` … `uint64`, `single`, `double`) and any number of dimensions.

mod hdf5;
mod v4;
mod v5;

use crate::error::{corrupt, unsupported, Result};

pub use hdf5::{read_v73, write_v73};
pub use v4::{read_v4, write_v4};
pub use v5::{read_v5, write_v5};

/// The most data one file may expand to.
pub const MAX_BYTES: usize = 1 << 30;
/// The deepest nesting of cells and structs accepted.
pub const MAX_DEPTH: usize = 64;
/// The most dimensions an array may have.
pub const MAX_DIMS: usize = 32;

/// The numeric classes of MATLAB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumClass {
    /// `double`
    Double,
    /// `single`
    Single,
    /// `int8`
    Int8,
    /// `uint8`
    UInt8,
    /// `int16`
    Int16,
    /// `uint16`
    UInt16,
    /// `int32`
    Int32,
    /// `uint32`
    UInt32,
    /// `int64`
    Int64,
    /// `uint64`
    UInt64,
}

impl NumClass {
    /// The name `class(x)` reports.
    pub fn name(self) -> &'static str {
        match self {
            NumClass::Double => "double",
            NumClass::Single => "single",
            NumClass::Int8 => "int8",
            NumClass::UInt8 => "uint8",
            NumClass::Int16 => "int16",
            NumClass::UInt16 => "uint16",
            NumClass::Int32 => "int32",
            NumClass::UInt32 => "uint32",
            NumClass::Int64 => "int64",
            NumClass::UInt64 => "uint64",
        }
    }

    /// The class named `s`.
    pub fn from_name(s: &str) -> Option<NumClass> {
        Some(match s {
            "double" => NumClass::Double,
            "single" => NumClass::Single,
            "int8" => NumClass::Int8,
            "uint8" => NumClass::UInt8,
            "int16" => NumClass::Int16,
            "uint16" => NumClass::UInt16,
            "int32" => NumClass::Int32,
            "uint32" => NumClass::UInt32,
            "int64" => NumClass::Int64,
            "uint64" => NumClass::UInt64,
            _ => return None,
        })
    }

    /// Bytes per element.
    pub fn size(self) -> usize {
        match self {
            NumClass::Int8 | NumClass::UInt8 => 1,
            NumClass::Int16 | NumClass::UInt16 => 2,
            NumClass::Int32 | NumClass::UInt32 | NumClass::Single => 4,
            NumClass::Int64 | NumClass::UInt64 | NumClass::Double => 8,
        }
    }
}

/// A value stored in a MAT-file.
#[derive(Clone, Debug, PartialEq)]
pub enum MatValue {
    /// A numeric array, column-major; `im` holds the imaginary parts of a complex array.
    /// Integers wider than 53 bits lose their low bits (the values are kept as `f64`).
    Numeric {
        /// The class.
        class: NumClass,
        /// The dimensions (two or more).
        dims: Vec<usize>,
        /// Real parts.
        re: Vec<f64>,
        /// Imaginary parts.
        im: Option<Vec<f64>>,
    },
    /// A logical array.
    Logical {
        /// The dimensions.
        dims: Vec<usize>,
        /// The values, column-major.
        data: Vec<bool>,
    },
    /// A character array as UTF-16 code units, column-major.
    Char {
        /// The dimensions.
        dims: Vec<usize>,
        /// The code units.
        data: Vec<u16>,
    },
    /// A cell array.
    Cell {
        /// The dimensions.
        dims: Vec<usize>,
        /// The cells, column-major.
        items: Vec<MatValue>,
    },
    /// A struct array. `values[e * fields.len() + f]` is field `f` of element `e`.
    Struct {
        /// The dimensions.
        dims: Vec<usize>,
        /// Field names in order.
        fields: Vec<String>,
        /// The values, element by element.
        values: Vec<MatValue>,
    },
    /// A sparse matrix in compressed-column form.
    Sparse {
        /// Rows.
        rows: usize,
        /// Columns.
        cols: usize,
        /// Row index of each stored value (0-based).
        row_idx: Vec<usize>,
        /// Start of each column in `row_idx` (`cols + 1` entries).
        col_ptr: Vec<usize>,
        /// Real parts.
        re: Vec<f64>,
        /// Imaginary parts.
        im: Option<Vec<f64>>,
        /// `true` for a sparse logical matrix.
        logical: bool,
    },
    /// Something this crate keeps by name only (function handles, objects, `string` arrays).
    Opaque {
        /// The MATLAB class name.
        class: String,
    },
}

impl MatValue {
    /// A real double scalar.
    pub fn scalar(x: f64) -> MatValue {
        MatValue::Numeric {
            class: NumClass::Double,
            dims: vec![1, 1],
            re: vec![x],
            im: None,
        }
    }

    /// A character row from text.
    pub fn text(s: &str) -> MatValue {
        let data: Vec<u16> = s.encode_utf16().collect();
        MatValue::Char {
            dims: vec![usize::from(!data.is_empty()), data.len()],
            data,
        }
    }

    /// The dimensions.
    pub fn dims(&self) -> Vec<usize> {
        match self {
            MatValue::Numeric { dims, .. }
            | MatValue::Logical { dims, .. }
            | MatValue::Char { dims, .. }
            | MatValue::Cell { dims, .. }
            | MatValue::Struct { dims, .. } => dims.clone(),
            MatValue::Sparse { rows, cols, .. } => vec![*rows, *cols],
            MatValue::Opaque { .. } => vec![1, 1],
        }
    }

    /// The text of a char array that is a row (or empty), if it is one.
    pub fn as_text(&self) -> Option<String> {
        match self {
            MatValue::Char { dims, data } if dims.len() == 2 && dims.first() != Some(&2) => {
                Some(String::from_utf16_lossy(data))
            }
            _ => None,
        }
    }
}

/// Which file format a MAT-file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatVersion {
    /// Level 4.
    V4,
    /// Level 5 without compression (`-v6`).
    V6,
    /// Level 5 with compression (`-v7`).
    V7,
    /// HDF5-based (`-v7.3`).
    V73,
}

/// A MAT-file: its variables in order.
#[derive(Clone, Debug, PartialEq)]
pub struct MatFile {
    /// The format it was read from or will be written as.
    pub version: MatVersion,
    /// The descriptive text in the header (level 5 and 7.3), empty for level 4.
    pub description: String,
    /// The variables.
    pub vars: Vec<(String, MatValue)>,
}

/// Reads a MAT-file of any level.
pub fn read(bytes: &[u8]) -> Result<MatFile> {
    if bytes.starts_with(b"\x89HDF\r\n\x1a\n") || bytes.get(512..520) == Some(b"\x89HDF\r\n\x1a\n")
    {
        return read_v73(bytes);
    }
    if bytes.len() >= 128 && matches!(bytes.get(126..128), Some(b"IM") | Some(b"MI")) {
        return read_v5(bytes);
    }
    if bytes.len() >= 20 {
        return read_v4(bytes);
    }
    corrupt("this is not a MAT-file")
}

/// Writes a MAT-file in the given level.
pub fn write(file: &MatFile) -> Result<Vec<u8>> {
    match file.version {
        MatVersion::V4 => write_v4(&file.vars),
        MatVersion::V6 => write_v5(&file.vars, &file.description, false),
        MatVersion::V7 => write_v5(&file.vars, &file.description, true),
        MatVersion::V73 => write_v73(&file.vars, &file.description),
    }
}

/// Checks that the dimensions describe `n` elements and are sane.
pub(crate) fn check_dims(dims: &[usize], n: usize) -> Result<()> {
    if dims.len() < 2 || dims.len() > MAX_DIMS {
        return unsupported("an array has fewer than 2 or more than 32 dimensions");
    }
    let mut total: usize = 1;
    for &d in dims {
        total = match total.checked_mul(d) {
            Some(t) if t <= MAX_BYTES / 8 => t,
            _ => return corrupt("array dimensions are too large"),
        };
    }
    if total != n {
        return corrupt("array dimensions do not match its data");
    }
    Ok(())
}

/// The number of elements of an array of these dimensions, within limits.
pub(crate) fn numel(dims: &[usize]) -> Result<usize> {
    if dims.len() < 2 || dims.len() > MAX_DIMS {
        return unsupported("an array has fewer than 2 or more than 32 dimensions");
    }
    let mut total: usize = 1;
    for &d in dims {
        total = match total.checked_mul(d) {
            Some(t) if t <= MAX_BYTES / 8 => t,
            _ => return corrupt("array dimensions are too large"),
        };
    }
    Ok(total)
}
