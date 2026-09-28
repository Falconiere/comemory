//! `maintenance::backup` — `comemory backup create|restore|merge-erasures`
//! (#256, B-4). CLI-only.
//!
//! [`create`] snapshots the database (`VACUUM INTO`), `memories/` and
//! `backup.json` with the exchange paused (`domains::sync::exchange_gate`)
//! and under `memory-save.lock`; [`restore`] installs one, under the same
//! two, with a new stream epoch and the erasure manifest merged in before it
//! is served; [`merge_erasures`] merges a manifest the restore could not
//! find. Every sync surface refuses while a restore is pending
//! (`replica::restore_state`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, Paths};
use crate::domains::memories::save_lock;
use crate::domains::sync::exchange_gate;
use crate::prelude::*;
use crate::store::migrate::{backup as snapshot, preflight};
use crate::store::{connection, replica_journal};

/// `comemory backup merge-erasures`, and the merge `restore` runs: the full
/// erase of every entity a manifest names, then every digest barred — a
/// flat sibling, since `domains/maintenance/` allows no new subfolder.
#[path = "backup_merge.rs"]
mod merge;

/// `comemory backup restore`: stage, merge and swap a snapshot in.
#[path = "backup_restore.rs"]
mod restore;

/// The restore's swap and the `restore.pending` record that makes it
/// resumable — a flat sibling, since `domains/maintenance/` allows no new
/// subfolder.
#[path = "backup_swap.rs"]
mod swap;

/// Directory under the data directory holding default backups.
pub const BACKUPS_DIR: &str = "backups";
/// The descriptor every backup directory carries, written last.
pub const DESCRIPTOR_FILE: &str = "backup.json";
/// The database copy inside a backup directory.
pub const DB_FILE: &str = "comemory.db";
/// The markdown copy inside a backup directory.
pub const MEMORIES_DIR: &str = "memories";

/// `backup.json`: what a backup directory holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    /// RFC3339 time the backup was taken.
    pub created_at: String,
    /// The comemory version that took it.
    pub binary_version: String,
    /// Every migration marker the copied database has applied.
    pub schema_markers: Vec<String>,
    /// The stream epoch the copied database carried.
    pub epoch: String,
}

/// `comemory backup create --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Created {
    /// The backup directory.
    pub dir: String,
    /// Markdown files copied (live and trashed).
    pub memory_files: u64,
    /// What `backup.json` records.
    #[serde(flatten)]
    pub descriptor: Descriptor,
}

/// `comemory backup restore` request.
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    /// The backup directory: `comemory.db` plus `memories/`.
    pub dir: PathBuf,
    /// The erasure manifest to merge, instead of
    /// `<data_dir>/replica/erasures.jsonl`.
    pub erasure_manifest: Option<PathBuf>,
}

/// `comemory backup restore --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Restored {
    /// The backup directory restored.
    pub source: String,
    /// The stream epoch the restored database now carries.
    pub epoch: String,
    /// The device id it carries: this data directory's.
    pub device_id: String,
    /// `erasure_unknown` when no established manifest could be merged: the
    /// restore is local-only until `backup merge-erasures`. `None` when sync
    /// is allowed.
    pub restore_state: Option<String>,
    /// Manifest lines merged into the snapshot.
    pub erasures_merged: usize,
    /// Whether this run finished a swap an interrupted run left pending.
    pub resumed: bool,
    /// What the restore replaced, kept beside the live files.
    pub pre_restore: Vec<String>,
}

/// `comemory backup merge-erasures --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Merged {
    /// Where the manifest now lives: `<data_dir>/replica/erasures.jsonl`.
    pub manifest: String,
    /// Manifest lines merged.
    pub lines: usize,
    /// Entries whose memory or document this database still held, erased.
    pub entities_erased: usize,
    /// The restore state this merge cleared; `None` when none was set.
    pub cleared: Option<String>,
}

/// `comemory backup restore`: install `req.dir` into `paths`' data directory
/// under a new epoch, with the erasure manifest merged in first.
///
/// # Errors
/// [`Error::Usage`] when `req.dir` holds no backup; [`Error::Unsupported`]
/// naming both files when its page size differs from the live database's;
/// [`Error::Conflict`] when a different restore is mid-swap; [`Error::Busy`]
/// when a lock is not granted within `[sync] pause_wait`; SQLite, filesystem
/// and migration failures. A failed staging leaves the live files untouched;
/// a failed swap leaves `restore.pending` for a rerun to finish.
pub fn restore(paths: &Paths, cfg: &Config, req: &RestoreRequest) -> Result<Restored> {
    restore::run(paths, cfg, req)
}

/// `comemory backup merge-erasures FILE`: when `file` is an established
/// manifest, put it in place, merge it into the live database and markdown,
/// stamp its count and clear the restore state.
///
/// # Errors
/// [`Error::Unavailable`] without a database; [`Error::Usage`] for a missing
/// `file`; [`Error::Conflict`] while a restore is still pending, or when
/// `file` is not established — nothing is changed; [`Error::Busy`] when a
/// lock is not granted within `[sync] pause_wait`; SQLite and filesystem
/// failures.
pub fn merge_erasures(paths: &Paths, cfg: &Config, file: &Path) -> Result<Merged> {
    merge::run(paths, cfg, file)
}

/// Back up `paths`' data directory to `out`, or to
/// `<data_dir>/backups/<UTC timestamp>/` when none is named.
///
/// # Errors
/// [`Error::Unavailable`] when there is no database to back up;
/// [`Error::Conflict`] when `out` already holds a backup; [`Error::Busy`]
/// when the exchange gate or `memory-save.lock` is not granted within
/// `[sync] pause_wait`;
/// SQLite and filesystem failures.
pub fn create(paths: &Paths, cfg: &Config, out: Option<PathBuf>) -> Result<Created> {
    let db = paths.db_path();
    if !db.try_exists()? {
        return Err(Error::Unavailable(format!(
            "no database to back up at {}",
            db.display()
        )));
    }
    let now = time::OffsetDateTime::now_utc();
    let dir = match out {
        Some(dir) => dir,
        None => paths.data_dir().join(BACKUPS_DIR).join(dir_name(now)?),
    };
    if dir.join(DB_FILE).try_exists()? || dir.join(DESCRIPTOR_FILE).try_exists()? {
        return Err(Error::Conflict(format!(
            "{} already holds a backup",
            dir.display()
        )));
    }
    let wait = cfg.sync.pause_wait_duration()?;
    let _pause = exchange_gate::pause(paths, wait)?;
    let _guard = save_lock::acquire_within(paths, wait)?;
    let conn = connection::open(&db)?;
    std::fs::create_dir_all(&dir)?;
    snapshot::snapshot(&conn, &dir.join(DB_FILE))?;
    let memory_files = copy_tree(&paths.memories_dir(), &dir.join(MEMORIES_DIR))?;
    let descriptor = Descriptor {
        created_at: crate::store::memory_row::iso_format(now)?,
        binary_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_markers: preflight::applied_keys(&conn)?.into_iter().collect(),
        epoch: replica_journal::stream_epoch(&conn)?,
    };
    let mut bytes = serde_json::to_vec_pretty(&descriptor)?;
    bytes.push(b'\n');
    crate::domains::sync::replica::identity::write_durable(&dir.join(DESCRIPTOR_FILE), &bytes)?;
    Ok(Created {
        dir: dir.to_string_lossy().into_owned(),
        memory_files,
        descriptor,
    })
}

/// A backup directory's name for `at`: sortable, and safe on every
/// filesystem (no `:`).
fn dir_name(at: time::OffsetDateTime) -> Result<String> {
    at.format(time::macros::format_description!(
        "[year][month][day]T[hour][minute][second].[subsecond digits:3]Z"
    ))
    .map_err(|e| Error::Other(format!("backup directory name: {e}")))
}

/// Copy the tree at `from` into `to`, creating `to`; returns the files
/// copied. A missing `from` copies nothing and still creates `to`, so a
/// backup or a restore of an engine with no markdown is a real empty tree.
///
/// # Errors
/// Filesystem failures.
pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    std::fs::create_dir_all(to)?;
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut copied = 0;
    for entry in entries {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
#[path = "tests/backup.rs"]
mod tests;
