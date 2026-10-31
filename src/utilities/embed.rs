//! Bounded embed-command execution shared by all delivery adapters.
//!
//! Content travels only on stdin. Concurrent pipe workers prevent deadlocks;
//! a supervisor bounds writes, reads and process exit with one deadline.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::prelude::*;
use crate::utilities::{embedding_input, progress::ProgressSink};

/// Whole-invocation deadline, allowing cold local inference.
pub const EMBED_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum JSON vector response captured from a provider.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 4096;

/// Vectorize text with the configured shell command and a bounded lifetime.
pub fn embed_query(cmd: &str, query: &str) -> Result<Vec<f32>> {
    embed_query_cancellable(cmd, query, None)
}

/// Execute with an explicit whole-invocation deadline (also useful to tests).
pub fn embed_query_with_timeout(cmd: &str, query: &str, timeout: Duration) -> Result<Vec<f32>> {
    execute(cmd, query, timeout, None)
}

/// Vectorize with cooperative cancellation, terminating the provider group promptly.
pub fn embed_query_cancellable(
    cmd: &str,
    query: &str,
    sink: Option<&dyn ProgressSink>,
) -> Result<Vec<f32>> {
    execute(cmd, query, EMBED_TIMEOUT, sink)
}

fn execute(
    cmd: &str,
    query: &str,
    timeout: Duration,
    sink: Option<&dyn ProgressSink>,
) -> Result<Vec<f32>> {
    if sink.is_some_and(ProgressSink::is_cancelled) {
        return Err(Error::Cancelled);
    }
    let deadline = Instant::now() + timeout;
    let child = Command::new("sh")
        .args(["-c", cmd])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| fail("spawn", e))?;
    let mut provider = Provider(child, false);
    let stdin = provider
        .0
        .stdin
        .take()
        .ok_or_else(|| fail("stdin", "unavailable"))?;
    let stdout = provider
        .0
        .stdout
        .take()
        .ok_or_else(|| fail("stdout", "unavailable"))?;
    let stderr = provider
        .0
        .stderr
        .take()
        .ok_or_else(|| fail("stderr", "unavailable"))?;
    let (tx, rx) = mpsc::channel();
    let input = query.as_bytes().to_vec();
    let writer_tx = tx.clone();
    let writer = std::thread::spawn(move || {
        let mut stdin = stdin;
        let result = match stdin.write_all(&input) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(fail("stdin write", e)),
            _ => Ok(None),
        };
        let _ = writer_tx.send(result);
    });
    let output_tx = tx.clone();
    let reader = std::thread::spawn(move || {
        let _ = output_tx.send(capture(stdout, MAX_OUTPUT_BYTES, true).map(Some));
    });
    let private_input = query.to_string();
    let private_command = cmd.to_string();
    let diagnostics = std::thread::spawn(move || {
        let result = capture(stderr, MAX_DIAGNOSTIC_BYTES, false).map(|(kind, text)| {
            Some((
                kind,
                sanitize_diagnostic(&text, &private_command, &private_input),
            ))
        });
        let _ = tx.send(result);
    });
    let result = supervise(&mut provider.0, &rx, deadline, sink);
    // Kill descendants even after a successful shell exit: they may own pipes.
    provider.terminate();
    for worker in [writer, reader, diagnostics] {
        if worker.join().is_err() {
            return Err(fail("pipe worker", "panicked"));
        }
    }
    result
}

// The stream identity must travel with its payload, independent of arrival order.
fn capture(mut stream: impl Read, limit: usize, reject_large: bool) -> Result<(bool, String)> {
    let mut captured = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let n = stream.read(&mut buffer).map_err(|e| fail("pipe read", e))?;
        if n == 0 {
            break;
        }
        if reject_large && captured.len() + n > limit {
            return Err(fail("stdout", "exceeds 1 MiB response limit"));
        }
        let keep = n.min(limit.saturating_sub(captured.len()));
        captured.extend_from_slice(&buffer[..keep]);
    }
    Ok((
        reject_large,
        String::from_utf8_lossy(&captured).into_owned(),
    ))
}

fn sanitize_diagnostic(raw: &str, command: &str, input: &str) -> String {
    let mut text = raw.replace(command, "<command>");
    for line in input.lines().filter(|line| line.len() >= 8) {
        text = text.replace(line, "<input>");
    }
    if crate::utilities::secret_scan::scan(&text).is_some() {
        "provider diagnostics withheld because they contain a secret".into()
    } else {
        text
    }
}

type PipeResult = Result<Option<(bool, String)>>;

fn supervise(
    child: &mut Child,
    rx: &mpsc::Receiver<PipeResult>,
    deadline: Instant,
    sink: Option<&dyn ProgressSink>,
) -> Result<Vec<f32>> {
    let mut completed = 0;
    let mut output = String::new();
    let mut diagnostic = String::new();
    loop {
        if sink.is_some_and(ProgressSink::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(fail(
                "invocation",
                "timed out; check provider availability and context/batch settings",
            ));
        }
        if completed < 3 {
            match rx.recv_timeout(remaining.min(Duration::from_millis(5))) {
                Ok(result) => {
                    completed += 1;
                    match result? {
                        Some((true, text)) => output = text,
                        Some((false, text)) => diagnostic = text,
                        None => {}
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(fail("pipes", "disconnected"));
                }
            }
        } else {
            std::thread::sleep(remaining.min(Duration::from_millis(5)));
        }
        if let Some(status) = child.try_wait().map_err(|e| fail("wait", e))? {
            if completed != 3 {
                continue;
            }
            if !status.success() {
                return Err(fail("exit", format!("{status}; {}", diagnostic.trim())));
            }
            return embedding_input::parse_payload(&output)
                .map_err(|_| fail("payload", "expected JSON {\"embedding\":[finite numbers]}"));
        }
    }
}

struct Provider(Child, bool);

impl Provider {
    fn terminate(&mut self) {
        if self.1 {
            return;
        }
        self.1 = true;
        // Each invocation owns a fresh process group, including shell children.
        if let Ok(id) = i32::try_from(self.0.id()) {
            // SAFETY: kill targets only the process group created for this child.
            unsafe { nix::libc::kill(-id, nix::libc::SIGKILL) };
        }
        let _ = self.0.wait();
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn fail(phase: &str, error: impl std::fmt::Display) -> Error {
    Error::Config(format!("embed-cmd {phase} failed: {error}"))
}

#[cfg(test)]
#[path = "tests/embed.rs"]
mod tests;
