//! `sources.toml` load/save: the durable, authoritative record of
//! registered document roots. Mutations run under an exclusive
//! [`crate::source::lock::FileLock`] for their whole
//! read-modify-write cycle (spec: "Concurrent registrations").

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::config::paths::Paths;
use crate::prelude::*;
use crate::source::lock::FileLock;
use crate::source::{SourceEntry, SourceId, SourceKind};

/// On-disk shape of `sources.toml`: a format tag plus the `[[source]]`
/// array of tables.
#[derive(Debug, Serialize, Deserialize)]
struct RegistryFile {
    format: u32,
    #[serde(default)]
    source: Vec<SourceEntry>,
}

/// Filesystem-backed CRUD over `<data-dir>/sources.toml`.
#[derive(Debug, Clone)]
pub struct Registry {
    paths: Paths,
}

impl Registry {
    /// Construct a registry rooted at `paths`. Callers are responsible
    /// for `paths.ensure_dirs()` having run at least once (same contract
    /// as [`crate::memory::MemoryStore`]).
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    /// Load every registered entry. A missing file loads as empty — a
    /// fresh data dir has no sources yet.
    pub fn load(&self) -> Result<Vec<SourceEntry>> {
        let path = self.paths.sources_file();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path)?;
        let file: RegistryFile = toml::from_str(&raw)?;
        Ok(file.source)
    }

    /// Register `path` (canonicalized, symlinks resolved) as a source
    /// root, or update its `repo` label (and re-detected `kind`) if it is
    /// already registered. Rejects a path that overlaps (contains, or is
    /// contained by) an existing, different registered path. Held under
    /// the registry lock for the whole read-modify-write.
    pub fn register(&self, path: &Path, repo: Option<String>) -> Result<SourceEntry> {
        let canonical = fs::canonicalize(path)?;
        let kind = kind_of(&canonical)?;
        let _guard = FileLock::acquire(&self.paths.sources_lock_file(), "registry")?;
        let mut entries = self.load()?;
        let now = OffsetDateTime::now_utc();

        let entry = if let Some(i) = entries.iter().position(|e| e.canonical_path == canonical) {
            entries[i].kind = kind;
            entries[i].repo = repo;
            entries[i].updated_at = now;
            entries[i].clone()
        } else {
            validate_no_overlap(&entries, &canonical)?;
            let e = SourceEntry {
                id: SourceId::from_canonical_path(&canonical),
                canonical_path: canonical,
                kind,
                repo,
                created_at: now,
                updated_at: now,
            };
            entries.push(e.clone());
            e
        };

        self.save(&entries)?;
        Ok(entry)
    }

    /// Unregister the source matching `id_or_path` (a [`SourceId`] hex
    /// string, or a filesystem path resolved and matched against a
    /// registered canonical path). Returns the removed entry, or `None`
    /// when nothing matched. Held under the registry lock for the whole
    /// read-modify-write.
    pub fn unregister(&self, id_or_path: &str) -> Result<Option<SourceEntry>> {
        // `.ok()`: a canonicalize failure (path no longer exists, or
        // `id_or_path` is a SourceId hex string rather than a path at
        // all) just makes `target` `None`, so the path-match arm below
        // never matches — the id-match arm still works unconditionally.
        // Intended: only unregistering by a since-deleted path is
        // affected, and that falls through to "not found" rather than
        // erroring.
        let target = fs::canonicalize(id_or_path).ok();
        let _guard = FileLock::acquire(&self.paths.sources_lock_file(), "registry")?;
        let mut entries = self.load()?;
        let pos = entries.iter().position(|e| {
            e.id.as_str() == id_or_path || Some(e.canonical_path.as_path()) == target.as_deref()
        });
        let Some(i) = pos else {
            return Ok(None);
        };
        let removed = entries.remove(i);
        self.save(&entries)?;
        Ok(Some(removed))
    }

    /// Atomic tmp-file + rename write, mirroring
    /// `memory::store::MemoryStore::save`'s durability pattern: on any
    /// failure between staging and rename, the tmp file is removed.
    fn save(&self, entries: &[SourceEntry]) -> Result<()> {
        let file = RegistryFile {
            format: 1,
            source: entries.to_vec(),
        };
        let rendered =
            toml::to_string_pretty(&file).map_err(|e| Error::Other(format!("toml: {e}")))?;
        let final_path = self.paths.sources_file();
        let tmp_path: PathBuf = self.paths.data_dir().join(".sources.toml.tmp");

        if let Err(e) = fs::write(&tmp_path, &rendered) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        if let Err(e) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        Ok(())
    }
}

/// Classify `canonical` (already resolved) as [`SourceKind::File`] or
/// [`SourceKind::Dir`] from live filesystem metadata.
fn kind_of(canonical: &Path) -> Result<SourceKind> {
    let meta = fs::metadata(canonical)?;
    if meta.is_dir() {
        Ok(SourceKind::Dir)
    } else if meta.is_file() {
        Ok(SourceKind::File)
    } else {
        Err(Error::Usage(format!(
            "source path is neither a file nor a directory: {}",
            canonical.display()
        )))
    }
}

/// Reject `candidate` when it contains, or is contained by, any entry
/// already in `entries` (equality is handled by the caller's
/// update-not-duplicate branch before this runs).
fn validate_no_overlap(entries: &[SourceEntry], candidate: &Path) -> Result<()> {
    for e in entries {
        if candidate.starts_with(&e.canonical_path) || e.canonical_path.starts_with(candidate) {
            return Err(Error::Usage(format!(
                "source path {} overlaps already-registered source {} ({})",
                candidate.display(),
                e.id,
                e.canonical_path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/registry.rs"]
mod tests;
