//! The compiler-enforced lock every markdown writer holds (#256, B-6):
//! `SaveGuard` can only be constructed by [`acquire_within`], over
//! `memory-save.lock`, so `MemoryStore::{save, rewrite}` and the trash
//! move/restore/purge helpers cannot be called without holding one.
//!
//! One process may already hold it — a save racing `comemory rebuild`'s
//! writer pause, or two saves of the same id — so the wait is bounded by
//! `[sync] pause_wait` rather than blocking forever.

use std::time::Duration;

use crate::config::paths::Paths;
use crate::prelude::*;
use crate::utilities::file_lock::FileLock;

/// Proof of holding `memory-save.lock`. The private field means only
/// [`acquire_within`] can construct one, so a write method that takes
/// `&SaveGuard` cannot be reached without it — a compile error, not a
/// runtime race, catches a writer that forgot to acquire. Held for its
/// `Drop` (the release), never read — the leading underscore says so.
#[derive(Debug)]
pub struct SaveGuard {
    _lock: FileLock,
}

/// `memory-save.lock`'s path under `paths`' data directory.
pub(crate) fn lock_path(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir().join("memory-save.lock")
}

/// Acquire `memory-save.lock`, waiting up to `timeout` for another holder —
/// this process or any other — to release it.
///
/// # Errors
/// [`Error::Busy`] once `timeout` elapses with the lock still held.
/// Propagates the lock file's own I/O failures.
pub fn acquire_within(paths: &Paths, timeout: Duration) -> Result<SaveGuard> {
    FileLock::acquire_within(&lock_path(paths), "memory-save", timeout)?
        .map(|lock| SaveGuard { _lock: lock })
        .ok_or_else(|| Error::Busy("memory-save.lock".to_string()))
}

#[cfg(test)]
#[path = "tests/save_lock.rs"]
mod tests;
