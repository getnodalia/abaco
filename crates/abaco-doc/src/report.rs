// SPDX-License-Identifier: MIT OR Apache-2.0
//! A serialisable account of a document's results, shared by every front end.

use crate::document::Document;
use crate::model::{CellError, CellKind};
use abaco_expr::line_col;
use serde_json::{json, Value as Json};

impl Document {
    /// One entry per cell, in page order, describing its kind, source and result.
    ///
    /// A math cell has `"status": "ok"` with `value` (a description with a `text` rendering at
    /// the document's significant digits) or `"status": "error"` with `error` carrying `code`,
    /// `message` and, when the problem has a position in the cell's source, 1-based `line` and
    /// `column`. Text cells have `"status": "text"`.
    pub fn report(&self) -> Json {
        let digits = self.settings().significant_digits;
        let cells: Vec<Json> = self
            .cells()
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let index = i + 1;
                match &cell.kind {
                    CellKind::Text { markdown } => json!({
                        "index": index,
                        "id": cell.id.0,
                        "type": "text",
                        "markdown": markdown,
                        "status": "text",
                    }),
                    CellKind::Math { source } => {
                        let mut entry = json!({
                            "index": index,
                            "id": cell.id.0,
                            "type": "math",
                            "source": source,
                        });
                        match self.result(cell.id) {
                            None => {
                                entry["status"] = json!("pending");
                            }
                            Some(r) => {
                                match &r.outcome {
                                    Ok(out) => {
                                        entry["status"] = json!("ok");
                                        entry["value"] = out
                                            .value
                                            .as_ref()
                                            .map_or(Json::Null, |v| v.to_json(digits));
                                        entry["defined"] = json!(out.defined);
                                    }
                                    Err(e) => {
                                        entry["status"] = json!("error");
                                        entry["error"] = error_json(e, source);
                                    }
                                }
                                if !r.warnings.is_empty() {
                                    entry["warnings"] = json!(r.warnings);
                                }
                            }
                        }
                        entry
                    }
                }
            })
            .collect();
        json!({
            "settings": {
                "significant_digits": digits,
                "strict_order": self.settings().strict_order,
            },
            "errors": self.error_count(),
            "cells": cells,
        })
    }
}

/// The JSON description of a cell error against the cell's source.
pub fn error_json(e: &CellError, source: &str) -> Json {
    let mut v = json!({"code": e.code(), "message": e.message()});
    if let Some(span) = e.span() {
        let (line, column) = line_col(source, span.start);
        v["line"] = json!(line);
        v["column"] = json!(column);
        v["start"] = json!(span.start);
        v["end"] = json!(span.end);
    }
    match e {
        CellError::Cycle { path, .. } => {
            v["cells"] = json!(path.iter().map(|c| c.0).collect::<Vec<_>>());
        }
        CellError::Dependency { cell, name, .. } => {
            v["cell"] = json!(cell.0);
            v["name"] = json!(name);
        }
        CellError::Duplicate { first, name, .. } => {
            v["cell"] = json!(first.0);
            v["name"] = json!(name);
        }
        CellError::Expr(_) => {}
    }
    v
}
