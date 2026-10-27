//! Data-directory layout: every on-disk location comemory reads or writes
//! is resolved relative to one root (default `~/.comemory`, override via
//! `COMEMORY_DATA_DIR`).

use std::path::{Path, PathBuf};

use crate::prelude::*;

/// Data-directory layout rooted at one directory.
#[derive(Debug, Clone)]
pub struct Paths {
    data_dir: PathBuf,
}

/// Resolve the effective data directory. Caller passes the CLI flag (which
/// also reads `COMEMORY_DATA_DIR`); on `None` we fall back to `$HOME/.comemory`.
pub fn resolve_data_dir(opt: Option<PathBuf>) -> PathBuf {
    opt.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".comemory")
    })
}

/// One `data_dir.join(segment)` getter per row. The method name stays the
/// typed surface callers use; the segment string is the only thing that
/// varies, so a table replaces sixteen near-identical bodies without a
/// lying parameter.
macro_rules! path_joins {
    ($(
        $(#[$meta:meta])*
        $name:ident => $segment:literal
    ),+ $(,)?) => {
        $(
            $(#[$meta])*
            pub fn $name(&self) -> PathBuf {
                self.data_dir.join($segment)
            }
        )+
    };
}

impl Paths {
    /// Root the layout at `data_dir`.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// The root data directory itself.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    path_joins! {
        /// Directory holding `{id}-{slug}.md` memory files.
        memories_dir => "memories",
        /// Legacy index directory (retained for the on-disk layout).
        index_dir => "index",
        /// Single-file SQLite mirror for v0.2 (`comemory.db`). Rooted directly at
        /// the data dir so callers that override `COMEMORY_DATA_DIR` see a
        /// predictable path next to `memories/`.
        db_path => "comemory.db",
        /// Layered config-file overlay (`config.toml`) applied on top of the
        /// shipped defaults.
        config_file => "config.toml",
        /// Device-login credentials written by `comemory auth login` (`auth.json`,
        /// mode `0600` on unix).
        auth_file => "auth.json",
        /// Cached org-repo allowlist left behind by releases before organization
        /// scoping. Nothing reads it any more; `comemory auth login` and
        /// `comemory auth logout` delete it so a stale copy cannot outlive the
        /// credential it was fetched for.
        allowlist_file => "allowlist.json",
        /// Durable source registry file (`src/domains/documents/source/registry.rs`), the
        /// authoritative record of registered document roots.
        sources_file => "sources.toml",
        /// Sibling exclusive-flock file guarding `sources.toml`
        /// read-modify-write cycles (`src/utilities/file_lock.rs`).
        sources_lock_file => "sources.toml.lock",
        /// Sibling exclusive-flock file guarding the migration preflight's
        /// `VACUUM INTO` snapshot, since SQLite will not serialize that
        /// statement itself (`src/store/migrate/preflight.rs`, generalized
        /// lock in `src/utilities/file_lock.rs`).
        migration_lock_file => "comemory.db.migrate.lock",
        /// Pre-swap snapshot of `comemory.db`, written by `comemory rebuild`
        /// (`src/domains/maintenance/rebuild.rs`) immediately before the rebuilt database
        /// replaces the live one.
        rebuild_backup => "comemory.db.pre-rebuild.bak",
    }

    /// Soft-delete directory under `memories/`.
    pub fn trash_dir(&self) -> PathBuf {
        self.memories_dir().join(".trash")
    }

    /// Path to the unified SQLite database.
    ///
    /// Previously this returned `stats.db`; v0.2 consolidates all tables
    /// into `comemory.db` (spec §4: one file). The method is kept so callers
    /// need no churn — they still pass the path to [`StatsDb::open`](crate::domains::learning::telemetry::StatsDb::open) which
    /// now delegates to [`crate::store::connection::open`].
    pub fn stats_db(&self) -> PathBuf {
        self.db_path()
    }

    /// Create `memories/`, `trash/`, and `index/` if missing.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.memories_dir(), self.trash_dir(), self.index_dir()] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/paths.rs"]
mod tests;
