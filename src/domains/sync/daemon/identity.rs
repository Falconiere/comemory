//! Who a coordinator is: the canonical data directory it serves, that
//! directory's short id (unit names, fallback socket names), and the binary
//! it runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Paths;
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

/// The binary a coordinator runs — what `ensure` compares (D13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryIdentity {
    /// `CARGO_PKG_VERSION` of the build.
    pub version: String,
    /// Canonical path of the executable.
    pub path: PathBuf,
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
        })
    }
}

#[cfg(test)]
#[path = "tests/identity.rs"]
mod tests;
