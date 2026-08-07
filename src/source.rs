//! Durable source registry: TOML-backed registration of external document
//! roots (`sources.toml`), an exclusive-flock guard over concurrent
//! read-modify-write cycles, and the reconciler that mirrors the registry
//! into SQLite's `source_roots` table. See the design spec's "Durable
//! source registry" section.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

/// v1 allowlist classification (extension + binary sniff), plus the
/// discovery walk's managed-directory exclusion outcome.
pub mod classify;
/// Discovery walk over a registered source root: boundary/ignore rules
/// plus classification, producing a deterministic candidate list.
pub mod discover;
/// Exclusive flock guard over the `sources.toml.lock` sibling file.
pub mod lock;
/// Reconciles the TOML registry into the SQLite `source_roots` mirror.
pub mod mirror;
/// `sources.toml` load/save, overlap validation, atomic durability.
pub mod registry;

/// Stable 128-bit lowercase-hex identifier for a registered source root.
///
/// Deterministically derived from the canonical path — the first 16 bytes
/// of `SHA-256(canonical path)`, hex-encoded — the same "hash the
/// identity" idiom [`crate::memory::id::memory_id`] uses for memory ids.
/// A path's id therefore never changes across re-registration, which is
/// what makes [`registry::Registry::register`]'s update-not-duplicate
/// behavior hold: the same canonical path always resolves to the same id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    /// Derive the id for an already-canonicalized path.
    pub fn from_canonical_path(canonical_path: &Path) -> Self {
        let digest = Sha256::digest(canonical_path.to_string_lossy().as_bytes());
        let mut hex = String::with_capacity(32);
        for byte in &digest[..16] {
            use fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }

    /// Borrow the 32-lowercase-hex-char string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a registered source root is a single file or a directory tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// A single registered file.
    File,
    /// A registered directory tree, walked recursively.
    Dir,
}

impl SourceKind {
    /// Canonical lowercase string used in TOML, SQL, and CLI output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
        }
    }
}

/// One registered source root, as stored in `sources.toml` and mirrored
/// into `source_roots`. Ephemeral state (reachability status, discovery
/// counts) lives only in the SQLite mirror, reconciled from this struct
/// by [`mirror::reconcile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    /// Stable identifier — see [`SourceId`].
    pub id: SourceId,
    /// Absolute, symlink-resolved path this entry roots.
    pub canonical_path: PathBuf,
    /// File-or-directory classification of `canonical_path`.
    pub kind: SourceKind,
    /// Optional repository label for `--repo` filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// When this entry was first registered.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When this entry's settings were last changed.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}
