use std::path::{Path, PathBuf};

use crate::prelude::*;

/// Data-directory layout: every on-disk location comemory reads or writes
/// is resolved relative to one root (default `~/.comemory`, override via
/// `COMEMORY_DATA_DIR`).
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

    /// Directory holding `{id}-{slug}.md` memory files.
    pub fn memories_dir(&self) -> PathBuf {
        self.data_dir.join("memories")
    }

    /// Soft-delete directory under `memories/`.
    pub fn trash_dir(&self) -> PathBuf {
        self.memories_dir().join(".trash")
    }

    /// Legacy index directory (retained for the on-disk layout).
    pub fn index_dir(&self) -> PathBuf {
        self.data_dir.join("index")
    }

    /// Path to the unified SQLite database.
    ///
    /// Previously this returned `stats.db`; v0.2 consolidates all tables
    /// into `comemory.db` (spec §4: one file). The method is kept so callers
    /// need no churn — they still pass the path to [`StatsDb::open`] which
    /// now delegates to [`crate::store::connection::open`].
    pub fn stats_db(&self) -> PathBuf {
        self.db_path()
    }

    /// Single-file SQLite mirror for v0.2 (`comemory.db`). Rooted directly at
    /// the data dir so callers that override `COMEMORY_DATA_DIR` see a
    /// predictable path next to `memories/`.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("comemory.db")
    }

    /// Layered config-file overlay (`config.toml`) applied on top of the
    /// shipped defaults.
    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.toml")
    }

    /// Durable source registry file (`src/source/registry.rs`), the
    /// authoritative record of registered document roots.
    pub fn sources_file(&self) -> PathBuf {
        self.data_dir.join("sources.toml")
    }

    /// Sibling exclusive-flock file guarding `sources.toml`
    /// read-modify-write cycles (`src/source/lock.rs`).
    pub fn sources_lock_file(&self) -> PathBuf {
        self.data_dir.join("sources.toml.lock")
    }

    /// Pre-migration snapshot of `comemory.db`, written by the migration
    /// preflight (`src/store/migrate/preflight.rs`) just before a
    /// destructive upgrade runs. `from` is the version being left, so the
    /// filename names the state the snapshot preserves. Rooted at
    /// `data_dir` directly, matching `db_path()` — callers migrating a
    /// database outside the data dir (e.g. `connection::open` on
    /// `comemory.db.rebuild.tmp`) derive their own backup name from that
    /// file's stem instead of going through this method.
    pub fn migration_backup(&self, from: u32) -> PathBuf {
        self.data_dir.join(format!("comemory.db.pre-v{from}.bak"))
    }

    /// Sibling exclusive-flock file guarding the migration preflight's
    /// `VACUUM INTO` snapshot, since SQLite will not serialize that
    /// statement itself (`src/store/migrate/preflight.rs`, generalized
    /// lock in `src/source/lock.rs`).
    pub fn migration_lock_file(&self) -> PathBuf {
        self.data_dir.join("comemory.db.migrate.lock")
    }

    /// Pre-swap snapshot of `comemory.db`, written by `comemory rebuild`
    /// (`src/api/rebuild.rs`) immediately before the rebuilt database
    /// replaces the live one.
    pub fn rebuild_backup(&self) -> PathBuf {
        self.data_dir.join("comemory.db.pre-rebuild.bak")
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
