//! Wake signal so a sleeping sync daemon ends its interval early.
//!
//! After a successful local save of a sync-eligible memory (non-empty `repo`),
//! the save path touches `$DATA_DIR/sync.wake`. The daemon's interruptible
//! sleep polls that file and runs one pull→push cycle immediately. This is
//! not in-process sync and not a platform broker — local latency only.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Paths;
use crate::prelude::*;

/// Poll slice while waiting for interval or wake (sub-second wake latency).
const SLEEP_SLICE: Duration = Duration::from_millis(200);

/// Absolute path of the wake file for `paths`.
pub fn wake_file(paths: &Paths) -> PathBuf {
    paths.sync_wake_file()
}

/// Best-effort wake after a successful save. No-ops for personal (empty repo)
/// memories; never fails the save.
pub fn wake_after_save_best_effort(paths: &Paths, repo: &str) {
    if repo.trim().is_empty() {
        return;
    }
    if let Err(e) = signal_wake(paths) {
        tracing::debug!(error = %e, "sync daemon wake skipped");
    }
}

/// Create / overwrite the wake file so a sleeping daemon returns early.
///
/// # Errors
/// [`Error::Io`] when the data dir or file cannot be written.
pub fn signal_wake(paths: &Paths) -> Result<()> {
    let path = wake_file(paths);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Contents unused — presence is the signal. Timestamp helps debugging.
    std::fs::write(&path, b"1")?;
    Ok(())
}

/// Remove the wake file if present. Returns whether it was there.
pub fn take_wake(paths: &Paths) -> bool {
    take_wake_at(&wake_file(paths))
}

fn take_wake_at(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            tracing::debug!(error = %e, path = %path.display(), "sync wake clear failed");
            false
        }
    }
}

/// Sleep up to `interval`, returning early when a wake file appears.
///
/// If a wake is already pending when called (e.g. save during the last cycle),
/// returns immediately without sleeping.
pub fn sleep_interruptible(interval: Duration, paths: &Paths) {
    sleep_interruptible_at(interval, &wake_file(paths));
}

/// Testable core: same as [`sleep_interruptible`] against an explicit path.
pub fn sleep_interruptible_at(interval: Duration, wake_path: &Path) {
    if take_wake_at(wake_path) {
        tracing::debug!("sync daemon woken (pending wake)");
        return;
    }
    let deadline = Instant::now() + interval;
    loop {
        if take_wake_at(wake_path) {
            tracing::debug!("sync daemon woken by save");
            return;
        }
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        thread::sleep(SLEEP_SLICE.min(deadline.saturating_duration_since(now)));
    }
}

#[cfg(test)]
#[path = "tests/daemon_wake.rs"]
mod tests;
