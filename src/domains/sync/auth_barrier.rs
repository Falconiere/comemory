//! The durable logout barrier, `$COMEMORY_DATA_DIR/auth.disabled`. Logout
//! raises it first; only an explicit login ([`AuthFile::save`]) clears it.
//! While it stands every credential read answers "logged out" — even with
//! `auth.json` on disk or `COMEMORY_API_KEY` inherited — and every drain stops
//! at its next batch boundary ([`crate::domains::sync::drain::stop`]).
//!
//! [`AuthFile::save`]: crate::domains::sync::AuthFile::save

use std::io::Write as _;
use std::path::PathBuf;

use crate::config::Paths;
use crate::domains::sync::drain::network;
use crate::prelude::*;

/// File name of the barrier, beside `auth.json`.
pub const BARRIER_FILE: &str = "auth.disabled";

fn path(paths: &Paths) -> PathBuf {
    paths.data_dir().join(BARRIER_FILE)
}

/// Raise the barrier durably: the file and its directory entry are synced
/// before this returns, so a crash right after a logout cannot bring the
/// credential back.
///
/// # Errors
/// The data directory cannot be created or the file written.
pub fn raise(paths: &Paths) -> Result<()> {
    std::fs::create_dir_all(paths.data_dir())?;
    let body = serde_json::json!({ "at": network::now()?, "reason": "logout" });
    let target = path(paths);
    let tmp = paths.data_dir().join(".auth.disabled.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(serde_json::to_string(&body)?.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, &target)?;
    std::fs::File::open(paths.data_dir())?.sync_all()?;
    Ok(())
}

/// Whether the barrier stands. Anything at the path counts: a barrier that
/// cannot be read is still a barrier.
#[must_use]
pub fn active(paths: &Paths) -> bool {
    path(paths).symlink_metadata().is_ok()
}

/// Remove the barrier; already gone is success.
///
/// # Errors
/// The file exists and cannot be removed.
pub fn clear(paths: &Paths) -> Result<()> {
    match std::fs::remove_file(path(paths)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
#[path = "tests/auth_barrier.rs"]
mod tests;
