//! `daemon.json`: where the live coordinator bound and who it says it is.
//!
//! Discovery only. A client reads it to find a socket that moved to the
//! private runtime directory; identity always comes from the handshake, and
//! a pid in here is signalled only after `ensure` checks it names a live
//! `comemory` process.

use std::io::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::prelude::*;

/// Record file name, beside the data.
pub const RECORD_FILE: &str = "daemon.json";

/// What a running coordinator records about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRecord {
    /// Its process id.
    pub pid: u32,
    /// Its per-start instance id.
    pub instance: String,
    /// The socket it bound.
    pub socket: PathBuf,
    /// Its binary's version.
    pub version: String,
    /// Its binary's canonical path.
    pub binary: PathBuf,
    /// When it started (RFC 3339).
    pub started_at: String,
    /// Its supervisor.
    pub supervisor: String,
}

fn path(paths: &Paths) -> PathBuf {
    paths.data_dir().join(RECORD_FILE)
}

/// Write the record atomically, mode 0600.
///
/// # Errors
/// The file cannot be written or renamed.
pub fn write(paths: &Paths, record: &RuntimeRecord) -> Result<()> {
    let tmp = paths.data_dir().join(".daemon.json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(serde_json::to_string_pretty(record)?.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path(paths))?;
    Ok(())
}

/// Read the record; a missing or unparsable one is `None` — it is advice.
#[must_use]
pub fn read(paths: &Paths) -> Option<RuntimeRecord> {
    let raw = std::fs::read_to_string(path(paths)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Remove the record when it still names `instance`, so a stopping
/// coordinator never deletes its successor's.
///
/// # Errors
/// The file exists, names `instance`, and cannot be removed.
pub fn remove_if_ours(paths: &Paths, instance: &str) -> Result<()> {
    if read(paths).is_some_and(|r| r.instance == instance) {
        match std::fs::remove_file(path(paths)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/runtime_record.rs"]
mod tests;
