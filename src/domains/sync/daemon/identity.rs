//! Who a coordinator is: the canonical data directory it serves, that
//! directory's short id (unit names, fallback socket names), and the binary
//! it runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::domains::sync::daemon::readiness::Readiness;
use crate::prelude::*;
use crate::utilities::digest::sha256_hex;

/// Hex digits of the data-directory digest in a unit name.
pub const UNIT_ID_LEN: usize = 12;

/// Hex digits of the data-directory digest in a fallback socket name.
pub const SOCKET_ID_LEN: usize = 16;

/// The canonical form of `paths`' data directory, which must exist.
///
/// # Errors
/// The directory is missing or cannot be resolved.
pub fn canonical_data_dir(paths: &Paths) -> Result<PathBuf> {
    std::fs::canonicalize(paths.data_dir()).map_err(|e| {
        Error::Unavailable(format!(
            "cannot resolve the data directory {}: {e}",
            paths.data_dir().display()
        ))
    })
}

/// The first `len` hex digits of sha256 over the canonical path's bytes.
#[must_use]
pub fn data_dir_id(canonical: &Path, len: usize) -> String {
    let mut id = sha256_hex(canonical.as_os_str().as_encoded_bytes());
    id.truncate(len);
    id
}

/// The binary a coordinator runs — what `ensure` compares (D13, #258 D3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryIdentity {
    /// `CARGO_PKG_VERSION` of the build.
    pub version: String,
    /// Canonical path of the executable.
    pub path: PathBuf,
    /// `<dev>:<ino>` of the executable file this process runs, captured once
    /// per process; `None` where it cannot be read.
    pub file: Option<String>,
}

impl BinaryIdentity {
    /// This process's binary.
    ///
    /// # Errors
    /// The executable path cannot be determined.
    pub fn current() -> Result<Self> {
        let exe = std::env::current_exe()?;
        Ok(Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            path: std::fs::canonicalize(&exe).unwrap_or(exe),
            file: running_file().cloned(),
        })
    }
}

/// `<dev>:<ino>` of the file at `path` now, or `None` when it cannot be read.
#[must_use]
pub fn file_id(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", meta.dev(), meta.ino()))
}

/// The file this process was started from, read on first use and kept: an
/// installer may rename a new file over the path afterwards. Linux names the
/// running image itself (`/proc/self/exe`), so it is exact whenever read;
/// elsewhere the executable path is stat'ed at the first call — for a CLI
/// command, its preflight's probe, moments after it started.
fn running_file() -> Option<&'static String> {
    static RUNNING: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    RUNNING
        .get_or_init(|| {
            if cfg!(target_os = "linux") {
                return file_id(Path::new("/proc/self/exe"));
            }
            std::env::current_exe().ok().and_then(|exe| file_id(&exe))
        })
        .as_ref()
}

/// Preflight's verdict on a verified coordinator: keep it unless its binary
/// is gone or [`preflight_replaces`] holds.
#[must_use]
pub fn preflight_accepts(readiness: &Readiness, current: &BinaryIdentity) -> bool {
    readiness.binary.exists()
        && !preflight_replaces(readiness, current, file_id(&current.path).as_deref())
}

/// #258 D3c: a coordinator at the caller's own path is replaced by
/// preflight only when the caller *is* the file on disk (`on_disk`) and the
/// coordinator runs a different file — an installer renamed a new binary
/// over it and died before `ensure`. A coordinator predating `binary_file`
/// is compared by version instead. A caller still running an older file
/// (its own file is not the one on disk) never evicts.
#[must_use]
pub fn preflight_replaces(
    readiness: &Readiness,
    caller: &BinaryIdentity,
    on_disk: Option<&str>,
) -> bool {
    if readiness.binary != caller.path {
        return false;
    }
    let Some(own) = caller.file.as_deref() else {
        return false;
    };
    if on_disk != Some(own) {
        return false;
    }
    match readiness.binary_file.as_deref() {
        Some(running) => running != own,
        None => readiness.version != caller.version,
    }
}

#[cfg(test)]
#[path = "tests/identity.rs"]
mod tests;
