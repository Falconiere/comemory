//! The blocking-pool bridge shared by the `serve` and `mcp` adapters.
//!
//! Every command core's DB work — and the `MutexGuard` it takes on the
//! shared connection — must never run on (or cross an `.await` on) the
//! async runtime's own worker threads. [`run_blocking`] runs the whole
//! closure, guard included, inside a `spawn_blocking` task, so callers must
//! lock the connection only inside that closure and never hold it across an
//! `.await`.

use crate::prelude::*;

/// Run `f` on the blocking-thread-pool, flattening a `JoinError` (task
/// panic) into the crate `Error` so callers can just `?` through it.
///
/// The flattening only fires under `panic = "unwind"` (dev/test default).
/// This crate's `[profile.release]`/`[profile.dist]` — every shipped
/// binary — set `panic = "abort"`, under which a panic here aborts the
/// whole process for every connected client before this branch can run.
pub async fn run_blocking<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("blocking task panicked: {e}")))?
}
