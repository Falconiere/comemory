//! Exclusive advisory lock over the `sources.toml.lock` sibling file.
//!
//! `registry::Registry::register` / `unregister` hold this for their
//! whole read-modify-write cycle so two concurrent `comemory index` (or
//! `unindex`) invocations serialize instead of the last writer silently
//! discarding the first registration (spec: "Concurrent registrations").
//! Built entirely on `std::fs::File::lock`/`unlock` (stable since Rust
//! 1.89), which wraps `flock(2)` on unix and `LockFileEx` on Windows
//! internally — no FFI, no `unsafe`, and no new dependency needed here.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::prelude::*;

/// RAII guard over an exclusive lock on the registry lock file. The lock
/// is released when the guard drops (explicit `unlock`, backstopped by
/// the file descriptor closing regardless).
pub struct RegistryLock {
    file: File,
}

impl RegistryLock {
    /// Open (creating if absent) the lock file at `path` and block until
    /// an exclusive lock is acquired.
    pub fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        file.lock()?;
        Ok(Self { file })
    }
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        if let Err(e) = self.file.unlock() {
            tracing::warn!(error = %e, "registry lock: unlock failed");
        }
    }
}

#[cfg(test)]
#[path = "tests/lock.rs"]
mod tests;
