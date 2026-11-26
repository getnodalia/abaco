// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! The Ábaco maths document: an ordered list of math and text cells, a dependency graph built
//! from the names each cell defines and uses, incremental evaluation in dependency order, and
//! the `.abaco` JSON file format.
//!
//! ```
//! use abaco_doc::Document;
//! let mut doc = Document::new();
//! let m = doc.push_math("m = 10 kg").unwrap();
//! let f = doc.push_math("F = m * g0 -> N").unwrap();
//! doc.evaluate();
//! let shown = doc.display(doc.result(f).unwrap().outcome.as_ref().unwrap().value.as_ref().unwrap());
//! assert_eq!(shown, "98.0665 N");
//! doc.set_source(m, "m = 20 kg").unwrap();
//! assert_eq!(doc.evaluate().len(), 2); // the edited cell and the one that uses it
//! ```

mod document;
mod format;
mod graph;
mod model;
mod report;

pub use document::Document;
pub use format::{EngineInfo, FormatError, FORMAT_NAME, FORMAT_VERSION, MAX_FILE_BYTES};
pub use model::{
    Cell, CellError, CellId, CellKind, CellOutput, CellResult, DocError, Settings,
    DEFAULT_SIGNIFICANT_DIGITS, MAX_CELLS, MAX_CELL_ID, MAX_SOURCE_BYTES, MAX_TEXT_BYTES,
};
pub use report::error_json;
