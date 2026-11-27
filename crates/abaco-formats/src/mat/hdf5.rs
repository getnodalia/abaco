// SPDX-License-Identifier: MIT OR Apache-2.0
//! MAT-file level 7.3: HDF5 with MATLAB's conventions.

use super::{MatFile, MatValue};
use crate::error::{unsupported, Result};

/// Reads a `-v7.3` MAT-file.
pub fn read_v73(_bytes: &[u8]) -> Result<MatFile> {
    unsupported("MAT-files of level 7.3")
}

/// Writes a `-v7.3` MAT-file.
pub fn write_v73(_vars: &[(String, MatValue)], _description: &str) -> Result<Vec<u8>> {
    unsupported("MAT-files of level 7.3")
}
