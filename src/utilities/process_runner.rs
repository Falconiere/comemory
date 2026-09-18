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

use crate::utilities::process_pipes::{Drained, Pipes, ReadDone, Streams};

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

/// A failed bounded run, with whatever diagnostic stderr had been drained
/// before it failed.
///
/// The stderr travels with the failure because that is exactly when it matters:
/// a child that prints `loading weights...` and then exceeds its budget leaves
/// no other clue, and the failure alone would say only "timed out".
#[derive(Debug, Error)]
#[error("{failure}")]
pub struct ProcessError {
    /// Why the run failed.
    pub failure: ProcessFailure,
    /// The retained stderr excerpt, empty when nothing was drained.
    pub stderr: Vec<u8>,
}

/// Result alias for a bounded run.
pub type ProcessResult<T> = std::result::Result<T, ProcessError>;

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

    /// Run the child once, writing `input` to its stdin.
    ///
    /// The deadline is taken before the spawn, so startup is inside the budget
    /// as much as the I/O and the exit are.
    pub fn run(&self, input: &[u8]) -> ProcessResult<ProcessOutput> {
        if input.len() > self.limits.max_input_bytes {
            return Err(bare(ProcessFailure::InputTooLarge {
                bytes: input.len(),
                max: self.limits.max_input_bytes,
            }));
        }
        // The budget is carried as a `Duration` and compared against
        // `started.elapsed()`, never added to an `Instant`. `Instant + Duration`
        // panics on overflow and the budget is a public input, so a caller
        // passing `Duration::MAX` would abort the process; there is no addition
        // left to overflow, and no fallback that could itself overflow.
        let started = Instant::now();
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
                return Err(bare(io_failed("pipe setup", message)));
            }
        };
        let drained = pipes.drain(&mut child, started, self.timeout);
        terminate(&mut child);
        let streams = pipes.retained();
        finish(
            drained,
            streams,
            started.elapsed(),
            self.timeout,
            self.limits,
        )
    }

    /// Spawn the configured program with all three streams piped.
    fn spawn(&self) -> ProcessResult<Child> {
        Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| bare(ProcessFailure::Spawn(e.to_string())))
    }
}

/// A failure that happened before any stderr could have been drained.
fn bare(failure: ProcessFailure) -> ProcessError {
    ProcessError {
        failure,
        stderr: Vec::new(),
    }
}

/// Kill the child unless it already exited, then reap it.
///
/// `kill` only signals; without the `wait` a killed child lingers as a zombie
/// until this process exits. `wait` on a signalled direct child returns as
/// soon as that child is reaped, whatever its descendants are doing.
fn terminate(child: &mut Child) {
    // Kill unless the child is positively known to have exited. A `try_wait`
    // that fails tells us nothing, and skipping the kill on it would leave
    // `wait` below to block on a child still running.
    if !matches!(child.try_wait(), Ok(Some(_))) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// Turn one drained run into an output or a typed failure.
///
/// The stderr excerpt travels with either verdict: on success it is the
/// output's, on failure it is the failure's. The stdout bytes are only handed
/// over once the stream is known to have reached EOF — a partial payload is
/// never a payload.
fn finish(
    drained: Drained,
    streams: Streams,
    elapsed: Duration,
    budget: Duration,
    limits: ProcessLimits,
) -> ProcessResult<ProcessOutput> {
    match completed(drained, budget, limits) {
        Ok(status) => Ok(ProcessOutput {
            status: status.0,
            stdout: streams.stdout,
            stderr: streams.stderr,
            input_truncated: status.1,
            elapsed,
        }),
        Err(failure) => Err(ProcessError {
            failure,
            stderr: streams.stderr,
        }),
    }
}

/// Account for stdout, stdin and the exit, or say what stopped the run.
///
/// The three arms are spelled out here rather than in a helper each, because a
/// helper per stream is the same three-arm match twice over.
fn completed(
    drained: Drained,
    budget: Duration,
    limits: ProcessLimits,
) -> Result<(ExitStatus, bool), ProcessFailure> {
    refuse_early(&drained, budget, limits)?;
    // stdout must have reached EOF for its retained bytes to be the whole
    // payload; a partial payload is never a payload.
    match drained.stdout {
        Some(ReadDone::Eof) => {}
        Some(ReadDone::Failed(message)) => return Err(io_failed("stdout read", message)),
        _ => return Err(ProcessFailure::TimedOut { budget }),
    }
    let input_truncated = match drained.write {
        Some(Ok(truncated)) => truncated,
        Some(Err(message)) => return Err(io_failed("stdin write", message)),
        None => return Err(ProcessFailure::TimedOut { budget }),
    };
    let status = drained.status.ok_or(ProcessFailure::TimedOut { budget })?;
    Ok((status, input_truncated))
}

/// The one construction of a phase-tagged pipe or wait failure.
fn io_failed(phase: &'static str, message: String) -> ProcessFailure {
    ProcessFailure::Io { phase, message }
}

/// Reject the three conditions that make a complete output impossible.
fn refuse_early(
    drained: &Drained,
    budget: Duration,
    limits: ProcessLimits,
) -> Result<(), ProcessFailure> {
    if let Some(message) = &drained.wait_error {
        return Err(io_failed("wait", message.clone()));
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

#[cfg(test)]
#[path = "tests/process_runner.rs"]
mod tests;
