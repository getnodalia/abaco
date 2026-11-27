// SPDX-License-Identifier: MIT OR Apache-2.0
//! HDF5, the container of MATLAB's `-v7.3` files: a reader for what MATLAB and common tools
//! write, and a writer of the plain subset MATLAB reads back (version 0 superblock, version 1
//! object headers, symbol-table groups, contiguous datasets).

pub mod read;
pub mod write;

/// A member of a compound type.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// Name.
    pub name: String,
    /// Byte offset inside the compound.
    pub offset: usize,
    /// Its type.
    pub dtype: Dtype,
}

/// An HDF5 datatype, in the classes this crate handles.
#[derive(Clone, Debug, PartialEq)]
pub enum Dtype {
    /// An integer.
    Fixed {
        /// Bytes.
        size: usize,
        /// Signed?
        signed: bool,
        /// Little endian?
        le: bool,
    },
    /// An IEEE float.
    Float {
        /// Bytes (4 or 8).
        size: usize,
        /// Little endian?
        le: bool,
    },
    /// A fixed-length string.
    Str {
        /// Bytes.
        size: usize,
    },
    /// A compound.
    Compound {
        /// Bytes.
        size: usize,
        /// Members.
        members: Vec<Member>,
    },
    /// An object reference.
    Ref {
        /// Bytes (8).
        size: usize,
    },
    /// A variable-length sequence or string.
    Vlen {
        /// The element type.
        base: Box<Dtype>,
        /// `true` for a string.
        is_string: bool,
    },
}

impl Dtype {
    /// Bytes one element takes in a dataset.
    pub fn size(&self) -> usize {
        match self {
            Dtype::Fixed { size, .. }
            | Dtype::Float { size, .. }
            | Dtype::Str { size }
            | Dtype::Compound { size, .. }
            | Dtype::Ref { size } => *size,
            Dtype::Vlen { .. } => 16,
        }
    }

    /// A little-endian integer.
    pub fn int(size: usize, signed: bool) -> Dtype {
        Dtype::Fixed {
            size,
            signed,
            le: true,
        }
    }

    /// A little-endian float.
    pub fn float(size: usize) -> Dtype {
        Dtype::Float { size, le: true }
    }
}
