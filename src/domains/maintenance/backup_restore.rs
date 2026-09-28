//! `comemory backup restore DIR --confirm [--erasure-manifest FILE]`
//! (#256, B-4): install a backup under a new stream epoch, with every erase
//! the backup predates merged back in before anything is served.
//!
//! With the exchange paused and under `memory-save.lock` for the whole run:
//! record `restore.pending`; copy the snapshot beside the live files and open
//! it (an older one migrates forward); mark it `merging`, mint an epoch into
//! `identity.json` (reason `restore`) and give it the identity's device id;
//! merge the manifest, or mark it `erasure_unknown` — local-only — when none
//! is established; snapshot what it replaces; then swap ([`super::swap`]). A
//! rerun finishes a swap it finds pending, and starts over a staging it finds
//! pending.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::swap::{self, Layout, Pending, Phase};
use super::{DB_FILE, MEMORIES_DIR, RestoreRequest as Request, Restored};
use crate::config::{Config, Paths};
use crate::domains::maintenance::rebuild;
use crate::domains::memories::save_lock::{self, SaveGuard};
use crate::domains::sync::exchange_gate;
use crate::domains::sync::replica::erasure_manifest;
use crate::domains::sync::replica::identity::{self, IdentityGuard, MANIFEST_FILE, RESTORED};
use crate::domains::sync::replica::restore_state::{self, State};
use crate::prelude::*;
use crate::store::replica_journal;
use crate::store::{Connection, connection, random_id, replace_in_place, replica_device};

/// The restore behind [`super::restore`].
pub(super) fn run(paths: &Paths, cfg: &Config, req: &Request) -> Result<Restored> {
    let layout = Layout::of(paths);
    let wait = cfg.sync.pause_wait_duration()?;
    std::fs::create_dir_all(paths.data_dir())?;
    let _pause = exchange_gate::pause(paths, wait)?;
    let guard = save_lock::acquire_within(paths, wait)?;
    if let Some(pending) = Pending::read(&layout.pending)?
        && pending.phase == Phase::Swapping
    {
        if !same_dir(&pending.source, &req.dir)? {
            return Err(Error::Conflict(format!(
                "a restore of {} is mid-swap; rerun `comemory backup restore {} --confirm` to \
                 finish it first",
                pending.source.display(),
                pending.source.display()
            )));
        }
        swap::finish(&layout, wait)?;
        return report(&layout, &pending, true);
    }
    let source = source_of(&req.dir)?;
    refuse_page_size(&source.join(DB_FILE), &layout.live_db)?;
    let mut pending = Pending {
        source,
        staged_db: layout.staged_db.clone(),
        staged_memories: layout.staged_memories.clone(),
        phase: Phase::Staging,
        erasures_merged: 0,
    };
    discard_staged(&layout)?;
    pending.write(&layout.pending)?;
    let manifest = req.erasure_manifest.as_deref();
    match prepare(paths, &guard, &layout, (&pending.source, manifest), wait) {
        Ok(merged) => pending.erasures_merged = merged,
        Err(e) => {
            discard_staged(&layout)?;
            std::fs::remove_file(&layout.pending)?;
            return Err(e);
        }
    }
    pending.phase = Phase::Swapping;
    pending.write(&layout.pending)?;
    swap::finish(&layout, wait)?;
    report(&layout, &pending, false)
}

/// Stage and merge the snapshot, then keep what the swap will replace: the
/// live database snapshotted to `comemory.db.pre-restore.bak` (a failed
/// snapshot never touches the previous one) and the previous restore's
/// `memories.pre-restore/` removed. Returns the manifest lines merged.
fn prepare(
    paths: &Paths,
    guard: &SaveGuard,
    layout: &Layout,
    snapshot: (&Path, Option<&Path>),
    wait: Duration,
) -> Result<usize> {
    let merged = stage(paths, guard, layout, snapshot, wait)?;
    if layout.live_db.try_exists()? {
        rebuild::snapshot_before_swap_inner(&layout.live_db, &layout.pre_db)?;
    }
    if layout.pre_memories.try_exists()? {
        std::fs::remove_dir_all(&layout.pre_memories)?;
    }
    Ok(merged)
}

/// Steps 2–4: copy the snapshot beside the live files and open it, so an
/// older one migrates forward; mark it `merging`; mint the new epoch into
/// `identity.json` and the identity into the staged database; merge the
/// manifest, then clear the state — or set `erasure_unknown` when the
/// manifest is not established, merging only the lines that verify.
fn stage(
    paths: &Paths,
    guard: &SaveGuard,
    layout: &Layout,
    (source, manifest): (&Path, Option<&Path>),
    wait: Duration,
) -> Result<usize> {
    for suffix in ["", "-wal"] {
        let from = suffixed(&source.join(DB_FILE), suffix);
        if from.try_exists()? {
            std::fs::copy(&from, suffixed(&layout.staged_db, suffix))?;
        }
    }
    super::copy_tree(&source.join(MEMORIES_DIR), &layout.staged_memories)?;
    let mut staged = connection::open(&layout.staged_db)?;
    let held = identity::lock(paths, wait)?;
    let (mut identity, _) = establish(&held, paths, layout, &staged)?;
    restore_state::set(&staged, State::Merging)?;
    let default = identity::file(paths, MANIFEST_FILE);
    let found = erasure_manifest::read(manifest.unwrap_or(&default))?;
    let established = found
        .as_ref()
        .is_some_and(|m| m.established(identity.erasures));
    let lines = found.map(|m| m.lines).unwrap_or_default();
    let erasures = identity.erasures.max(identity::count(lines.len()));
    let epoch = random_id::random_hex(16)?;
    let at = crate::store::memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    identity::rotate(
        &held,
        paths,
        &mut identity,
        (&epoch, RESTORED, &at),
        erasures,
    )?;
    replica_journal::set_identity(&staged, &epoch, &identity.device_id, &at)?;
    super::merge::lines_into(guard, &layout.staged_memories, &mut staged, &lines)?;
    identity::stamp(&staged, erasures)?;
    if established {
        restore_state::clear(&staged)?;
    } else {
        tracing::warn!(
            merged = lines.len(),
            "no established erasure manifest: the restore is local-only until `comemory backup merge-erasures`"
        );
        restore_state::set(&staged, State::ErasureUnknown)?;
    }
    Ok(lines.len())
}

/// The identity to restore under: `identity.json`, or — the first time —
/// one written from the live database, or from the snapshot on a data
/// directory that has none.
fn establish(
    held: &IdentityGuard,
    paths: &Paths,
    layout: &Layout,
    staged: &Connection,
) -> Result<(identity::Identity, bool)> {
    if layout.live_db.try_exists()? {
        let live = connection::open(&layout.live_db)?;
        return identity::establish(held, paths, &live);
    }
    identity::establish(held, paths, staged)
}

/// Refuse a snapshot whose page size differs from the live database's: the
/// in-place copy cannot change a WAL file's page size.
fn refuse_page_size(snapshot: &Path, live: &Path) -> Result<()> {
    if !live.try_exists()? {
        return Ok(());
    }
    let (from, to) = (
        replace_in_place::page_size(snapshot)?,
        replace_in_place::page_size(live)?,
    );
    if from == to {
        return Ok(());
    }
    Err(Error::Unsupported(format!(
        "cannot restore {}: it uses {from}-byte pages and the live database {} uses \
         {to}-byte pages",
        snapshot.display(),
        live.display()
    )))
}

/// The backup directory `dir`, absolute; it must hold a database.
fn source_of(dir: &Path) -> Result<PathBuf> {
    if !dir.join(DB_FILE).is_file() {
        return Err(Error::Usage(format!(
            "{} holds no {DB_FILE} backup",
            dir.display()
        )));
    }
    Ok(std::fs::canonicalize(dir)?)
}

/// Whether `dir` names the pending restore's `source` — a directory since
/// removed can only match by its spelling.
fn same_dir(source: &Path, dir: &Path) -> Result<bool> {
    match std::fs::canonicalize(dir) {
        Ok(found) => Ok(found == source),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(dir == source),
        Err(e) => Err(e.into()),
    }
}

/// Remove a staging an earlier attempt left: the staged database and its
/// sidecars, and `memories.restore/`.
fn discard_staged(layout: &Layout) -> Result<()> {
    rebuild::remove_db_and_sidecars(&layout.staged_db);
    match std::fs::remove_dir_all(&layout.staged_memories) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// `path` with `suffix` appended to its file name.
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(suffix);
    PathBuf::from(raw)
}

/// What the restored database carries, read back from the live file.
fn report(layout: &Layout, pending: &Pending, resumed: bool) -> Result<Restored> {
    let conn = connection::open(&layout.live_db)?;
    let mut pre_restore = Vec::new();
    for kept in [&layout.pre_db, &layout.pre_memories] {
        if kept.try_exists()? {
            pre_restore.push(kept.to_string_lossy().into_owned());
        }
    }
    Ok(Restored {
        source: pending.source.to_string_lossy().into_owned(),
        epoch: replica_journal::stream_epoch(&conn)?,
        device_id: replica_device::id(&conn)?,
        restore_state: restore_state::read(&conn)?,
        erasures_merged: pending.erasures_merged,
        resumed,
        pre_restore,
    })
}
