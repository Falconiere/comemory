//! Where the control socket lives, and the ownership checks that keep a
//! client from talking to a foreign one.
//!
//! `<data_dir>/daemon.sock` when that path is at most [`MAX_SOCKET_PATH`]
//! bytes. Longer, and the socket moves to `<base>/comemory-<uid>/<id>.sock`,
//! where `<base>` is `$XDG_RUNTIME_DIR`, else `$TMPDIR`, else `/tmp`. That
//! directory must be owned by the data directory's owner with mode exactly
//! 0700 — a pre-created or loosened one is refused, never used.

use std::os::unix::fs::{DirBuilderExt as _, FileTypeExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};

use crate::domains::sync::daemon::identity::{SOCKET_ID_LEN, data_dir_id};
use crate::prelude::*;

/// Socket file name beside the data.
pub const SOCKET_FILE: &str = "daemon.sock";

/// Longest socket path used (`sun_path` is 104 bytes on macOS, 108 on Linux).
pub const MAX_SOCKET_PATH: usize = 100;

/// Owner uid of the canonical data directory — whose socket this is.
///
/// # Errors
/// The directory cannot be read.
pub fn owner_uid(canonical: &Path) -> Result<u32> {
    Ok(std::fs::metadata(canonical)?.uid())
}

/// Where a coordinator for `canonical` binds, creating the private runtime
/// directory when the socket cannot live beside the data.
///
/// # Errors
/// [`Error::Unavailable`] when the private directory is unsafe or even it
/// would give an over-long path.
pub fn plan(canonical: &Path) -> Result<PathBuf> {
    let primary = canonical.join(SOCKET_FILE);
    if fits(&primary) {
        return Ok(primary);
    }
    let uid = owner_uid(canonical)?;
    let dir = private_dir(uid);
    ensure_private_dir(&dir, uid)?;
    fallback_socket(canonical, &dir)
}

/// Where a coordinator for `canonical` would bind, creating nothing.
///
/// # Errors
/// As [`plan`], without the directory creation.
pub fn expected(canonical: &Path) -> Result<PathBuf> {
    let primary = canonical.join(SOCKET_FILE);
    if fits(&primary) {
        return Ok(primary);
    }
    fallback_socket(canonical, &private_dir(owner_uid(canonical)?))
}

fn fits(path: &Path) -> bool {
    path.as_os_str().len() <= MAX_SOCKET_PATH
}

fn fallback_socket(canonical: &Path, dir: &Path) -> Result<PathBuf> {
    let socket = dir.join(format!("{}.sock", data_dir_id(canonical, SOCKET_ID_LEN)));
    if fits(&socket) {
        return Ok(socket);
    }
    Err(Error::Unavailable(format!(
        "the sync daemon socket path {} is over {MAX_SOCKET_PATH} bytes — set TMPDIR to a shorter directory",
        socket.display()
    )))
}

/// `<base>/comemory-<uid>`.
#[must_use]
pub fn private_dir(uid: u32) -> PathBuf {
    let base = ["XDG_RUNTIME_DIR", "TMPDIR"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(format!("comemory-{uid}"))
}

/// Create `dir` 0700, or accept an existing one only when it is a real
/// directory owned by `uid` with mode exactly 0700.
///
/// # Errors
/// [`Error::Unavailable`] naming the fix when the directory is unsafe.
pub fn ensure_private_dir(dir: &Path, uid: u32) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let mode = meta.mode() & 0o777;
    if !meta.is_dir() || meta.uid() != uid || mode != 0o700 {
        return Err(Error::Unavailable(format!(
            "refusing the sync daemon runtime directory {}: it must be a directory owned by uid {uid} with mode 0700 (found uid {} mode {mode:o}) — remove it and retry",
            dir.display(),
            meta.uid()
        )));
    }
    Ok(())
}

/// A client's check before it connects: `socket` is a socket (not a
/// symlink) owned by `uid`, in a directory no one else can write to.
///
/// # Errors
/// [`Error::Unavailable`] describing what is wrong.
pub fn check_socket(socket: &Path, uid: u32) -> Result<()> {
    let meta = std::fs::symlink_metadata(socket)?;
    if !meta.file_type().is_socket() {
        return Err(Error::Unavailable(format!(
            "{} is not a socket",
            socket.display()
        )));
    }
    if meta.uid() != uid {
        return Err(Error::Unavailable(format!(
            "{} belongs to uid {}, not {uid}",
            socket.display(),
            meta.uid()
        )));
    }
    let parent = socket.parent().unwrap_or_else(|| Path::new("/"));
    if std::fs::metadata(parent)?.mode() & 0o022 != 0 {
        return Err(Error::Unavailable(format!(
            "{} is group- or world-writable — run `chmod go-w {}`",
            parent.display(),
            parent.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/socket_path.rs"]
mod tests;
