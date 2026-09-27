//! The exchange gate (#256, B-3): the one way a maintenance command that
//! replaces or copies the whole store — `rebuild`, `backup restore`,
//! `backup create` — stops replication for its duration.
//!
//! Every exchange path already serializes on [`auto::PASS_LOCK`]: the
//! resident coordinator's pass worker, a manual `comemory sync`, the raw
//! channel follower and an `--action auto` pass block on it, and the inline
//! push only tries it. Holding it is therefore the pause: an in-flight pass
//! finishes before [`pause`] returns (the drain), and the coordinator's
//! queued pass runs as soon as the [`ExchangePause`] drops (the resume). It
//! is outermost in the lock order — taken before `memory-save.lock`.
//!
//! [`auto::PASS_LOCK`]: crate::domains::sync::auto::PASS_LOCK

use std::time::Duration;

use crate::config::paths::Paths;
use crate::domains::sync::auto::PASS_LOCK;
use crate::prelude::*;
use crate::utilities::file_lock::FileLock;

/// The held exchange gate; replication resumes when it drops. Held for its
/// `Drop` (the release), never read.
#[derive(Debug)]
pub struct ExchangePause {
    _lock: FileLock,
}

/// Pause the exchange, waiting up to `wait` for a pass already running to
/// finish.
///
/// # Errors
/// [`Error::Busy`] once `wait` elapses with a pass still holding the gate;
/// nothing has been paused then. Propagates the lock file's own I/O
/// failures.
pub fn pause(paths: &Paths, wait: Duration) -> Result<ExchangePause> {
    FileLock::acquire_within(&paths.data_dir().join(PASS_LOCK), "exchange-gate", wait)?
        .map(|lock| ExchangePause { _lock: lock })
        .ok_or_else(|| Error::Busy(PASS_LOCK.to_string()))
}

#[cfg(test)]
#[path = "tests/exchange_gate.rs"]
mod tests;
