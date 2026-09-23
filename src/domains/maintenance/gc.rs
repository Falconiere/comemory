//! `maintenance::gc::{Request, run}` — the shared middle of `comemory gc` / `POST
//! /api/v1/gc`: reap entries in `memories/.trash/` older than
//! `prune.trash_retention_days` (30 by default) **together with their
//! mirror rows** in `comemory.db` (`store::memory_purge`), and evict
//! learning telemetry (`retrieval_log`, `feedback_events`) past
//! `prune.learning_retention_days`. Moved out of `cli::gc::run` (Binding
//! Rule 1). Both windows are the `GET|PUT /api/v1/gc/policy` knobs.
//!
//! It also runs the abandoned-stage sweep ([`replica_sweep`]), whose window
//! is its own: an unfinished upload is debris within a day, and it published
//! nothing, so it does not wait out a telemetry retention window.
//!
//! A reaped file's `memories` row must go with it: a row left behind is a
//! zombie — `GET /api/v1/trash` lists it with `path: null` forever,
//! `POST /trash/{id}/restore` answers 404, and `stats.trashed` only ever
//! grows. Earlier `gc` versions unlinked the file alone, so every sweep
//! also purges the rows whose `deleted_at` is past the window and whose
//! trash file is already gone, healing a store those versions left behind.
//!
//! **Must-not-create-the-db invariant:** `run` calls [`Ctx::conn`] only when
//! `comemory.db` already exists on disk — `gc` on a fresh data dir must never
//! create (and migrate) a db as a side effect.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::domains::memories::trash::trash_entry_id;
use crate::prelude::*;
use crate::store::{
    Connection, activity, candidate_observations, gc_learning, gc_runs, memory_purge, memory_row,
    random_id, replica_sweep,
};
use crate::utilities::context::Ctx;

/// `comemory gc` / `POST /api/v1/gc` request. No CLI args today.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// Removal counts from one `gc` run.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Trashed memory files hard-deleted (mtime past
    /// `prune.trash_retention_days`).
    pub removed: u64,
    /// `retrieval_log` rows evicted past the configured retention window.
    pub log_rows: u64,
    /// `feedback_events` rows evicted past the configured retention window.
    pub event_rows: u64,
    /// Captured candidate observations evicted past the same window
    /// (`candidate_query_observations` rows; their `candidate_observations`
    /// children go with them). An observation carrying a reviewed judgment is
    /// retained however old it is, so this counts only unjudged ones.
    pub observation_rows: u64,
    /// `activity_log` rows evicted past the same window: the recorded
    /// command runs behind `GET /api/v1/activity`.
    pub activity_rows: u64,
    /// Summed size, in bytes, of the trashed files this run actually
    /// removed (stat'd before the unlink, never estimated after).
    pub bytes_freed: u64,
    /// Soft-deleted `memories` rows hard-deleted from `comemory.db`, each
    /// with its tags, FTS, vector, edge, code-ref and feedback rows
    /// ([`memory_purge::purge_memory`]): the rows behind the files this
    /// run reaped, plus any zombie row an earlier sweep left behind
    /// (`deleted_at` past the window, trash file already gone). `0` when
    /// `comemory.db` does not exist.
    pub purged_rows: u64,
    /// Rows of uploads that never finished: staged parts whose last part
    /// never arrived, plus generations still `staged`, both past
    /// [`replica_sweep::ABANDONED_AFTER_HOURS`]. An active generation, its
    /// projection and every receipt are untouched — a swept receipt would
    /// turn a peer's retry into a second acceptance.
    pub staged_rows: u64,
    /// The purge left the derived artifacts stale: `edge_fts` could not be
    /// rebuilt after the rows went. The purge itself committed — this is a
    /// freshness warning, not a failure — but relation search is behind
    /// until the next write refreshes it, and the operator reading a `gc`
    /// report is who should know. Omitted from the JSON when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub derived_stale: bool,
}

/// Random bytes behind a `gc_runs` row id — 8 bytes, rendered as 16
/// lowercase-hex chars (the same width as a job id).
const RUN_ID_BYTES: usize = 8;

/// What one pass over `memories/.trash/` did.
struct Sweep {
    /// Files unlinked.
    removed: u64,
    /// Summed size of the unlinked files.
    bytes_freed: u64,
    /// Memory ids of the `{id}-{slug}.md` files this pass unlinked.
    reaped: Vec<String>,
    /// Memory ids of the entries still on disk after the pass — a row with
    /// one of these is not a zombie, whatever its `deleted_at` says.
    kept: HashSet<String>,
}

/// Remove every file in the trash directory whose mtime is older than
/// `prune.trash_retention_days`, then — only when `comemory.db` already
/// exists — purge the reaped memories' mirror rows (and the zombie rows of
/// earlier sweeps), evict learning telemetry older than
/// `prune.learning_retention_days`, AND record this run in `gc_runs`.
/// Missing trash directory is a no-op. The must-not-create-the-db invariant
/// means a fresh data dir writes no `gc_runs` row either — there is nowhere
/// to write it.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<Response> {
    let trash_days = ctx.cfg.prune.trash_retention_days;
    let sweep = sweep_trash(&ctx.paths.trash_dir(), trash_days);

    let (counts, observation_rows, activity_rows, purge, staged) = if ctx.paths.db_path().exists() {
        let retention_days = ctx.cfg.prune.learning_retention_days;
        let conn = ctx.conn()?;
        let now = OffsetDateTime::now_utc();
        // The trash purge runs first, so an observation redacted by it is
        // still the same row this sweep may then evict.
        let purge = purge_rows(conn, &sweep, trash_days)?;
        // One cutoff for both sweeps: telemetry ages out unconditionally,
        // while an observation carrying a reviewed judgment is retained
        // however old it is — the evidence behind human review outlives the
        // window that evicts raw telemetry.
        let cutoff = retention_cutoff(retention_days, now)?;
        let counts = gc_learning::evict_before(conn, &cutoff)?;
        let (observation_rows, _candidate_rows) =
            candidate_observations::evict_unjudged_before(conn, &cutoff)?;
        // The activity feed is telemetry of the same class as `retrieval_log`
        // and `feedback_events`, so it ages out under the same window rather
        // than under a second one of its own.
        let activity_rows = activity::delete_before(conn, &cutoff)?;
        // An upload that never finished published nothing, so its rows are
        // debris on their own window rather than on the telemetry one.
        let staged = replica_sweep::run(conn, now)?;
        record_run(conn, &sweep, counts, activity_rows, now)?;
        (counts, observation_rows, activity_rows, purge, staged)
    } else {
        (
            (0, 0),
            0,
            0,
            Purge::default(),
            replica_sweep::Swept::default(),
        )
    };
    let (log_rows, event_rows) = counts;

    Ok(Response {
        removed: sweep.removed,
        log_rows,
        event_rows,
        bytes_freed: sweep.bytes_freed,
        observation_rows,
        activity_rows,
        purged_rows: purge.rows,
        staged_rows: staged.parts + staged.generations,
        derived_stale: purge.derived_stale,
    })
}

/// Remove every file directly under `trash_dir` whose mtime is older than
/// `retention_days`. Each file's size is stat'd BEFORE the unlink, so a
/// failed `remove_file` never counts toward `bytes_freed`. Missing trash
/// directory yields an empty [`Sweep`]. Every file past the window is
/// unlinked (as before); only the ones named `{id}-{slug}.md` contribute an
/// id to `reaped` / `kept`.
fn sweep_trash(trash_dir: &Path, retention_days: u32) -> Sweep {
    let mut sweep = Sweep {
        removed: 0,
        bytes_freed: 0,
        reaped: Vec::new(),
        kept: HashSet::new(),
    };
    let Ok(rd) = std::fs::read_dir(trash_dir) else {
        return sweep;
    };
    let window = std::time::Duration::from_secs(u64::from(retention_days) * 86_400);
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name();
        let id = trash_entry_id(&name.to_string_lossy()).map(str::to_owned);
        let too_old = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|d| d > window);
        if too_old && std::fs::remove_file(entry.path()).is_ok() {
            sweep.removed += 1;
            sweep.bytes_freed += meta.len();
            sweep.reaped.extend(id);
        } else {
            sweep.kept.extend(id);
        }
    }
    sweep
}

/// Hard-delete the mirror rows behind this sweep: one purge per reaped
/// file, then one pass over the rows whose `deleted_at` is past the window
/// but whose trash file is already gone (an earlier `gc` unlinked it before
/// gc purged rows at all). `Sweep::kept` excludes every entry still on
/// disk, so a row with a live file is never purged on the strength of an
/// old stamp. Returns the rows actually purged — a reaped file whose row is
/// live, or already gone, counts nothing. The derived artifacts (memory
/// rank, `edge_fts`) are refreshed once afterwards, since purged incoming
/// edges leave the triplet index stale.
///
/// That refresh runs AFTER the purge transactions commit — inside them a
/// failure would roll back rows that were correctly removed — so it cannot
/// fail the run. It is reported instead: [`Purge::derived_stale`] carries
/// it into the response rather than leaving it in the log alone.
fn purge_rows(conn: &mut Connection, sweep: &Sweep, trash_days: u32) -> Result<Purge> {
    let mut purged = 0u64;
    for id in &sweep.reaped {
        purged += u64::from(memory_purge::purge_memory(conn, id)?);
    }
    for id in memory_purge::expired_deleted_ids(conn, trash_days)? {
        if !sweep.kept.contains(&id) {
            purged += u64::from(memory_purge::purge_memory(conn, &id)?);
        }
    }
    let derived_stale =
        purged > 0 && !crate::domains::graph::derived::refresh_derived_best_effort(conn);
    Ok(Purge {
        rows: purged,
        derived_stale,
    })
}

/// What [`purge_rows`] did: the rows it hard-deleted, and whether the
/// derived-artifact refresh that follows them failed. `default()` is the
/// no-database case — nothing purged, nothing to refresh.
#[derive(Default)]
struct Purge {
    /// Mirror rows hard-deleted.
    rows: u64,
    /// The triplet index could not be rebuilt after the purge.
    derived_stale: bool,
}

/// Insert one `gc_runs` row for this completed sweep. Only reached when
/// `comemory.db` already exists (the caller's `conn` came from [`Ctx::conn`]
/// after that check). `counts` is `(log_rows, event_rows)`.
fn record_run(
    conn: &Connection,
    sweep: &Sweep,
    counts: (u64, u64),
    activity_rows: u64,
    now: OffsetDateTime,
) -> Result<()> {
    let id = random_id::random_hex(RUN_ID_BYTES)?;
    let at = memory_row::iso_format(now)?;
    gc_runs::insert(
        conn,
        &gc_runs::NewGcRun {
            id: &id,
            at: &at,
            removed: sweep.removed,
            log_rows: counts.0,
            event_rows: counts.1,
            bytes_freed: sweep.bytes_freed,
            activity_rows,
        },
    )
}
/// The retention cutoff both sweeps compare against, rendered in the
/// fixed-width ISO-8601 UTC shape every `at` column is written in.
///
/// `memory_row::iso_format` renders ISO-8601 with nine fractional digits,
/// verified empirically (whole-second values render as `.000000000Z`, see the
/// shape assertion in `tests/cli/gc.rs`). On identical-width ISO-8601 UTC
/// strings lexicographic `<` is exactly chronological, so a plain string
/// comparison against the rendered cutoff is correct without any `substr`
/// truncation.
fn retention_cutoff(retention_days: u32, now: OffsetDateTime) -> Result<String> {
    memory_row::iso_format(now - time::Duration::days(i64::from(retention_days)))
}

#[cfg(test)]
#[path = "tests/gc.rs"]
mod tests;
