//! `api::prune::{Request, run}` — the shared middle of `comemory prune` /
//! `GET /api/v1/prune`: scan `comemory.db` for prune candidates and, when
//! `apply` is set, execute the cleanup. Moved out of `cli::prune::run`
//! (Binding Rule 1).
//!
//! The full `apply` path (soft-delete + orphan/stale cleanup) moves here in
//! full because the CLI still drives it end to end via `--apply`; only the
//! HTTP surface is scoped down for this step — `GET /api/v1/prune` always
//! forces `apply: false` before calling [`run`] (see
//! `serve::routes::maint::prune`). The confirm-gated mutating `POST
//! /api/v1/prune` route lands in a later step.

use serde::Deserialize;
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::cli::delete;
use crate::config::{Config, Paths};
use crate::output::page::Page;
use crate::output::prune::{PruneRow, Report};
use crate::output::search::title_of;
use crate::prelude::*;
use crate::prune::{low_value, stale_code};
use crate::retrieval::score;
use crate::store::{Connection, prune_apply};

/// `comemory prune` / `GET /api/v1/prune` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Execute the cleanup (soft-delete low-value memories, drop orphan
    /// edges + stale code symbols). `false` only scans and reports.
    #[serde(default)]
    pub apply: bool,
    /// `--limit` / `--offset` window over the dry-run `stale_code_files`
    /// and `low_value_memories` lists (display only — `apply` always acts
    /// on the full candidate set). `0` means "all".
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of leading results to skip before the display window starts.
    #[serde(default)]
    pub offset: usize,
    /// Restrict `apply` to these memory ids (CLI `--ids`, HTTP `ids[]`).
    /// Empty — the default — means "every low-value candidate", today's
    /// behavior. When non-empty, `apply` acts on the INTERSECTION with the
    /// scan's candidates: an id that is not a candidate is ignored rather
    /// than deleted, so a stale console selection can never soft-delete a
    /// memory the scan did not flag.
    #[serde(default)]
    pub ids: Vec<String>,
}

/// `PaginationArgs`' CLI default (`--limit`, unset = 50), reused here so an
/// HTTP request omitting `limit` pages identically to the CLI.
fn default_limit() -> usize {
    50
}

/// Scan `comemory.db` for prune candidates and, only when `req.apply` is
/// set, apply the cleanup. The scan runs FIRST so the emitted report
/// reflects the candidates that were (about to be) pruned; its
/// `stale_code_files` and `low_value_memories` lists are windowed to
/// `(req.limit, req.offset)` for display. `apply` then acts on the FULL
/// low-value candidate set the scan captured (never the page), so
/// pagination can never reduce what gets soft-deleted.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Report> {
    let cfg = ctx.cfg;
    let paths = ctx.paths;
    let conn = ctx.conn()?;
    let mut scanned = scan(&*conn, paths, cfg, req.limit, req.offset)?;
    if req.apply {
        let selected = select_ids(&scanned.full_low_value, &req.ids)?;
        scanned.report.derived_stale = apply(conn, paths, &selected)?;
    }
    Ok(scanned.report)
}

/// The ids `apply` acts on: every candidate when `requested` is empty, else
/// the candidates that were also requested. Order follows `candidates` (the
/// scan's), so the deletion sequence does not depend on how the caller
/// sorted its selection.
///
/// Every requested id is validated through [`crate::cli::parse_id_csv`] —
/// the same 8-hex check `save --supersedes` and `feedback` use — so a
/// malformed id is a hard error naming the flag, not a silently ignored
/// entry.
fn select_ids(candidates: &[String], requested: &[String]) -> Result<Vec<String>> {
    if requested.is_empty() {
        return Ok(candidates.to_vec());
    }
    let wanted = crate::cli::parse_id_csv(&requested.join(","), "--ids")?;
    Ok(candidates
        .iter()
        .filter(|id| wanted.contains(id))
        .cloned()
        .collect())
}

/// A completed scan: the windowed [`Report`] for display plus the FULL
/// (unwindowed) low-value candidate list that `apply` must act on.
struct Scan {
    /// Display report, with both lists windowed to `(limit, offset)`.
    report: Report,
    /// Every flagged low-value id, regardless of the page window.
    full_low_value: Vec<String>,
}

/// Read-only candidate scan. Builds the windowed display [`Report`] AND
/// captures the full low-value candidate list (so `apply` acts on every
/// id, never just the page). `limit == 0` is the shared "all" sentinel.
fn scan(
    conn: &Connection,
    paths: &Paths,
    cfg: &Config,
    limit: usize,
    offset: usize,
) -> Result<Scan> {
    let orphan_edges = prune_apply::count_orphan_memory_edges(conn)?;
    let stale: Vec<String> = prune_apply::stale_code_files(conn)?;
    // Detect the full candidate set once, each id paired with the rule
    // label that flagged it: keep the bare id list for `apply`, enrich only
    // the display-windowed slice into full `PruneRow`s (see [`row_page`]).
    let low_pairs = low_value::detect_with_reasons(conn, cfg)?;
    let full_low_value: Vec<String> = low_pairs.iter().map(|(id, _)| id.clone()).collect();
    // Ghost code-refs are advisory (spec Non-Goal 5): detected and reported,
    // never fed to `apply`.
    let ghost_pairs: Vec<(String, &'static str)> = stale_code::detect(conn)?
        .into_iter()
        .map(|id| (id, "stale code"))
        .collect();

    let now = OffsetDateTime::now_utc();
    let low_value_memories = row_page(conn, cfg, low_pairs, limit, offset, now)?;
    let ghost_ref_memories = row_page(conn, cfg, ghost_pairs, limit, offset, now)?;
    let (trash_count, reclaimable_bytes) = trash_stats(paths);

    let report = Report {
        orphan_edges,
        stale_code_files: Page::from_slice(stale, limit, offset),
        low_value_memories,
        ghost_ref_memories,
        trash_count,
        reclaimable_bytes,
        // A scan reports nothing about the graph; `apply` sets this when
        // its soft deletes leave the triplet index behind.
        derived_stale: false,
    };
    Ok(Scan {
        report,
        full_low_value,
    })
}

/// Window `(id, reason)` pairs to `(limit, offset)`, then enrich ONLY the
/// windowed slice into full [`PruneRow`]s. Each row read costs one query
/// against the memory's body — bounded by the display window (see the
/// module-level cost note on [`build_row`]), never the full candidate set.
fn row_page(
    conn: &Connection,
    cfg: &Config,
    pairs: Vec<(String, &'static str)>,
    limit: usize,
    offset: usize,
    now: OffsetDateTime,
) -> Result<Page<PruneRow>> {
    let windowed = Page::from_slice(pairs, limit, offset);
    let mut rows = Vec::with_capacity(windowed.items.len());
    for (id, reason) in &windowed.items {
        rows.push(build_row(conn, cfg, id, reason, now)?);
    }
    Ok(Page::new(
        rows,
        windowed.limit,
        windowed.offset,
        windowed.total,
        windowed.has_more,
    ))
}

/// Build one [`PruneRow`]: title from the stored body
/// ([`crate::output::search::title_of`]), activation via
/// [`score::activation`] (using `last_accessed`, falling back to
/// `created_at`, and the configured decay), and whole days since creation.
fn build_row(
    conn: &Connection,
    cfg: &Config,
    id: &str,
    reason: &str,
    now: OffsetDateTime,
) -> Result<PruneRow> {
    let row = prune_apply::memory_for_prune(conn, id)?;
    let last = row.last_accessed.as_deref().unwrap_or(&row.created_at);
    let activation = score::activation(
        row.access_count.max(0) as u64,
        score::days_since(last, now),
        cfg.rank.decay,
    );
    let age_days = score::days_since(&row.created_at, now).floor().max(0.0) as u64;
    Ok(PruneRow {
        id: id.to_string(),
        title: title_of(&row.body),
        reason: reason.to_string(),
        activation,
        age_days,
    })
}

/// Count and summed size, in bytes, of every regular file directly under
/// `memories/.trash/`. Missing trash directory (no soft-delete has run yet)
/// yields `(0, 0)` rather than an error.
fn trash_stats(paths: &Paths) -> (u64, u64) {
    let mut count = 0u64;
    let mut bytes = 0u64;
    let Ok(rd) = std::fs::read_dir(paths.trash_dir()) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_file() {
            count += 1;
            bytes += meta.len();
        }
    }
    (count, bytes)
}

/// Apply the cleanup reported by [`scan`]: soft-delete low-value memories
/// ([`soft_delete_low_value`]) then drop orphan/stale rows in one transaction
/// ([`cleanup_orphans`]). Safe on a clean DB — every `DELETE` is a no-op when
/// no candidates exist.
/// Returns whether the soft deletes left the derived artifacts stale — the
/// same signal `delete` and `gc` report, carried up to the prune report.
fn apply(conn: &mut Connection, paths: &Paths, low_value_ids: &[String]) -> Result<bool> {
    let derived_stale = soft_delete_low_value(conn, paths, low_value_ids)?;
    cleanup_orphans(conn)?;
    Ok(derived_stale)
}

/// Soft-delete every flagged low-value id through [`delete::soft_delete`] (the
/// same path `comemory delete` uses), healing the DB mirror when a flagged
/// memory's markdown is already gone so prune cannot wedge on a half-deleted
/// row. Ghost-ref candidates are intentionally NOT deleted here: they are
/// advisory (spec Non-Goal 5).
/// Returns whether ANY of the deletes left the derived artifacts stale, so
/// the caller can report it the way `delete` and `gc` do rather than let a
/// stale relation index reach only the log.
fn soft_delete_low_value(
    conn: &mut Connection,
    paths: &Paths,
    low_value_ids: &[String],
) -> Result<bool> {
    let mut derived_stale = false;
    for id in low_value_ids {
        match delete::soft_delete(paths, conn, id) {
            Ok((_id, _hash, stale)) => derived_stale |= stale,
            // Half-deleted state: live DB row, markdown already gone —
            // producible by a crash inside `delete` between its file move
            // and its DB transaction. The markdown (source of truth)
            // already says deleted, so heal the mirror side instead of
            // aborting; otherwise detect re-flags the row every run and
            // prune wedges forever on the same id.
            Err(Error::NotFound(_)) => {
                tracing::warn!(
                    id = %id,
                    "prune: markdown missing for flagged memory; healing DB mirror"
                );
                derived_stale |= delete::mirror_soft_delete(conn, id)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(derived_stale)
}

/// Drop orphan/stale rows in a single transaction: orphan memory edges, the
/// `code_vec` / `code_fts` / `code_symbols` rows for files no longer in
/// `indexed_files`, and the now-dangling `references_*` / `co_activated` edges.
fn cleanup_orphans(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    prune_apply::delete_orphan_memory_edges(&tx)?;
    prune_apply::purge_stale_code_rows(&tx)?;
    prune_apply::drop_dangling_edges(&tx)?;
    prune_apply::drop_orphan_code_refs(&tx)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/prune.rs"]
mod tests;
