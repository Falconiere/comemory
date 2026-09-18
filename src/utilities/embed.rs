//! Shared embed-command shell-out (Memory-tab semantic enrich + `serve`).
//!
//! Runs the user-configured command as `sh -c <cmd>` through
//! [`ProcessRunner`], pipes the query to its stdin, and parses a JSON
//! `{"embedding":[..]}` payload from its stdout. The shell is the *program*
//! here — a pre-existing, user-configured contract — while the runner itself
//! never assembles a command line.
//!
//! [`EMBED_TIMEOUT`] is an end-to-end budget covering startup, concurrent
//! stdin/stdout handling and exit (#211); it previously bounded only the
//! stdout read. Every failure path returns an `Error`.

use std::ffi::OsString;
use std::time::Duration;

use crate::prelude::*;
use crate::utilities::embedding_input;
use crate::utilities::process_runner::{ProcessFailure, ProcessRunner};

/// Maximum time to wait for the embed command to produce its vector.
pub const EMBED_TIMEOUT: Duration = Duration::from_secs(10);

/// Vectorize `query` via `cmd` (`sh -c <cmd>`), bounded by [`EMBED_TIMEOUT`].
/// Returns the parsed embedding, or an `Error` describing the failed phase.
pub fn embed_query(cmd: &str, query: &str) -> Result<Vec<f32>> {
    embed_query_with_timeout(cmd, query, EMBED_TIMEOUT)
}

/// [`embed_query`] with an explicit `timeout`. Exposed so tests can drive the
/// timeout path with a tiny bound instead of waiting [`EMBED_TIMEOUT`].
///
/// A command may close its end of the stdin pipe, or exit, before the whole
/// query has been written — whether it never reads stdin at all (a `printf` of
/// a canned payload, a script that embeds from an argument) or stops after
/// consuming part of it. That is the command's choice, not a failure: its exit
/// status and stdout still decide the outcome, so the runner's truncation flag
/// is deliberately ignored here.
pub fn embed_query_with_timeout(cmd: &str, query: &str, timeout: Duration) -> Result<Vec<f32>> {
    let runner = ProcessRunner::new("sh", vec![OsString::from("-c"), OsString::from(cmd)])
        .with_timeout(timeout);
    let output = runner.run(query.as_bytes()).map_err(fail)?;
    if !output.status.success() {
        let status = output.status;
        return Err(Error::Config(format!("embed-cmd exited with {status}")));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| Error::Config(format!("embed-cmd stdout read failed: {e}")))?;
    embedding_input::parse_payload(&stdout)
}

/// Restate a bounded-run failure in the embed command's own wording, which
/// `comemory doctor` and `POST /api/v1/doctor/reembed` already surface.
fn fail(failure: ProcessFailure) -> Error {
    let message = match failure {
        ProcessFailure::TimedOut { .. } => "embed-cmd timed out".to_string(),
        ProcessFailure::Spawn(e) => format!("embed-cmd spawn failed: {e}"),
        ProcessFailure::Io { phase, message } => format!("embed-cmd {phase} failed: {message}"),
        ProcessFailure::InputTooLarge { bytes, max } => {
            format!("embed-cmd query of {bytes} bytes exceeds the {max}-byte limit")
        }
        ProcessFailure::StdoutTooLarge { max } => {
            format!("embed-cmd output exceeds the {max}-byte limit")
        }
    };
    Error::Config(message)
}

#[cfg(test)]
#[path = "tests/embed.rs"]
mod tests;
