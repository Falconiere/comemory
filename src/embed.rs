//! Shared embed-command shell-out (Memory-tab semantic enrich + `serve`).
//!
//! Spawns the user-configured command via `sh -c`, pipes the query to its
//! stdin, and parses a JSON `{"embedding":[..]}` payload from its stdout. The
//! read is bounded by [`EMBED_TIMEOUT`] so a hung embedder cannot pin the
//! caller's thread. Every failure path returns an `Error` for the caller to
//! surface — it never panics and never blocks forever.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::cli::embedding_input;
use crate::prelude::*;

/// Maximum time to wait for the embed command to produce its vector.
pub const EMBED_TIMEOUT: Duration = Duration::from_secs(10);

/// Vectorize `query` via `cmd` (`sh -c <cmd>`), bounded by [`EMBED_TIMEOUT`].
/// Returns the parsed embedding, or an `Error` describing the failed phase.
pub fn embed_query(cmd: &str, query: &str) -> Result<Vec<f32>> {
    embed_query_with_timeout(cmd, query, EMBED_TIMEOUT)
}

/// [`embed_query`] with an explicit read `timeout`. Exposed so tests can drive
/// the timeout path with a tiny bound instead of waiting [`EMBED_TIMEOUT`].
pub fn embed_query_with_timeout(cmd: &str, query: &str, timeout: Duration) -> Result<Vec<f32>> {
    let mut child = spawn(cmd)?;
    write_stdin(&mut child, query)?;
    let stdout = read_with_timeout(&mut child, timeout)?;
    let status = child.wait().map_err(|e| fail("wait", e))?;
    if !status.success() {
        return Err(Error::Config(format!("embed-cmd exited with {status}")));
    }
    embedding_input::parse_payload(&stdout)
}

/// Spawn `sh -c <cmd>` with piped stdin/stdout and a silenced stderr.
fn spawn(cmd: &str) -> Result<Child> {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| fail("spawn", e))
}

/// Write `query` to the child's stdin and close it (signals EOF on drop).
///
/// A command may close its end of the pipe, or exit, before the whole
/// query has been written — whether it never reads stdin at all (a
/// `printf` of a canned payload, a script that embeds from an argument)
/// or stops after consuming part of it. `write_all` loops over write
/// syscalls, and the first one to hit the closed read end fails with
/// `EPIPE`, which surfaces here as `ErrorKind::BrokenPipe` no matter how
/// many bytes went through first. That is the command's choice, not a
/// failure: its exit status and stdout still decide the outcome, so a
/// broken pipe is swallowed and every other write error propagates.
fn write_stdin(child: &mut Child, query: &str) -> Result<()> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Config("embed-cmd stdin unavailable".into()))?;
    match stdin.write_all(query.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(fail("stdin write", e)),
    }
}

/// Read the child's stdout to EOF on a helper thread, bounded by
/// [`EMBED_TIMEOUT`]; on timeout the child is killed and an error returned.
fn read_with_timeout(child: &mut Child, timeout: Duration) -> Result<String> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Config("embed-cmd stdout unavailable".into()))?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let res = stdout.read_to_string(&mut buf).map(|_| buf);
        let _ = tx.send(res);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(buf)) => Ok(buf),
        Ok(Err(e)) => Err(fail("stdout read", e)),
        Err(_) => {
            // Kill AND reap: `Child::kill` only signals; without `wait` the
            // SIGKILLed child lingers as a zombie until this process exits.
            let _ = child.kill();
            let _ = child.wait();
            Err(Error::Config("embed-cmd timed out".into()))
        }
    }
}

/// Build a `Config` error tagged with the failing embed-cmd phase.
fn fail(phase: &str, e: std::io::Error) -> Error {
    Error::Config(format!("embed-cmd {phase} failed: {e}"))
}

#[cfg(test)]
#[path = "tests/embed.rs"]
mod tests;
