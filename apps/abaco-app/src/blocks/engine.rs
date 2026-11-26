// SPDX-License-Identifier: MIT OR Apache-2.0
//! Block diagrams run on a worker thread. The window sends a [`Job`] and folds in the [`Reply`];
//! a run is stopped by setting the cancel flag the solver checks at every step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use abaco_blocks::{compile, BlockError, Model, SimOptions, SimResult};

/// What the window asks for.
#[derive(Debug)]
pub enum Job {
    /// Compile and run a diagram.
    Run { id: u64, model: Box<Model> },
}

/// What the worker answers.
#[derive(Debug)]
pub enum Reply {
    /// A run ended.
    Finished(Box<Finished>),
}

#[derive(Debug)]
pub struct Finished {
    pub id: u64,
    /// The recorded signals, or what went wrong (with the block named).
    pub outcome: Result<SimResult, String>,
    pub millis: u64,
    /// The person stopped the run.
    pub stopped: bool,
}

/// The worker thread has ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerGone;

/// The worker thread and the channels to it.
pub struct BlocksEngine {
    tx: Sender<Job>,
    rx: Receiver<Reply>,
    cancel: Arc<AtomicBool>,
}

impl BlocksEngine {
    /// Starts the worker. `wake` is called after every reply so the window repaints.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> std::io::Result<BlocksEngine> {
        let (tx, jobs) = channel::<Job>();
        let (replies, rx) = channel::<Reply>();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("abaco-blocks".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let reply = handle(job, &flag);
                    if replies.send(reply).is_err() {
                        return;
                    }
                    wake();
                }
            })?;
        Ok(BlocksEngine { tx, rx, cancel })
    }

    /// Sends a job. An error means the worker is gone.
    pub fn send(&mut self, job: Job) -> Result<(), WorkerGone> {
        self.cancel.store(false, Ordering::Relaxed);
        self.tx.send(job).map_err(|_| WorkerGone)
    }

    /// Asks the run in progress to stop at its next step.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Takes the replies that have arrived.
    pub fn poll(&mut self) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
            out.push(r);
        }
        out
    }
}

fn handle(job: Job, cancel: &Arc<AtomicBool>) -> Reply {
    match job {
        Job::Run { id, model } => {
            let start = Instant::now();
            let opts = SimOptions {
                cancel: Some(cancel.clone()),
                ..SimOptions::default()
            };
            // The engine does not panic on input; this keeps the worker alive if it ever does.
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compile(&model).and_then(|c| c.simulate(&opts))
            }));
            let (outcome, stopped) = match ran {
                Ok(Ok(r)) => (Ok(r), false),
                Ok(Err(BlockError::Cancelled)) => (Err(BlockError::Cancelled.to_string()), true),
                Ok(Err(e)) => (Err(e.to_string()), false),
                Err(_) => (
                    Err("the simulation failed unexpectedly; the diagram was not changed".into()),
                    false,
                ),
            };
            Reply::Finished(Box::new(Finished {
                id,
                outcome,
                millis: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                stopped,
            }))
        }
    }
}
