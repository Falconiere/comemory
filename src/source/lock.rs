//! Exclusive advisory lock over a sibling lock file, generalized from the
//! `sources.toml.lock` guard it started as.
//!
//! `registry::Registry::register` / `unregister` hold one of these for
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
//! Stays under `src/source/` rather than moving to `src/store/` or a
//! shared location: it is fully generic already (a path plus a label), the
//! move would only relocate a handful of lines while touching every
//! caller, and `src.nested` in `guardrails.config.json` would need a new
//! allowlist entry for wherever it landed. Binding Rule 1 (no duplication)
//! is satisfied by reuse in place, not by relocation.

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
#[path = "tests/lock.rs"]
mod tests;
