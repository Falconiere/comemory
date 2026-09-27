//! `maintenance::erase` — permanently erase one memory or one document
//! (#256, B-5): `comemory erase` / `POST /api/v1/erase`.
//!
//! Unlike `delete` (reversible) or `gc`'s purge (still restorable on a peer),
//! an erase removes the entity's text from every table, journal copy and the
//! markdown tree, keeping feed rows, revisions, receipts and digests — the
//! barrier a later offer of the same bytes reads as `payload_erased`.
//! [`run`] takes `memory-save.lock`, then `identity.lock`, then calls the
//! lock-free [`apply_locked`] — which appends the erase to the erasure
//! manifest before it commits — and [`settle`]; a lock holder composes them
//! itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::domains::memories::save_lock::{self, SaveGuard};
use crate::domains::sync::replica::contract::Disposition;
use crate::domains::sync::replica::erasure_manifest::Entry;
use crate::domains::sync::replica::identity::{self, IdentityGuard};
use crate::prelude::*;
use crate::store::erase_rows::{self, FtsIndex};
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_outbox::{self, Outcome, Scope};
use crate::store::{Connection, Transaction, replica_redaction_copies};
use crate::utilities::context::Ctx;

/// The memory half of [`apply_locked`], its own file for the size ceiling —
/// a flat sibling, since `domains/maintenance/` allows no new subfolder.
#[path = "erase_memory.rs"]
mod memory;

/// The document half of [`apply_locked`] — same reasoning as [`memory`].
#[path = "erase_document.rs"]
mod document;

/// How long [`settle`] retries the truncating checkpoint while a reader
/// holds an open transaction.
const CHECKPOINT_WAIT: Duration = Duration::from_secs(5);

/// `comemory erase` / `POST /api/v1/erase` request: exactly one of the two.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Memory id to erase.
    #[serde(default)]
    pub memory: Option<String>,
    /// Shared document id (`document_share.shared_id`) to erase.
    #[serde(default)]
    pub document: Option<String>,
}

/// The one entity an erase reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A memory, by id.
    Memory(String),
    /// A document, local or pulled, by shared id.
    Document(String),
}

impl Target {
    /// `"memory"` or `"document"`, as the report names it.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Document(_) => "document",
        }
    }

    /// The id or shared id.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Memory(key) | Self::Document(key) => key,
        }
    }
}

impl TryFrom<Request> for Target {
    type Error = Error;

    fn try_from(req: Request) -> Result<Self> {
        match (req.memory, req.document) {
            (Some(id), None) if !id.trim().is_empty() => Ok(Self::Memory(id.trim().to_string())),
            (None, Some(id)) if !id.trim().is_empty() => Ok(Self::Document(id.trim().to_string())),
            _ => Err(Error::BadRequest(
                "name exactly one of `memory` or `document`".to_string(),
            )),
        }
    }
}

/// What one committed erase did, before its post-commit steps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// A tombstone was journalled — the entity was live and shared here.
    pub tombstoned: bool,
    /// Every payload digest blanked with `redaction = erased`.
    pub digests: Vec<String>,
    /// Pending upserts and restores marked `rejected` / `payload_erased`.
    pub operations_withdrawn: u64,
    /// Staged parts of complete sets hashing to an erased digest.
    pub staged_removed: u64,
    /// Replay scratch rows naming an erased digest.
    pub replay_blanked: u64,
    /// The FTS indexes the erase deleted postings from.
    pub touched: Vec<FtsIndex>,
}

/// `comemory erase --json` / `POST /api/v1/erase` response.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// `memory` or `document`.
    pub kind: &'static str,
    /// The id or shared id erased.
    pub key: String,
    /// Whether a tombstone was journalled first.
    pub tombstoned: bool,
    /// Payloads whose bytes were blanked.
    pub payloads_erased: usize,
    /// Pending upserts and restores withdrawn.
    pub operations_withdrawn: u64,
    /// Staged parts removed.
    pub staged_removed: u64,
    /// Replay scratch rows cleared.
    pub replay_blanked: u64,
    /// The WAL was truncated after the commit; `false` when a reader held it
    /// past the retry window — the frames go at the next checkpoint.
    pub wal_truncated: bool,
    /// Rollback snapshots in the data directory that still hold pre-erase
    /// state. Deleting them is the operator's decision.
    pub snapshots_with_prior_state: Vec<String>,
}

/// Where a committed erase records itself: the held `identity.lock` and the
/// data directory whose erasure manifest it appends to (#256, B-4).
#[derive(Debug, Clone, Copy)]
pub struct Ledger<'a> {
    /// Proof `identity.lock` is held.
    pub held: &'a IdentityGuard,
    /// The data directory holding the manifest.
    pub paths: &'a Paths,
}

/// Erase `req`'s entity under `memory-save.lock` and then `identity.lock`,
/// each bounded by `[sync] pause_wait`.
///
/// # Errors
/// [`Error::BadRequest`] unless exactly one entity is named;
/// [`Error::NotFound`] for an entity this engine never held (nothing is
/// erased or recorded); [`Error::Busy`] when a lock is not granted in time;
/// SQLite and filesystem failures.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Report> {
    let target = Target::try_from(req)?;
    // A data directory with no database holds nothing to erase, and opening
    // one here would create it — an erase of nothing must write nothing.
    if !ctx.paths.db_path().exists() {
        return Err(Error::NotFound(format!(
            "{} {}",
            target.kind(),
            target.key()
        )));
    }
    let wait = ctx.cfg.sync.pause_wait_duration()?;
    let guard = save_lock::acquire_within(ctx.paths, wait)?;
    let held = identity::lock(ctx.paths, wait)?;
    let paths = ctx.paths;
    let memories_dir = paths.memories_dir();
    let conn = ctx.conn()?;
    // Before the erase's transaction: the first identity is written from the
    // database's erasures as they stood, and this erase then appends its own.
    identity::establish(&held, paths, conn)?;
    let ledger = Ledger { held: &held, paths };
    let applied = apply_locked(&guard, Some(ledger), &memories_dir, conn, &target)?;
    let wal_truncated = settle(conn, &applied.touched)?;
    drop(held);
    drop(guard);
    Ok(Report {
        kind: target.kind(),
        key: target.key().to_string(),
        tombstoned: applied.tombstoned,
        payloads_erased: applied.digests.len(),
        operations_withdrawn: applied.operations_withdrawn,
        staged_removed: applied.staged_removed,
        replay_blanked: applied.replay_blanked,
        wal_truncated,
        snapshots_with_prior_state: snapshots_with_prior_state(ctx.paths.data_dir())?,
    })
}

/// The core, which takes no lock itself: erase `target` from `conn` and from the markdown tree
/// under `memories_dir`, in one transaction with `secure_delete` on. The
/// guard is the proof the caller holds `memory-save.lock`, so no markdown
/// writer moves the files this deletes. With a `ledger`, the erase is
/// appended to the erasure manifest before the transaction commits; a caller
/// replaying the manifest itself passes `None`.
///
/// # Errors
/// [`Error::NotFound`] when neither the markdown, the mirror nor the journal
/// holds `target` — nothing is written; SQLite and filesystem failures.
pub fn apply_locked(
    guard: &SaveGuard,
    ledger: Option<Ledger<'_>>,
    memories_dir: &Path,
    conn: &mut Connection,
    target: &Target,
) -> Result<Applied> {
    let at = crate::store::memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    erase_rows::with_secure_delete(conn, |conn| match target {
        Target::Memory(id) => memory::erase(guard, ledger, memories_dir, conn, id, &at),
        Target::Document(shared_id) => document::erase(ledger, conn, shared_id, &at),
    })
}

/// Record the erase of `(kind, key)` in `ledger`'s manifest, when there is
/// one, then commit `tx` — the manifest line is on disk before the erase is.
fn commit(
    tx: Transaction<'_>,
    ledger: Option<Ledger<'_>>,
    (kind, key): (&str, &str),
    digests: &[String],
    at: &str,
) -> Result<()> {
    if let Some(ledger) = ledger {
        let entry = Entry {
            kind,
            key,
            digests,
            erased_at: at,
        };
        identity::record(ledger.held, ledger.paths, &tx, &entry)?;
    }
    tx.commit()?;
    Ok(())
}

/// The post-commit steps: refresh the derived graph state, `optimize` every
/// FTS index in `touched` (and `edge_fts`, which the refresh rewrote), then
/// truncate the WAL — all with `secure_delete` on, so freed pages are zeroed.
/// Returns whether the WAL was truncated.
///
/// # Errors
/// Propagates SQLite failures; the refresh itself is best-effort.
pub fn settle(conn: &mut Connection, touched: &[FtsIndex]) -> Result<bool> {
    erase_rows::with_secure_delete(conn, |conn| {
        let _fresh = crate::domains::graph::derived::refresh_derived_best_effort(conn);
        for index in touched.iter().copied().chain([FtsIndex::Edges]) {
            erase_rows::optimize(conn, index)?;
        }
        erase_rows::truncate_wal(conn, CHECKPOINT_WAIT)
    })
}

/// Mark every pending upsert or restore for `(kind, key)` rejected with
/// `payload_erased` — a pending tombstone still goes. Returns rows withdrawn.
fn withdraw(tx: &Connection, kind: &str, key: &str, at: &str) -> Result<u64> {
    let mut withdrawn = 0u64;
    for op in replica_outbox::read(tx, Scope::Entity(kind, key), usize::MAX)? {
        if op.op == ReplicaOp::Tombstone {
            continue;
        }
        let rejected = Outcome::Rejected {
            disposition: Disposition::PayloadErased.as_str(),
        };
        withdrawn +=
            u64::try_from(replica_outbox::record(tx, &op.operation_id, rejected, at)?).unwrap_or(0);
    }
    Ok(withdrawn)
}

/// Blank every copy of `digests` beyond `replica_payload`: the replay scratch
/// and complete staged sets. Returns `(replay rows, staged parts)`.
fn clear_copies(tx: &Connection, digests: &[String]) -> Result<(u64, u64)> {
    Ok((
        replica_redaction_copies::clear_replay_of(tx, digests)?,
        erase_rows::erase_staged_of(tx, digests)?,
    ))
}

/// Every rollback snapshot present in `data_dir` that predates this erase:
/// the migration, rebuild and restore `.bak` files, `memories.pre-restore/`,
/// and each `backups/<timestamp>/` directory `backup create` writes.
fn snapshots_with_prior_state(data_dir: &Path) -> Result<Vec<String>> {
    let mut found: Vec<PathBuf> = entries(data_dir)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(is_snapshot)
        })
        .collect();
    found.extend(
        entries(&data_dir.join("backups"))?
            .into_iter()
            .filter(|path| path.is_dir()),
    );
    found.sort();
    Ok(found
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect())
}

/// The entries directly under `dir`; none when `dir` does not exist.
fn entries(dir: &Path) -> Result<Vec<PathBuf>> {
    match std::fs::read_dir(dir) {
        Ok(read) => read
            .map(|entry| Ok(entry?.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(Error::from),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Whether a data-directory entry is one of the named rollback snapshots.
fn is_snapshot(name: &str) -> bool {
    let migration = name.starts_with("comemory.db.pre-v")
        && Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bak"));
    migration
        || matches!(
            name,
            "comemory.db.pre-rebuild.bak" | "comemory.db.pre-restore.bak" | "memories.pre-restore"
        )
}

#[cfg(test)]
#[path = "tests/erase.rs"]
mod tests;
