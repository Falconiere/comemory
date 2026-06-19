//! `comemory prune` — surface candidates for deletion against the SQLite mirror.
//!
//! Reported classes: orphan `edges` (source memory missing/deleted), stale code
//! files (`code_symbols` paths gone from `indexed_files`), low-value memories
//! ([`low_value::detect`]: cold/unloved/low-quality/unreferenced or superseded),
//! and ghost code-refs ([`stale_code::detect`]: memories whose pinned symbol no
//! longer resolves — advisory only, never auto-deleted, per spec Non-Goal 5).
//!
//! Default is a dry run. `--apply` soft-deletes low-value memories through the
//! `comemory delete` path then runs the orphan/stale cleanup in one
//! transaction; ghost-ref candidates are surfaced but not deleted.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::pagination::PaginationArgs;
use crate::cli::{delete, load_config, resolve_data_dir};
use crate::config::Config;
use crate::config::paths::Paths;
use crate::output::page::Page;
use crate::output::prune as output;
use crate::prelude::*;
use crate::prune::{low_value, stale_code};
use crate::store::connection;

/// Example invocations shown at the bottom of `comemory prune --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Default is a dry run: inspect candidates without mutating anything
  comemory prune

  # Apply: soft-delete low-value memories (markdown -> memories/.trash/)
  # and clean up orphan edges + stale code symbols
  comemory prune --apply

  # Page the dry-run lists (window applies to display only; --apply is
  # always full-set): second page of 20 candidates
  comemory prune --limit 20 --offset 20

  # JSON output for CI/automation; Report fields:
  #   low_value_memories / stale_code_files / ghost_ref_memories — Page
  #     envelopes ({items, limit, offset, total, has_more}). low_value ids
  #     match ALL of: activation < COMEMORY_PRUNE_MIN_ACTIVATION (-2.0), Beta
  #     feedback <= COMEMORY_PRUNE_MIN_FEEDBACK (0.25), quality <=
  #     COMEMORY_PRUNE_BELOW_QUALITY (2), and zero incoming edges — OR
  #     superseded by a live memory with no access since the supersede edge.
  #   ghost_ref_memories: owners of a pinned --ref-symbol whose target is gone
  #     from a CURRENT index (advisory — never deleted by --apply).
  comemory prune --json";

/// Arguments to `comemory prune`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Execute the cleanup (soft-delete low-value memories, drop orphan
    /// edges + stale code symbols). Without this flag prune only scans and
    /// reports.
    #[arg(long, default_value_t = false)]
    pub apply: bool,
    /// `--limit` / `--offset` window over the dry-run `stale_code_files`
    /// and `low_value_memories` lists. The same window applies to BOTH.
    /// It windows DISPLAY ONLY: `--apply` always acts on the full
    /// candidate set regardless of `--limit` / `--offset`.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// Output schema for both JSON and TTY rendering. Lives at module scope so
/// downstream tooling can parse the JSON shape directly.
#[derive(Serialize, Debug)]
pub struct Report {
    /// Count of `edges` rows whose source memory is missing or
    /// soft-deleted. A bare count (already a number, not a list), so it
    /// is never paginated.
    pub orphan_edges: i64,
    /// Paginated `<repo>:<path>` values whose corresponding `indexed_files`
    /// row has been removed. The repo prefix disambiguates identical paths
    /// across different repos (e.g. `src/main.rs` in two checkouts). The
    /// shared `--limit` / `--offset` window applies to the dry-run display
    /// only.
    pub stale_code_files: Page<String>,
    /// Paginated memory ids flagged by [`low_value::detect`] — soft-delete
    /// candidates (applied to the FULL set, not the page, when `--apply`
    /// is set). The window applies to the dry-run display only.
    pub low_value_memories: Page<String>,
    /// Paginated memory ids flagged by [`stale_code::detect`] — owners of a
    /// pinned `references_symbol` whose target is a `ghost` (gone from a
    /// current index). Advisory: surfaced for the operator, never deleted by
    /// `--apply` (spec Non-Goal 5). The window applies to display only.
    pub ghost_ref_memories: Page<String>,
}

/// Scan `comemory.db` for prune candidates and, only when `--apply` is
/// set, apply the cleanup. The scan runs FIRST so the emitted report reflects
/// the candidates that were (about to be) pruned; its `stale_code_files` and
/// `low_value_memories` lists are windowed to `a.page` for display. `--apply`
/// then acts on the FULL low-value candidate set captured by the scan (never
/// the page) so pagination can never reduce what gets soft-deleted. Always
/// emits the report.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let mut conn = connection::open(paths.db_path())?;
    let scanned = scan(&conn, &cfg, a.page.limit, a.page.offset)?;
    if a.apply {
        // Act on the FULL candidate set the scan captured (not the windowed
        // report) — pagination is a display concern and must not gate
        // deletions.
        apply(&mut conn, &paths, &scanned.full_low_value)?;
    }
    output::emit(&scanned.report, json_flag)
}

/// A completed scan: the windowed [`Report`] for display plus the FULL
/// (unwindowed) low-value candidate list that `--apply` must act on.
struct Scan {
    /// Display report, with both lists windowed to `(limit, offset)`.
    report: Report,
    /// Every flagged low-value id, regardless of the page window.
    full_low_value: Vec<String>,
}

/// Read-only candidate scan. Builds the windowed display [`Report`] AND
/// captures the full low-value candidate list (so `--apply` acts on every
/// id, never just the page). `limit == 0` is the shared "all" sentinel.
fn scan(conn: &rusqlite::Connection, cfg: &Config, limit: usize, offset: usize) -> Result<Scan> {
    let orphan_edges: i64 = conn.query_row(
        "SELECT count(*) FROM edges e \
          WHERE e.src_kind = 'memory' \
            AND NOT EXISTS(SELECT 1 FROM memories m \
                             WHERE m.id = e.src_id AND m.deleted_at IS NULL)",
        [],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT repo, path FROM code_symbols \
          WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                             WHERE i.repo = code_symbols.repo \
                               AND i.path = code_symbols.path) \
          ORDER BY repo, path",
    )?;
    let stale: Vec<String> = stmt
        .query_map([], |r| {
            let repo: String = r.get(0)?;
            let path: String = r.get(1)?;
            Ok(format!("{repo}:{path}"))
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    // Detect the full candidate set once: window a clone for the report,
    // keep the full list for `--apply`.
    let full_low_value = low_value::detect(conn, cfg)?;
    // Ghost code-refs are advisory (spec Non-Goal 5): detected and reported,
    // never fed to `--apply`.
    let ghost_refs = stale_code::detect(conn)?;
    let report = Report {
        orphan_edges,
        stale_code_files: Page::from_slice(stale, limit, offset),
        low_value_memories: Page::from_slice(full_low_value.clone(), limit, offset),
        ghost_ref_memories: Page::from_slice(ghost_refs, limit, offset),
    };
    Ok(Scan {
        report,
        full_low_value,
    })
}

/// Apply the cleanup reported by [`scan`]: soft-delete low-value memories
/// ([`soft_delete_low_value`]) then drop orphan/stale rows in one transaction
/// ([`cleanup_orphans`]). Safe on a clean DB — every `DELETE` is a no-op when
/// no candidates exist.
fn apply(conn: &mut rusqlite::Connection, paths: &Paths, low_value_ids: &[String]) -> Result<()> {
    soft_delete_low_value(conn, paths, low_value_ids)?;
    cleanup_orphans(conn)
}

/// Soft-delete every flagged low-value id through [`delete::soft_delete`] (the
/// same path `comemory delete` uses), healing the DB mirror when a flagged
/// memory's markdown is already gone so prune cannot wedge on a half-deleted
/// row. Ghost-ref candidates are intentionally NOT deleted here: they are
/// advisory (spec Non-Goal 5).
fn soft_delete_low_value(
    conn: &mut rusqlite::Connection,
    paths: &Paths,
    low_value_ids: &[String],
) -> Result<()> {
    for id in low_value_ids {
        match delete::soft_delete(paths, conn, id) {
            Ok(_) => {}
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
                delete::mirror_soft_delete(conn, id)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Drop orphan/stale rows in a single transaction: orphan memory edges, the
/// `code_vec` / `code_fts` / `code_symbols` rows for files no longer in
/// `indexed_files`, and the now-dangling `references_*` / `co_activated` edges.
fn cleanup_orphans(conn: &mut rusqlite::Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM edges WHERE src_kind = 'memory' \
           AND NOT EXISTS(SELECT 1 FROM memories m \
                            WHERE m.id = src_id AND m.deleted_at IS NULL)",
        [],
    )?;
    purge_stale_code_rows(&tx)?;
    drop_dangling_edges(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Delete the `code_vec` / `code_fts` / `code_symbols` rows for files no longer
/// in `indexed_files`. The two virtual tables (vec0 / fts5) don't participate
/// in the FK cascade, so their rows are dropped first by the about-to-be-removed
/// `code_symbols.id`.
fn purge_stale_code_rows(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute(
        "DELETE FROM code_vec WHERE symbol_id IN ( \
             SELECT id FROM code_symbols \
              WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                                 WHERE i.repo = code_symbols.repo \
                                   AND i.path = code_symbols.path))",
        [],
    )?;
    tx.execute(
        "DELETE FROM code_fts WHERE symbol_id IN ( \
             SELECT id FROM code_symbols \
              WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                                 WHERE i.repo = code_symbols.repo \
                                   AND i.path = code_symbols.path))",
        [],
    )?;
    tx.execute(
        "DELETE FROM code_symbols \
          WHERE NOT EXISTS(SELECT 1 FROM indexed_files i \
                             WHERE i.repo = code_symbols.repo \
                               AND i.path = code_symbols.path)",
        [],
    )?;
    Ok(())
}

/// Drop edges that dangle once a file's `code_symbols` rows are purged:
/// `references_symbol` / `references_file` (bare qualified dst) and
/// `co_activated` (the `file:`-prefixed node id from `graph::edges`). The
/// read path tolerates a dangling dst, but the count grows every prune cycle.
fn drop_dangling_edges(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute(
        "DELETE FROM edges \
          WHERE rel = 'references_symbol' \
            AND NOT EXISTS( \
                SELECT 1 FROM code_symbols cs \
                 WHERE edges.dst_id = cs.repo || ':' || cs.path || ':' || cs.symbol \
            )",
        [],
    )?;
    tx.execute(
        "DELETE FROM edges \
          WHERE rel = 'references_file' \
            AND NOT EXISTS( \
                SELECT 1 FROM code_symbols cs \
                 WHERE edges.dst_id = cs.repo || ':' || cs.path \
            )",
        [],
    )?;
    tx.execute(
        "DELETE FROM edges \
          WHERE rel = 'co_activated' \
            AND NOT EXISTS( \
                SELECT 1 FROM code_symbols cs \
                 WHERE edges.dst_id = 'file:' || cs.repo || ':' || cs.path \
            )",
        [],
    )?;
    drop_orphan_code_refs(tx)?;
    Ok(())
}

/// Drop `code_ref` rows left behind once their backing `references_file` /
/// `references_symbol` edge is gone. `code_ref` and `edges` share the same
/// `(memory_id, rel, dst_id)` key for these two relations, so a `code_ref`
/// with no surviving edge is an orphan — exactly the rows
/// [`drop_dangling_edges`] just purged (dangling dst) or that a deleted
/// memory's edge sweep removed. Without this, `stale_code::detect` would keep
/// re-flagging the memory and the side table would diverge from the read path.
fn drop_orphan_code_refs(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute(
        "DELETE FROM code_ref \
          WHERE NOT EXISTS( \
              SELECT 1 FROM edges e \
               WHERE e.rel = code_ref.rel \
                 AND e.src_id = code_ref.memory_id \
                 AND e.dst_id = code_ref.dst_id \
          )",
        [],
    )?;
    Ok(())
}
