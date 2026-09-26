//! `replica::identity` — the stream identity (#256, B-4), kept in
//! `<data_dir>/replica/` outside the database, so a database replaced by a
//! restore or a hand-copied `.bak` is checked against it, not trusted.
//!
//! The database agrees when it carries the same epoch and device id and
//! reflects at least the manifest lines the identity counts
//! ([`ERASURES_KEY`], stamped by every erase). Otherwise [`ensure`] mints a
//! new epoch and merges the manifest back in.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::erasure_manifest::{self, Entry};
use crate::config::{Config, Paths};
use crate::domains::memories::save_lock;
use crate::prelude::*;
use crate::store::{Connection, replica_device, replica_journal, replica_redaction, schema_meta};
use crate::utilities::file_lock::FileLock;

/// The mismatch repair: a new epoch and the manifest merged — its own file
/// for the size ceiling, a flat sibling like `maintenance::erase`'s halves.
#[path = "identity_repair.rs"]
mod repair;

/// Directory under the data directory holding the identity, the manifest
/// and their lock.
pub const DIR: &str = "replica";
/// The identity file under [`DIR`].
pub const IDENTITY_FILE: &str = "identity.json";
/// The erasure manifest under [`DIR`].
pub const MANIFEST_FILE: &str = "erasures.jsonl";
/// The lock every write to either file holds.
pub const LOCK_FILE: &str = "identity.lock";
/// `schema_meta` key: how many manifest lines this database reflects. A
/// `replica_` key, so `comemory rebuild` keeps it.
pub const ERASURES_KEY: &str = "replica_erasures";
/// Epoch reason: the identity was first written from the database's epoch.
pub const CREATED: &str = "created";
/// Epoch reason: the database was found replaced by something other than a
/// supported restore, and `ensure` re-epoched it.
pub const REPLACED: &str = "replaced";

/// `identity.json`'s format version.
const VERSION: u32 = 1;

/// `name` under `paths`' [`DIR`].
#[must_use]
pub fn file(paths: &Paths, name: &str) -> PathBuf {
    paths.data_dir().join(DIR).join(name)
}

/// `identity.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Format version.
    pub v: u32,
    /// The current stream epoch.
    pub epoch: String,
    /// This device's id.
    pub device_id: String,
    /// Every epoch this data directory has served, oldest first.
    pub epochs: Vec<Epoch>,
    /// How many lines the erasure manifest holds.
    pub erasures: u64,
}

/// One epoch in [`Identity::epochs`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Epoch {
    /// The epoch.
    pub epoch: String,
    /// RFC3339 time it took effect here.
    pub since: String,
    /// [`CREATED`] or [`REPLACED`].
    pub reason: String,
}

/// Proof of holding `identity.lock`; released on drop.
#[derive(Debug)]
pub struct IdentityGuard {
    _lock: FileLock,
}

/// Take `identity.lock`, waiting up to `wait`.
///
/// # Errors
/// [`Error::Busy`] once `wait` elapses with the lock still held; filesystem
/// failures creating [`DIR`] or the lock file.
pub fn lock(paths: &Paths, wait: std::time::Duration) -> Result<IdentityGuard> {
    std::fs::create_dir_all(paths.data_dir().join(DIR))?;
    FileLock::acquire_within(&file(paths, LOCK_FILE), "identity", wait)?
        .map(|lock| IdentityGuard { _lock: lock })
        .ok_or_else(|| Error::Busy(LOCK_FILE.to_string()))
}

/// What [`ensure`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ensured {
    /// The database is the stream `identity.json` names.
    Current,
    /// There was no identity: both files were written from the database.
    Created,
    /// The database was replaced: a new epoch, and the established manifest
    /// merged (`merged` lines).
    Reepoched {
        /// The new epoch.
        epoch: String,
        /// Manifest lines merged.
        merged: usize,
    },
    /// The database was replaced and the manifest is not established
    /// (missing, torn, edited or short): a new epoch, and only the `merged`
    /// lines that verify merged — what else was erased is unknown.
    ErasureUnknown {
        /// The new epoch.
        epoch: String,
        /// Manifest lines merged.
        merged: usize,
    },
}

/// Check the database behind `conn` against `paths`' identity, and repair a
/// mismatch: the fast path holds `identity.lock` alone; a mismatch takes
/// `memory-save.lock` and then `identity.lock` (the lock order), checks
/// again, and re-epochs and merges in place. Each wait is bounded by
/// `[sync] pause_wait`.
///
/// # Errors
/// [`Error::Busy`] when a lock is not granted in time; SQLite, filesystem and
/// JSON failures — including an unreadable `identity.json`, which is never
/// guessed at.
pub fn ensure(paths: &Paths, cfg: &Config, conn: &mut Connection) -> Result<Ensured> {
    let wait = cfg.sync.pause_wait_duration()?;
    {
        let held = lock(paths, wait)?;
        let (identity, created) = establish(&held, paths, conn)?;
        if created {
            return Ok(Ensured::Created);
        }
        if Stamped::read(conn)?.agrees(&identity) {
            return Ok(Ensured::Current);
        }
    }
    let save = save_lock::acquire_within(paths, wait)?;
    let held = lock(paths, wait)?;
    let (identity, _) = establish(&held, paths, conn)?;
    if Stamped::read(conn)?.agrees(&identity) {
        return Ok(Ensured::Current);
    }
    repair::run(&save, &held, paths, conn, identity)
}

/// `paths`' identity, writing both files from the database when there is
/// none yet; `true` when it was just written. A manifest found without an
/// identity (a crash between the two writes, or an identity removed by hand)
/// is kept, never rewritten: the identity counts what it holds.
///
/// # Errors
/// SQLite, filesystem and JSON failures.
pub fn establish(
    held: &IdentityGuard,
    paths: &Paths,
    conn: &Connection,
) -> Result<(Identity, bool)> {
    if let Some(identity) = read(paths)? {
        return Ok((identity, false));
    }
    let stamped = Stamped::read(conn)?;
    let manifest_path = file(paths, MANIFEST_FILE);
    let erasures = if let Some(kept) = erasure_manifest::read(&manifest_path)? {
        count(kept.lines.len()).max(stamped.erasures)
    } else {
        manifest_from_database(held, &manifest_path, conn, stamped.erasures)?
    };
    let identity = Identity {
        v: VERSION,
        epochs: vec![Epoch {
            epoch: stamped.epoch.clone(),
            since: now()?,
            reason: CREATED.to_string(),
        }],
        epoch: stamped.epoch,
        device_id: stamped.device_id,
        erasures,
    };
    write(held, paths, &identity)?;
    Ok((identity, true))
}

/// Write the manifest from the entities whose payloads `conn` has erased,
/// stamping their count on the database first — a crash before the file
/// leaves both absent, and the next read writes them again from the same
/// rows. Returns the count.
fn manifest_from_database(
    held: &IdentityGuard,
    path: &Path,
    conn: &Connection,
    stamped: u64,
) -> Result<u64> {
    let entities = replica_redaction::erased_entities(conn)?;
    let erasures = count(entities.len());
    if erasures != stamped {
        stamp(conn, erasures)?;
    }
    let entries: Vec<Entry<'_>> = entities
        .iter()
        .map(|e| Entry {
            kind: &e.kind,
            key: &e.key,
            digests: &e.digests,
            erased_at: &e.erased_at,
        })
        .collect();
    erasure_manifest::create(held, path, &entries)?;
    Ok(erasures)
}

/// Record one erase in its own transaction `tx`, before it commits: append
/// the entity's line (fsynced), advance `identity.json`'s count, and stamp
/// the count on the database. A crash after the append leaves the manifest a
/// line ahead, which only means one more entity is erased on the next merge.
///
/// # Errors
/// [`Error::Other`] when no identity was established under this lock;
/// filesystem, JSON and SQLite failures.
pub fn record(
    held: &IdentityGuard,
    paths: &Paths,
    tx: &Connection,
    entry: &Entry<'_>,
) -> Result<()> {
    let mut identity = read(paths)?.ok_or_else(|| {
        Error::Other(format!(
            "{IDENTITY_FILE} was not established before the erase"
        ))
    })?;
    erasure_manifest::append(held, &file(paths, MANIFEST_FILE), entry)?;
    identity.erasures += 1;
    write(held, paths, &identity)?;
    stamp(tx, identity.erasures)
}

/// `paths`' `identity.json`; `None` when there is none.
///
/// # Errors
/// Filesystem failures other than a missing file, and a file that does not
/// parse.
pub fn read(paths: &Paths) -> Result<Option<Identity>> {
    let path = file(paths, IDENTITY_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::Other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Replace `identity.json` durably.
fn write(_held: &IdentityGuard, paths: &Paths, identity: &Identity) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(identity)?;
    bytes.push(b'\n');
    write_durable(&file(paths, IDENTITY_FILE), &bytes)
}

/// What the database itself carries of its identity.
struct Stamped {
    epoch: String,
    device_id: String,
    erasures: u64,
}

impl Stamped {
    /// Read it from `conn`. A count that does not parse reads as `0`, which
    /// can only send the database to the repair that stamps it afresh.
    fn read(conn: &Connection) -> Result<Self> {
        let erasures = schema_meta::get(conn, ERASURES_KEY)?.map_or(0, |raw| {
            raw.parse().unwrap_or_else(|e| {
                tracing::warn!(key = ERASURES_KEY, value = %raw, error = %e, "unreadable erasure count; checking the database again");
                0
            })
        });
        Ok(Self {
            epoch: replica_journal::stream_epoch(conn)?,
            device_id: replica_device::id(conn)?,
            erasures,
        })
    }

    /// Whether this database is the stream `identity` names.
    fn agrees(&self, identity: &Identity) -> bool {
        self.epoch == identity.epoch
            && self.device_id == identity.device_id
            && self.erasures >= identity.erasures
    }
}

/// Stamp `erasures` on the database.
fn stamp(conn: &Connection, erasures: u64) -> Result<()> {
    schema_meta::upsert(conn, ERASURES_KEY, &erasures.to_string())
}

/// A line count as the identity stores it.
fn count(lines: usize) -> u64 {
    u64::try_from(lines).unwrap_or(u64::MAX)
}

/// Now, as every replica timestamp is written.
fn now() -> Result<String> {
    crate::store::memory_row::iso_format(time::OffsetDateTime::now_utc())
}

/// Write `bytes` to `path` durably: a temporary file beside it, fsynced, then
/// renamed into place and the directory fsynced, so a crash leaves the old
/// file or the new one and never a torn one.
///
/// # Errors
/// Filesystem failures.
pub(super) fn write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut staged = File::create(&tmp)?;
    staged.write_all(bytes)?;
    staged.sync_all()?;
    std::fs::rename(&tmp, path)?;
    sync_dir(path)
}

/// Fsync the directory holding `path`, which is what makes a rename or a
/// new file's name durable.
///
/// # Errors
/// Filesystem failures.
pub(super) fn sync_dir(path: &Path) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::Other(format!("{} has no directory", path.display())))?;
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/identity.rs"]
mod tests;
