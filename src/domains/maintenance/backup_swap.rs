//! A restore's swap (#256, B-4) and `<data_dir>/restore.pending`, the record
//! that makes it resumable.
//!
//! The record is written before anything is staged and names the phase. A
//! restore killed while `staging` is started over by its rerun; one killed
//! while `swapping` is finished: every step below is idempotent — the
//! markdown moves check what already moved, the in-place copy runs again
//! from the staged file until that file is removed, and the record goes
//! last. While it exists every sync surface refuses
//! (`replica::restore_state`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::DB_FILE;
use crate::config::Paths;
use crate::domains::maintenance::rebuild;
use crate::domains::sync::replica::identity::{sync_dir, write_durable};
use crate::domains::sync::replica::restore_state;
use crate::prelude::*;
use crate::store::replace_in_place;

/// Where a restore stages and what it keeps, under one data directory.
#[derive(Debug, Clone)]
pub(super) struct Layout {
    /// `comemory.db`.
    pub live_db: PathBuf,
    /// `comemory.db.restore.tmp`: the snapshot, migrated and merged.
    pub staged_db: PathBuf,
    /// `memories/`.
    pub live_memories: PathBuf,
    /// `memories.restore/`: the snapshot's markdown, merged.
    pub staged_memories: PathBuf,
    /// `comemory.db.pre-restore.bak`: the database the last restore replaced.
    pub pre_db: PathBuf,
    /// `memories.pre-restore/`: the markdown the last restore replaced.
    pub pre_memories: PathBuf,
    /// `restore.pending`.
    pub pending: PathBuf,
}

impl Layout {
    /// The layout under `paths`' data directory.
    pub fn of(paths: &Paths) -> Self {
        let dir = paths.data_dir();
        Self {
            live_db: paths.db_path(),
            staged_db: dir.join(format!("{DB_FILE}.restore.tmp")),
            live_memories: paths.memories_dir(),
            staged_memories: dir.join("memories.restore"),
            pre_db: dir.join(format!("{DB_FILE}.pre-restore.bak")),
            pre_memories: dir.join("memories.pre-restore"),
            pending: restore_state::pending_path(paths),
        }
    }
}

/// Where a restore stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    /// Copying, migrating and merging the snapshot beside the live files.
    Staging,
    /// Moving the markdown and copying the staged database in place.
    Swapping,
}

/// `restore.pending`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Pending {
    /// The backup directory being restored.
    pub source: PathBuf,
    /// The staged database.
    pub staged_db: PathBuf,
    /// The staged markdown tree.
    pub staged_memories: PathBuf,
    /// Where the restore stands.
    pub phase: Phase,
    /// Manifest lines the staging merged.
    pub erasures_merged: usize,
}

impl Pending {
    /// The record at `path`; `None` when no restore is pending.
    pub fn read(path: &Path) -> Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| Error::Other(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write the record durably at `path`.
    pub fn write(&self, path: &Path) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        write_durable(path, &bytes)
    }
}

/// Finish the swap `layout` stages: the markdown first, then the database
/// in place, then the record. `wait` bounds the wait for the live file's
/// write lock.
///
/// # Errors
/// [`Error::Busy`] when another writer holds the live database past `wait`;
/// [`Error::Conflict`] when a writer recreated `memories/` with files after
/// it was moved aside; SQLite and filesystem failures. The record stays, so
/// a rerun finishes.
pub(super) fn finish(layout: &Layout, wait: Duration) -> Result<()> {
    move_memories(layout)?;
    if layout.staged_db.try_exists()? {
        if layout.live_db.try_exists()? {
            let mut live = replace_in_place::open_destination(&layout.live_db, wait)?;
            replace_in_place::replace_in_place(&mut live, &layout.staged_db)?;
        } else {
            // Nothing holds a file that does not exist: copy it through
            // SQLite (so the staged WAL is read) and rename it into place.
            rebuild::snapshot_before_swap_inner(&layout.staged_db, &layout.live_db)?;
        }
        rebuild::remove_db_and_sidecars(&layout.staged_db);
    }
    std::fs::remove_file(&layout.pending)?;
    sync_dir(&layout.pending)
}

/// Move `memories/` to `memories.pre-restore/` and `memories.restore/` into
/// its place, skipping whatever a killed run already moved. The previous
/// restore's `memories.pre-restore/` is removed before the phase turns
/// `swapping`, so one found now is this restore's.
fn move_memories(layout: &Layout) -> Result<()> {
    if !layout.staged_memories.try_exists()? {
        return Ok(());
    }
    if layout.live_memories.try_exists()? {
        if !layout.pre_memories.try_exists()? {
            std::fs::rename(&layout.live_memories, &layout.pre_memories)?;
        } else if holds_no_file(&layout.live_memories)? {
            // Recreated empty (`memories/.trash/`) after the first move.
            std::fs::remove_dir_all(&layout.live_memories)?;
        } else {
            return Err(Error::Conflict(format!(
                "{} was recreated and written after the restore moved the previous tree to {}; \
                 move it aside and rerun the restore",
                layout.live_memories.display(),
                layout.pre_memories.display()
            )));
        }
    }
    std::fs::rename(&layout.staged_memories, &layout.live_memories)?;
    sync_dir(&layout.live_memories)
}

/// Whether the tree at `dir` holds no file at any depth.
fn holds_no_file(dir: &Path) -> Result<bool> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !holds_no_file(&entry.path())? {
            return Ok(false);
        }
    }
    Ok(true)
}
