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
//! Everything Ábaco's window needs that is not drawing.
//!
//! The window (`main.rs` and its modules) only draws and collects what the person did. The
//! rest lives here so it can be tested without a display:
//!
//! * [`engine`]: the maths document on a worker thread, driven by [`engine::Request`]s and
//!   answering with [`engine::Reply`]s that carry a [`engine::Snapshot`] of every cell's result;
//! * [`workbook`]: the view-model the window reads, and its reducer;
//! * [`catalog`]: the units and constants the side panel lists, and the conversion it offers;
//! * [`script`]: Ábaco Script on a worker thread, and what the script workspace shows;
//! * [`blocks`]: block diagrams: the diagram being edited and Ábaco Blocks on a worker thread;
//! * [`sample`]: the document a first launch opens;
//! * [`settings`]: the preferences file;
//! * [`i18n`]: Spanish and English.

pub mod blocks;
pub mod catalog;
pub mod commands;
pub mod engine;
pub mod highlight;
pub mod i18n;
pub mod icon;
pub mod sample;
pub mod script;
pub mod settings;
mod spanish;
pub mod workbook;

/// The name in the interface.
pub const NAME: &str = "Ábaco";
/// The application id (desktop file, D-Bus names, window class).
pub const APP_ID: &str = "dev.danielmiguelt.Abaco";
/// The file extension of a maths document.
pub const EXTENSION: &str = "abaco";
