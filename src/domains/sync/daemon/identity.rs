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

/// A Homebrew keg file `<prefix>/Cellar/<formula>/<keg>/<rel>`, split up.
struct KegFile {
    prefix: PathBuf,
    formula: std::ffi::OsString,
    keg: std::ffi::OsString,
    rel: PathBuf,
}

fn keg_file(path: &Path) -> Option<KegFile> {
    let parts: Vec<_> = path.components().collect();
    let cellar = parts.iter().rposition(|c| c.as_os_str() == "Cellar")?;
    let rel: PathBuf = parts.get(cellar + 3..)?.iter().collect();
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(KegFile {
        prefix: parts.get(..cellar)?.iter().collect(),
        formula: parts.get(cellar + 1)?.as_os_str().to_os_string(),
        keg: parts.get(cellar + 2)?.as_os_str().to_os_string(),
        rel,
    })
}

/// homebrew-tap#1: for a Homebrew keg file `exe`, the stable
/// `<prefix>/opt/<formula>/<rel>` link — only when that link resolves to
/// `exe` now, i.e. `exe` is the formula's linked install. A unit that runs
/// this path follows `brew upgrade` instead of pinning one keg, which
/// `brew cleanup` later deletes.
///
/// The returned path is the link itself, deliberately not canonicalized.
/// Identity checks never compare against it: the coordinator the unit starts
/// canonicalizes its own executable ([`BinaryIdentity::current`]) and so
/// reports the keg file, exactly like a caller that ran the keg directly.
#[must_use]
pub fn homebrew_opt_link(exe: &Path) -> Option<PathBuf> {
    let keg = keg_file(exe)?;
    let link = keg.prefix.join("opt").join(&keg.formula).join(&keg.rel);
    (std::fs::canonicalize(&link).ok()? == exe).then_some(link)
}

/// Whether `running` is another keg of the Homebrew formula whose linked
/// install `caller` is. `brew upgrade` keeps the old keg on macOS, so its
/// file still exists and would otherwise keep the old coordinator alive.
fn superseded_keg(running: &Path, caller: &Path) -> bool {
    let (Some(old), Some(new)) = (keg_file(running), keg_file(caller)) else {
        return false;
    };
    old.prefix == new.prefix
        && old.formula == new.formula
        && old.rel == new.rel
        && old.keg != new.keg
        && homebrew_opt_link(caller).is_some()
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
///
/// homebrew-tap#1: a coordinator on another keg of the Homebrew formula the
/// caller is the linked (`opt`) install of is replaced too — `brew upgrade`
/// switched the link and kept the old keg. A caller on an old keg is not the
/// linked install, so it never evicts.
#[must_use]
pub fn preflight_replaces(
    readiness: &Readiness,
    caller: &BinaryIdentity,
    on_disk: Option<&str>,
) -> bool {
    if superseded_keg(&readiness.binary, &caller.path) {
        return true;
    }
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
