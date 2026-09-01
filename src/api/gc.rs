//! `api::gc::{Request, run}` — the shared middle of `comemory gc` / `POST
//! /api/v1/gc`: reap entries in `memories/.trash/` older than
//! `prune.trash_retention_days` (30 by default) **together with their
//! mirror rows** in `comemory.db` (`store::memory_purge`), and evict
//! learning telemetry (`retrieval_log`, `feedback_events`) past
//! `prune.learning_retention_days`. Moved out of `cli::gc::run` (Binding
//! Rule 1). Both windows are the `GET|PUT /api/v1/gc/policy` knobs.
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

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::api::trash::trash_entry_id;
use crate::prelude::*;
use crate::store::{gc_runs, memory_purge, memory_row, random_id};

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

    let (log_rows, event_rows, purged_rows) = if ctx.paths.db_path().exists() {
        let retention_days = ctx.cfg.prune.learning_retention_days;
        let conn = ctx.conn()?;
        let now = OffsetDateTime::now_utc();
        let purged = purge_rows(conn, &sweep, trash_days)?;
        let counts = sweep_learning(conn, retention_days, now)?;
        record_run(conn, &sweep, counts, now)?;
        (counts.0, counts.1, purged)
    } else {
        (0, 0, 0)
    };

    Ok(Response {
        removed: sweep.removed,
        log_rows,
        event_rows,
        bytes_freed: sweep.bytes_freed,
        purged_rows,
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
fn purge_rows(conn: &mut Connection, sweep: &Sweep, trash_days: u32) -> Result<u64> {
    let mut purged = 0u64;
    for id in &sweep.reaped {
        purged += u64::from(memory_purge::purge_memory(conn, id)?);
    }
    for id in memory_purge::expired_deleted_ids(conn, trash_days)? {
        if !sweep.kept.contains(&id) {
            purged += u64::from(memory_purge::purge_memory(conn, &id)?);
        }
    }
    if purged > 0 {
        crate::graph::derived::refresh_derived_best_effort(conn);
    }
    Ok(purged)
}

/// Insert one `gc_runs` row for this completed sweep. Only reached when
/// `comemory.db` already exists (the caller's `conn` came from [`Ctx::conn`]
/// after that check). `counts` is `(log_rows, event_rows)`.
fn record_run(
    conn: &Connection,
    sweep: &Sweep,
    counts: (u64, u64),
    now: OffsetDateTime,
) -> Result<()> {
    let id = random_id::random_hex(RUN_ID_BYTES)?;
    let at = memory_row::iso_format(now)?;
    gc_runs::insert(
        conn,
        &id,
        &at,
        sweep.removed,
        counts.0,
        counts.1,
        sweep.bytes_freed,
    )
}

/// Evict learning telemetry older than the retention window. Counters in
/// `feedback` are permanent; only raw event rows age out.
///
/// Both `retrieval_log.at` and `feedback_events.at` are written via
/// [`memory_row::iso_format`] (`Iso8601::DEFAULT`), which renders a
/// fixed-width `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ` string — always nine
/// fractional digits, verified empirically (whole-second values render as
/// `.000000000Z`, see the shape assertion in `tests/cli/gc.rs`). On
/// identical-width ISO-8601 UTC strings, lexicographic `<` is exactly
/// chronological, so a plain string comparison against the rendered cutoff
/// is correct without any `substr` truncation.
fn sweep_learning(
    conn: &Connection,
    retention_days: u32,
    now: OffsetDateTime,
) -> Result<(u64, u64)> {
    let cutoff = memory_row::iso_format(now - time::Duration::days(i64::from(retention_days)))?;
    let logs = conn.execute("DELETE FROM retrieval_log WHERE at < ?1", [&cutoff])? as u64;
    let events = conn.execute("DELETE FROM feedback_events WHERE at < ?1", [&cutoff])? as u64;
    Ok((logs, events))
}

#[cfg(test)]
#[path = "tests/gc.rs"]
mod tests;
