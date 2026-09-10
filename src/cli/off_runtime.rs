//! Run blocking platform I/O outside the tokio runtime.
//!
//! `main` is `#[tokio::main]`, so every subcommand body executes inside an
//! async context. `sync::client` uses `reqwest::blocking`, which builds its
//! own current-thread runtime per call and panics on drop when it finds
//! another runtime already in scope:
//!
//! ```text
//! Cannot drop a runtime in a context where blocking is not allowed.
//! ```
//!
//! `tokio::task::block_in_place` does not help — the runtime handle is still
//! current, which is exactly what `reqwest` objects to. A plain scoped thread
//! has no handle at all, so the blocking client builds and drops cleanly.
//!
//! Only the CLI needs this. `serve` already runs `api::` bodies inside
//! `spawn_blocking`, and `sync::auto` already detaches its own thread.

use crate::prelude::*;

/// Run `f` on a scoped thread with no tokio runtime in scope, and return what
/// it returned.
///
/// # Errors
/// Propagates `f`'s error. A panic inside `f` becomes [`Error::Other`] rather
/// than unwinding through the runtime.
pub fn off_runtime<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send,
    T: Send,
{
    std::thread::scope(|scope| scope.spawn(f).join())
        .map_err(|_| Error::Other("platform request thread panicked".into()))?
}
