// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `.abaco` file format: JSON, version 1.
//!
//! ```json
//! {
//!   "format": "abaco",
//!   "format_version": 1,
//!   "settings": { "significant_digits": 6, "strict_order": false },
//!   "next_cell_id": 3,
//!   "cells": [
//!     { "type": "math", "id": 1, "source": "x = 3 m" },
//!     { "type": "text", "id": 2, "markdown": "Some *prose*." }
//!   ],
//!   "engine": { "abaco-doc": "0.1.0", "abaco-expr": "0.1.0", "nodalia-units": "0.1.0" }
//! }
//! ```
//!
//! Keys are written in the order above and cells in page order, so saving the same document
//! twice gives identical bytes. Results are not stored: they are recomputed when a document
//! opens. `engine` records which versions wrote the file and is ignored on read.
//!
//! Unknown fields in a version-1 file are ignored on read and are not preserved when the
//! document is saved again; that is why a file with a newer `format_version` is refused instead
//! of being read partially.

use crate::document::Document;
use crate::model::{
    Cell, CellId, CellKind, Settings, DEFAULT_SIGNIFICANT_DIGITS, MAX_CELLS, MAX_CELL_ID,
    MAX_SOURCE_BYTES, MAX_TEXT_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use thiserror::Error;

/// The value of the `format` key.
pub const FORMAT_NAME: &str = "abaco";
/// The version this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// Largest file this crate will read, in bytes.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Why a file could not be read or written.
#[derive(Debug, Error)]
pub enum FormatError {
    /// The file is not valid JSON of the expected shape.
    #[error("the file is not a valid Ábaco document (line {line}, column {column}): {message}")]
    Malformed {
        /// 1-based line of the problem.
        line: usize,
        /// 1-based column of the problem.
        column: usize,
        /// What is wrong.
        message: String,
    },
    /// Valid JSON, but not an Ábaco document.
    #[error("this is not an Ábaco document: {0}")]
    NotAbaco(String),
    /// Written by a newer version of the format.
    #[error("this document uses format version {found}, but this program reads up to version {supported}; update Ábaco to open it")]
    NewerVersion {
        /// The version in the file.
        found: u64,
        /// The newest version this build reads.
        supported: u32,
    },
    /// A size limit was exceeded.
    #[error("{0}")]
    TooLarge(String),
    /// A value is out of range or inconsistent.
    #[error("{0}")]
    Invalid(String),
    /// Reading or writing the file failed.
    #[error("{action} `{path}`: {source}")]
    Io {
        /// What was being done.
        action: &'static str,
        /// The path.
        path: String,
        /// The operating-system error.
        #[source]
        source: std::io::Error,
    },
}

/// Versions of the engines, recorded in saved files.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EngineInfo {
    /// This crate.
    #[serde(rename = "abaco-doc")]
    pub abaco_doc: String,
    /// The expression language.
    #[serde(rename = "abaco-expr")]
    pub abaco_expr: String,
    /// The units engine.
    #[serde(rename = "nodalia-units")]
    pub nodalia_units: String,
}

impl EngineInfo {
    /// The versions of this build.
    pub fn current() -> EngineInfo {
        EngineInfo {
            abaco_doc: env!("CARGO_PKG_VERSION").to_string(),
            abaco_expr: abaco_expr::VERSION.to_string(),
            nodalia_units: nodalia_units::VERSION.to_string(),
        }
    }
}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    format_version: u32,
    settings: FileSettings,
    next_cell_id: u64,
    cells: Vec<FileCellOut<'a>>,
    engine: EngineInfo,
}

#[derive(Serialize, Deserialize)]
struct FileSettings {
    #[serde(default = "default_digits")]
    significant_digits: usize,
    #[serde(default)]
    strict_order: bool,
}

fn default_digits() -> usize {
    DEFAULT_SIGNIFICANT_DIGITS
}

impl Default for FileSettings {
    fn default() -> FileSettings {
        FileSettings {
            significant_digits: DEFAULT_SIGNIFICANT_DIGITS,
            strict_order: false,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum FileCellOut<'a> {
    Math { id: u64, source: &'a str },
    Text { id: u64, markdown: &'a str },
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    format_version: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    settings: FileSettings,
    #[serde(default)]
    next_cell_id: Option<u64>,
    cells: Vec<FileCellIn>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum FileCellIn {
    Math { id: u64, source: String },
    Text { id: u64, markdown: String },
}

fn malformed(e: &serde_json::Error) -> FormatError {
    FormatError::Malformed {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    }
}

impl Document {
    /// The file contents for this document.
    pub fn to_json(&self) -> Result<String, FormatError> {
        let cells = self
            .cells()
            .iter()
            .map(|c| match &c.kind {
                CellKind::Math { source } => FileCellOut::Math { id: c.id.0, source },
                CellKind::Text { markdown } => FileCellOut::Text {
                    id: c.id.0,
                    markdown,
                },
            })
            .collect();
        let out = FileOut {
            format: FORMAT_NAME,
            format_version: FORMAT_VERSION,
            settings: FileSettings {
                significant_digits: self.settings().significant_digits,
                strict_order: self.settings().strict_order,
            },
            next_cell_id: self.next_id(),
            cells,
            engine: EngineInfo::current(),
        };
        let mut text = serde_json::to_string_pretty(&out)
            .map_err(|e| FormatError::Invalid(format!("the document could not be written: {e}")))?;
        text.push('\n');
        Ok(text)
    }

    /// Reads a document from file contents. The returned document has not been evaluated.
    pub fn from_json(text: &str) -> Result<Document, FormatError> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err(FormatError::TooLarge(format!(
                "the file is {} bytes; the limit is {MAX_FILE_BYTES}",
                text.len()
            )));
        }
        // First pass: only the signature and version, so a newer file is recognised before its
        // new content can confuse the reader.
        let header: Header = serde_json::from_str(text).map_err(|e| malformed(&e))?;
        match header.format.as_deref() {
            Some(FORMAT_NAME) => {}
            Some(other) => {
                return Err(FormatError::NotAbaco(format!(
                    "its `format` is `{other}`, expected `{FORMAT_NAME}`"
                )))
            }
            None => {
                return Err(FormatError::NotAbaco(
                    "there is no `format` key at the top level".into(),
                ))
            }
        }
        let version = match header.format_version {
            Some(serde_json::Value::Number(n)) => n.as_u64().ok_or_else(|| {
                FormatError::Invalid(format!(
                    "`format_version` must be a whole number of at least 1, got {n}"
                ))
            })?,
            Some(_) => {
                return Err(FormatError::Invalid(
                    "`format_version` must be a number".into(),
                ))
            }
            None => {
                return Err(FormatError::NotAbaco(
                    "there is no `format_version` key".into(),
                ))
            }
        };
        if version == 0 {
            return Err(FormatError::Invalid(
                "`format_version` must be at least 1".into(),
            ));
        }
        if version > u64::from(FORMAT_VERSION) {
            return Err(FormatError::NewerVersion {
                found: version,
                supported: FORMAT_VERSION,
            });
        }

        let file: FileIn = serde_json::from_str(text).map_err(|e| malformed(&e))?;
        if file.cells.len() > MAX_CELLS {
            return Err(FormatError::TooLarge(format!(
                "the document has {} cells; the limit is {MAX_CELLS}",
                file.cells.len()
            )));
        }
        let settings = Settings {
            significant_digits: file.settings.significant_digits,
            strict_order: file.settings.strict_order,
        };
        settings
            .validate()
            .map_err(|e| FormatError::Invalid(e.to_string()))?;

        let mut seen: HashSet<u64> = HashSet::new();
        let mut cells = Vec::with_capacity(file.cells.len());
        let mut max_id = 0u64;
        for (i, c) in file.cells.into_iter().enumerate() {
            let (id, kind) = match c {
                FileCellIn::Math { id, source } => {
                    if source.len() > MAX_SOURCE_BYTES {
                        return Err(FormatError::TooLarge(format!(
                            "cell {} holds {} bytes of source; the limit is {MAX_SOURCE_BYTES}",
                            i + 1,
                            source.len()
                        )));
                    }
                    (id, CellKind::Math { source })
                }
                FileCellIn::Text { id, markdown } => {
                    if markdown.len() > MAX_TEXT_BYTES {
                        return Err(FormatError::TooLarge(format!(
                            "cell {} holds {} bytes of text; the limit is {MAX_TEXT_BYTES}",
                            i + 1,
                            markdown.len()
                        )));
                    }
                    (id, CellKind::Text { markdown })
                }
            };
            if id == 0 || id > MAX_CELL_ID {
                return Err(FormatError::Invalid(format!(
                    "cell {} has the id {id}; ids run from 1 to {MAX_CELL_ID}",
                    i + 1
                )));
            }
            if !seen.insert(id) {
                return Err(FormatError::Invalid(format!(
                    "the id {id} is used by more than one cell"
                )));
            }
            max_id = max_id.max(id);
            cells.push(Cell {
                id: CellId(id),
                kind,
            });
        }
        // Ids are never reused: the counter must stay above every id in the file.
        let next = file.next_cell_id.unwrap_or(0).max(max_id + 1);
        if next > MAX_CELL_ID {
            return Err(FormatError::Invalid(format!(
                "`next_cell_id` {next} is beyond the largest id {MAX_CELL_ID}"
            )));
        }
        Ok(Document::from_parts(cells, next, settings))
    }

    /// Opens a file. The size is checked before the contents are read.
    pub fn open(path: impl AsRef<Path>) -> Result<Document, FormatError> {
        let path = path.as_ref();
        let io = |action: &'static str, source: std::io::Error| FormatError::Io {
            action,
            path: path.display().to_string(),
            source,
        };
        let meta = std::fs::metadata(path).map_err(|e| io("cannot open", e))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(FormatError::TooLarge(format!(
                "`{}` is {} bytes; the limit is {MAX_FILE_BYTES}",
                path.display(),
                meta.len()
            )));
        }
        let bytes = std::fs::read(path).map_err(|e| io("cannot read", e))?;
        let text = String::from_utf8(bytes).map_err(|e| {
            FormatError::Invalid(format!(
                "`{}` is not text in UTF-8 (invalid byte at {})",
                path.display(),
                e.utf8_error().valid_up_to()
            ))
        })?;
        Document::from_json(&text)
    }

    /// Saves to `path` atomically: the file is written beside it and then renamed over it, so
    /// a crash or a full disk never leaves half a document behind.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), FormatError> {
        let path = path.as_ref();
        let io = |action: &'static str, source: std::io::Error| FormatError::Io {
            action,
            path: path.display().to_string(),
            source,
        };
        let text = self.to_json()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_string());
        let tmp = path.with_file_name(format!(".{name}.tmp"));
        if let Err(e) = std::fs::write(&tmp, text.as_bytes()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io("cannot write", e));
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io("cannot replace", e));
        }
        Ok(())
    }
}
