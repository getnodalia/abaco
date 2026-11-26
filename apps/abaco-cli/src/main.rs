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
//! The headless `abaco` command.
//!
//! ```text
//! abaco eval '<expression>' [--json] [--digits N]
//! abaco run <file.abaco> [--json] [--digits N]
//! abaco units list [--json]
//! abaco units convert '<quantity>' '<unit>' [--json] [--digits N]
//! ```
//!
//! Exit codes: 0 success; 1 the expression, a cell or a conversion failed; 2 the command line
//! or the file could not be used.

mod args;
mod commands;

use std::io::Write;
use std::process::ExitCode;

/// Writes to standard output; a closed pipe (`abaco ... | head`) ends quietly.
pub(crate) fn out(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.write_all(b"\n");
}

/// Writes to standard error.
pub(crate) fn err(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
    let _ = stderr.write_all(b"\n");
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match args::parse(&argv) {
        Ok(args::Parsed::Help(topic)) => {
            out(&args::help(topic));
            ExitCode::SUCCESS
        }
        Ok(args::Parsed::Version) => {
            out(&format!("abaco {}", env!("CARGO_PKG_VERSION")));
            ExitCode::SUCCESS
        }
        Ok(args::Parsed::Command(cmd)) => commands::run(&cmd),
        Err(message) => {
            err(&format!("error: {message}"));
            err("Run `abaco --help` for usage.");
            ExitCode::from(2)
        }
    }
}
