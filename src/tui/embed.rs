//! Embed-command shell-out for Memory-tab semantic enrichment.
//!
//! Spawns the user-configured command via `sh -c`, pipes the query to its
//! stdin, and parses a JSON `{"embedding":[..]}` payload from its stdout. The
//! read is bounded by [`EMBED_TIMEOUT`] so a hung embedder cannot pin the
//! DB-worker thread. Every failure path returns an `Error` for the caller to
//! surface on the status line — it never panics and never blocks forever.

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
    let mut child = spawn(cmd)?;
    write_stdin(&mut child, query)?;
    let stdout = read_with_timeout(&mut child)?;
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
fn write_stdin(child: &mut Child, query: &str) -> Result<()> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Config("embed-cmd stdin unavailable".into()))?;
    stdin
        .write_all(query.as_bytes())
        .map_err(|e| fail("stdin write", e))
}

/// Read the child's stdout to EOF on a helper thread, bounded by
/// [`EMBED_TIMEOUT`]; on timeout the child is killed and an error returned.
fn read_with_timeout(child: &mut Child) -> Result<String> {
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
    match rx.recv_timeout(EMBED_TIMEOUT) {
        Ok(Ok(buf)) => Ok(buf),
        Ok(Err(e)) => Err(fail("stdout read", e)),
        Err(_) => {
            let _ = child.kill();
            Err(Error::Config("embed-cmd timed out".into()))
        }
    }
}

/// Build a `Config` error tagged with the failing embed-cmd phase.
fn fail(phase: &str, e: std::io::Error) -> Error {
    Error::Config(format!("embed-cmd {phase} failed: {e}"))
}
