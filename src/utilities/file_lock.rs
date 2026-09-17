//! Exclusive advisory lock over a sibling lock file, generalized from the
//! `sources.toml.lock` guard it started as.
//!
//! `source::registry::Registry::register` / `unregister` hold one of these for
//! their whole read-modify-write cycle so two concurrent `comemory index`
//! (or `unindex`) invocations serialize instead of the last writer silently
//! discarding the first registration (spec: "Concurrent registrations").
//! `store::migrate`'s preflight snapshot (`VACUUM INTO`, which SQLite will
//! not serialize on its own) is a second consumer, hence the neutral
//! `FileLock` name and the `label` carried alongside it. Built entirely on
//! `std::fs::File::lock`/`unlock` (stable since Rust 1.89), which wraps
//! `flock(2)` on unix and `LockFileEx` on Windows internally — no FFI, no
//! `unsafe`, and no new dependency needed here.
//!
//! Two owners across two future domains (documents and infrastructure) is
//! exactly why #166 gave it a neutral home here rather than leaving it under
//! `src/source/`, where `store::migrate::preflight` would have had to reach
//! into the documents capability for a lock.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::prelude::*;

/// RAII guard over an exclusive lock on a sibling lock file. The lock is
/// released when the guard drops (explicit `unlock`, backstopped by the
/// file descriptor closing regardless).
pub struct FileLock {
    file: File,
    label: &'static str,
}

impl FileLock {
    /// Open (creating if absent) the lock file at `path` and block until
    /// an exclusive lock is acquired. `label` names the caller (e.g.
    /// `"registry"`, `"migration"`) so a failed-unlock warning identifies
    /// which lock misbehaved.
    pub fn acquire(path: &Path, label: &'static str) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        file.lock()?;
        Ok(Self { file, label })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if let Err(e) = self.file.unlock() {
            tracing::warn!(error = %e, label = self.label, "file lock: unlock failed");
        }
    }
}

#[cfg(test)]
#[path = "tests/file_lock.rs"]
mod tests;
