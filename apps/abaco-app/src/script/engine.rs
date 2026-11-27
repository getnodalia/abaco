// SPDX-License-Identifier: MIT OR Apache-2.0
//! Ábaco Script on a worker thread. The window sends [`Job`]s and folds in [`Reply`]s; nothing
//! here is drawn, and nothing the interpreter does can block the window.
//!
//! One interpreter lives for the whole session, so variables and functions defined by one run
//! are there for the next. A run is stopped by setting the interpreter's cancel flag from the
//! window's thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use abaco_script::display;
use abaco_script::plot::Figures;
use abaco_script::{Event, Interp, Limits};

/// Native stack for the interpreter thread. The interpreter recurses for nested expressions and
/// calls and bounds itself by [`STACK_BUDGET`].
const STACK: usize = 128 * 1024 * 1024;
/// About three quarters of [`STACK`], as the interpreter's limits ask.
const STACK_BUDGET: usize = 96 * 1024 * 1024;
/// Longest text kept when a variable is inspected.
const MAX_INSPECT: usize = 20_000;

/// What the window asks for.
#[derive(Debug)]
pub enum Job {
    /// Run source text in the base workspace.
    Run { id: u64, source: String },
    /// Forget every variable.
    ClearVariables,
    /// Forget one variable.
    Remove(String),
    /// Show what one variable holds.
    Inspect(String),
    /// Close every figure.
    CloseFigures,
}

/// One variable of the workspace, as the browser lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct VarRow {
    pub name: String,
    /// `double`, `char`, `logical`, `cell`, `struct`, `function_handle`…
    pub class: String,
    /// `3x3`.
    pub size: String,
    /// One line: the value for a scalar, `1x3 double` for the rest.
    pub summary: String,
}

/// Where a run failed.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// What went wrong, with the function trace.
    pub message: String,
    /// The 1-based line of the source, when the error knows it.
    pub line: Option<usize>,
}

/// What the interpreter answers.
#[derive(Debug)]
pub enum Reply {
    /// A run ended.
    Finished(Box<Finished>),
    /// The workspace changed without a run (cleared, one variable removed).
    Workspace(Vec<VarRow>),
    /// The text of one variable.
    Inspected { name: String, text: String },
    /// The figures changed without a run.
    Figures(Figures),
}

#[derive(Debug)]
pub struct Finished {
    pub id: u64,
    pub failure: Option<Failure>,
    /// Printed text, warnings and `clc`, in order.
    pub events: Vec<Event>,
    pub vars: Vec<VarRow>,
    /// The figures after the run, when they exist or existed.
    pub figures: Option<Figures>,
    pub millis: u64,
    /// The run was stopped by the person.
    pub stopped: bool,
}

/// The interpreter's thread has ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerGone;

/// The worker thread and the channels to it.
pub struct ScriptEngine {
    tx: Sender<Job>,
    rx: Receiver<Reply>,
    cancel: Arc<AtomicBool>,
    in_flight: usize,
}

impl ScriptEngine {
    /// Starts the worker. `wake` is called after every reply so the window repaints.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> std::io::Result<ScriptEngine> {
        let (tx, jobs) = channel::<Job>();
        let (replies, rx) = channel::<Reply>();
        let mut interp = Interp::with_limits(Limits {
            max_stack: STACK_BUDGET,
            files: true,
            ..Limits::default()
        });
        let cancel = interp.cancel.clone();
        std::thread::Builder::new()
            .name("abaco-script".into())
            .stack_size(STACK)
            .spawn(move || {
                let mut had_figures = false;
                while let Ok(job) = jobs.recv() {
                    let reply = handle(&mut interp, job, &mut had_figures);
                    if replies.send(reply).is_err() {
                        return;
                    }
                    wake();
                }
            })?;
        Ok(ScriptEngine {
            tx,
            rx,
            cancel,
            in_flight: 0,
        })
    }

    /// Sends a job. An error means the worker is gone.
    pub fn send(&mut self, job: Job) -> Result<(), WorkerGone> {
        if matches!(job, Job::Run { .. }) {
            self.cancel.store(false, Ordering::Relaxed);
        }
        self.tx.send(job).map_err(|_| WorkerGone)?;
        self.in_flight += 1;
        Ok(())
    }

    /// Asks the run in progress to stop at its next step.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Takes the replies that have arrived.
    pub fn poll(&mut self) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Ok(reply) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            out.push(reply);
        }
        out
    }

    /// Jobs sent and not yet answered.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }
}

fn rows(interp: &Interp) -> Vec<VarRow> {
    let long = interp.format_long;
    interp
        .variables()
        .into_iter()
        .map(|(name, value)| {
            let (r, c) = value.dims();
            VarRow {
                name,
                class: value.class().to_owned(),
                size: format!("{r}x{c}"),
                summary: display::summary(&value, long),
            }
        })
        .collect()
}

fn handle(interp: &mut Interp, job: Job, had_figures: &mut bool) -> Reply {
    match job {
        Job::Run { id, source } => {
            let start = Instant::now();
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| interp.run(&source)));
            let events = interp.take_events();
            let (failure, stopped) = match outcome {
                Ok(Ok(())) => (None, false),
                Ok(Err(e)) => {
                    let stopped = interp.cancel.load(Ordering::Relaxed);
                    let line = e.span.map(|s| s.line_col(&source).0);
                    (
                        Some(Failure {
                            message: e.render(&source),
                            line,
                        }),
                        stopped,
                    )
                }
                Err(_) => (
                    Some(Failure {
                        message: "Something went wrong inside the interpreter.".to_owned(),
                        line: None,
                    }),
                    false,
                ),
            };
            let has = !interp.figures.list.is_empty();
            let figures = (has || *had_figures).then(|| interp.figures.clone());
            *had_figures = has;
            Reply::Finished(Box::new(Finished {
                id,
                failure,
                events,
                vars: rows(interp),
                figures,
                millis: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                stopped,
            }))
        }
        Job::ClearVariables => {
            interp.clear_all();
            Reply::Workspace(rows(interp))
        }
        Job::Remove(name) => {
            interp.remove(&name);
            Reply::Workspace(rows(interp))
        }
        Job::Inspect(name) => {
            let text = match interp.get(&name) {
                Some(v) => {
                    let mut text = display::show(&name, v, interp.format_long);
                    if text.len() > MAX_INSPECT {
                        let mut cut = MAX_INSPECT;
                        while !text.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        text.truncate(cut);
                        text.push_str("\n…");
                    }
                    text
                }
                None => String::new(),
            };
            Reply::Inspected { name, text }
        }
        Job::CloseFigures => {
            interp.figures.close(None);
            *had_figures = false;
            Reply::Figures(interp.figures.clone())
        }
    }
}
