//! Run one child process under a single end-to-end deadline.
//!
//! The budget starts before the spawn and covers startup, concurrent
//! stdin/stdout/stderr handling and exit. Input, stdout and stderr are each
//! byte-bounded. The child is always killed (unless it exited) and always
//! reaped, on every path.
//!
//! The program and its arguments are passed to `Command` separately and are
//! never assembled into a shell command line, so payload text can only reach
//! the child as bytes on stdin.

use std::ffi::OsString;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::utilities::process_pipes::{Drained, Pipes, ReadDone, WriteDone};

/// Default end-to-end budget for one bounded run.
pub const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(10);

/// Byte bounds one run is held to.
#[derive(Debug, Clone, Copy)]
pub struct ProcessLimits {
    /// Largest payload that may be written to the child's stdin.
    pub max_input_bytes: usize,
    /// Largest stdout payload that may be retained; more is a failure.
    pub max_stdout_bytes: usize,
    /// Largest stderr excerpt retained; more is drained and discarded.
    pub max_stderr_bytes: usize,
}

impl Default for ProcessLimits {
    /// 8 MiB in, 8 MiB out — the ceiling `utilities::vector_stdin` already
    /// applies to a caller-supplied payload — and a 16 KiB stderr excerpt.
    fn default() -> Self {
        Self {
            max_input_bytes: 8 << 20,
            max_stdout_bytes: 8 << 20,
            max_stderr_bytes: 16 << 10,
        }
    }
}

/// What a completed run produced. A non-zero `status` is still a completed
/// run: how to read it is the caller's policy.
#[derive(Debug)]
pub struct ProcessOutput {
    /// The child's exit status.
    pub status: ExitStatus,
    /// Everything the child wrote to stdout, up to the cap.
    pub stdout: Vec<u8>,
    /// The retained stderr excerpt, up to the cap.
    pub stderr: Vec<u8>,
    /// The child closed its stdin read end before the whole payload landed.
    pub input_truncated: bool,
    /// Wall-clock time from just before the spawn to the reap.
    pub elapsed: Duration,
}

/// Why a bounded run did not produce a complete output.
#[derive(Debug, Error)]
pub enum ProcessFailure {
    /// The payload is larger than `max_input_bytes`; nothing was spawned.
    #[error("input of {bytes} bytes exceeds the {max}-byte limit")]
    InputTooLarge {
        /// Size of the refused payload.
        bytes: usize,
        /// The configured ceiling.
        max: usize,
    },
    /// The process could not be started.
    #[error("spawn failed: {0}")]
    Spawn(String),
    /// A pipe or wait operation failed.
    #[error("{phase} failed: {message}")]
    Io {
        /// Which step failed.
        phase: &'static str,
        /// The underlying operating-system message.
        message: String,
    },
    /// The child wrote more than `max_stdout_bytes`.
    #[error("output exceeds the {max}-byte limit")]
    StdoutTooLarge {
        /// The configured ceiling.
        max: usize,
    },
    /// The deadline passed before the run was fully accounted for.
    #[error("timed out after {budget:?}")]
    TimedOut {
        /// The budget that was exceeded.
        budget: Duration,
    },
}

/// Result alias for a bounded run.
pub type ProcessResult<T> = std::result::Result<T, ProcessFailure>;

/// A child process to run once, under one end-to-end deadline.
///
/// Immutable after construction and `Send + Sync`: `run` borrows `&self` and
/// owns every piece of per-run state, so one value may drive concurrent runs.
#[derive(Debug, Clone)]
pub struct ProcessRunner {
    program: OsString,
    args: Vec<OsString>,
    timeout: Duration,
    limits: ProcessLimits,
}

impl ProcessRunner {
    /// Select the executable and its arguments. Both come from the caller's
    /// trusted configuration; neither is ever parsed out of payload data.
    pub fn new(program: impl Into<OsString>, args: Vec<OsString>) -> Self {
        Self {
            program: program.into(),
            args,
            timeout: DEFAULT_PROCESS_TIMEOUT,
            limits: ProcessLimits::default(),
        }
    }

    /// Replace the end-to-end budget.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Replace the byte bounds.
    #[must_use]
    pub fn with_limits(mut self, limits: ProcessLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The byte bounds this runner enforces.
    pub fn limits(&self) -> ProcessLimits {
        self.limits
    }

    /// Run the child once, writing `input` to its stdin.
    ///
    /// The deadline is taken before the spawn, so startup is inside the budget
    /// as much as the I/O and the exit are.
    pub fn run(&self, input: &[u8]) -> ProcessResult<ProcessOutput> {
        if input.len() > self.limits.max_input_bytes {
            return Err(ProcessFailure::InputTooLarge {
                bytes: input.len(),
                max: self.limits.max_input_bytes,
            });
        }
        let started = Instant::now();
        let deadline = started + self.timeout;
        let mut child = self.spawn()?;
        let pipes = Pipes::start(
            &mut child,
            input.to_vec(),
            self.limits.max_stdout_bytes,
            self.limits.max_stderr_bytes,
        );
        let pipes = match pipes {
            Ok(pipes) => pipes,
            Err(message) => {
                terminate(&mut child);
                return Err(ProcessFailure::Io {
                    phase: "pipe setup",
                    message,
                });
            }
        };
        let drained = pipes.drain(&mut child, deadline);
        terminate(&mut child);
        finish(drained, started.elapsed(), self.timeout, self.limits)
    }

    /// Spawn the configured program with all three streams piped.
    fn spawn(&self) -> ProcessResult<Child> {
        Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ProcessFailure::Spawn(e.to_string()))
    }
}

/// Kill the child unless it already exited, then reap it.
///
/// `kill` only signals; without the `wait` a killed child lingers as a zombie
/// until this process exits. `wait` on a signalled direct child returns as
/// soon as that child is reaped, whatever its descendants are doing.
fn terminate(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Turn one drained run into an output or a typed failure.
fn finish(
    drained: Drained,
    elapsed: Duration,
    budget: Duration,
    limits: ProcessLimits,
) -> ProcessResult<ProcessOutput> {
    refuse_early(&drained, budget, limits)?;
    let stdout = stdout_bytes(drained.stdout, budget)?;
    let input_truncated = write_truncated(drained.write, budget)?;
    let status = drained.status.ok_or(ProcessFailure::TimedOut { budget })?;
    Ok(ProcessOutput {
        status,
        stdout,
        // A failed or missing stderr read is never fatal: it is diagnostic.
        stderr: match drained.stderr {
            Some(ReadDone::Eof(bytes)) => bytes,
            _ => Vec::new(),
        },
        input_truncated,
        elapsed,
    })
}

/// Reject the three conditions that make a complete output impossible.
fn refuse_early(drained: &Drained, budget: Duration, limits: ProcessLimits) -> ProcessResult<()> {
    if let Some(message) = &drained.wait_error {
        return Err(ProcessFailure::Io {
            phase: "wait",
            message: message.clone(),
        });
    }
    if matches!(drained.stdout, Some(ReadDone::Overflow)) {
        return Err(ProcessFailure::StdoutTooLarge {
            max: limits.max_stdout_bytes,
        });
    }
    if drained.timed_out {
        return Err(ProcessFailure::TimedOut { budget });
    }
    Ok(())
}

/// The complete, EOF-terminated stdout payload, or why it never arrived.
fn stdout_bytes(done: Option<ReadDone>, budget: Duration) -> ProcessResult<Vec<u8>> {
    match done {
        Some(ReadDone::Eof(bytes)) => Ok(bytes),
        Some(ReadDone::Failed(message)) => Err(ProcessFailure::Io {
            phase: "stdout read",
            message,
        }),
        _ => Err(ProcessFailure::TimedOut { budget }),
    }
}

/// Whether the child closed stdin early, or why the write never finished.
fn write_truncated(done: Option<WriteDone>, budget: Duration) -> ProcessResult<bool> {
    match done {
        Some(Ok(truncated)) => Ok(truncated),
        Some(Err(message)) => Err(ProcessFailure::Io {
            phase: "stdin write",
            message,
        }),
        None => Err(ProcessFailure::TimedOut { budget }),
    }
}

#[cfg(test)]
#[path = "tests/process_runner.rs"]
mod tests;
