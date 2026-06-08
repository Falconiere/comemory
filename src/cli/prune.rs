//! `comemory prune` — surface candidates for deletion against the v0.2
//! SQLite mirror.
//!
//! v0.2 collapses the v0.1 markdown-scanning detectors (orphan trash
//! entries, low-value memories) into a single SQL pass against
//! `comemory.db`. Two classes are reported:
//!
//! 1. **Orphan edges** — `edges` rows whose `src_kind = 'memory'` source
//!    is missing from `memories` or has `deleted_at IS NOT NULL`. These
//!    accumulate when a memory is soft-deleted by `comemory delete`.
//! 2. **Stale code files** — distinct `code_symbols.path`s that no
//!    longer appear in `indexed_files` (i.e. the source file was
//!    removed and a follow-up `index-code` never cleaned up the
//!    leftover symbol rows).
//!
//! Default behaviour applies the cleanup in one transaction. Use
//! `--dry-run` to inspect what would be removed without touching the
//! DB.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::output::prune as output;
use crate::prelude::*;
use crate::store::connection;

/// Example invocations shown at the bottom of `comemory prune --help`.
pub const EXAMPLES: &str = "\
Examples:
  # Inspect candidates without mutating the DB
  comemory prune --dry-run

  # Apply orphan-edge + stale-code-symbol cleanup
  comemory prune

  # JSON output for CI/automation
  comemory prune --json";

/// Arguments to `comemory prune`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Report candidates without applying any deletes.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

/// Output schema for both JSON and TTY rendering. Lives at module scope so
/// downstream tooling can parse the JSON shape directly.
#[derive(Serialize, Debug)]
pub struct Report {
    /// Count of `edges` rows whose source memory is missing or
    /// soft-deleted.
    pub orphan_edges: i64,
    /// Distinct `<repo>:<path>` values whose corresponding `indexed_files`
    /// row has been removed. The repo prefix disambiguates identical paths
    /// across different repos (e.g. `src/main.rs` in two checkouts).
    pub stale_code_files: Vec<String>,
}

/// Scan `comemory.db` for prune candidates and (unless `--dry-run`)
/// apply the cleanup in one transaction. Always emits the report.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let report = scan(&conn)?;
    if !a.dry_run {
        apply(&mut conn)?;
    }
    output::emit(&report, json_flag)
}

/// Read-only candidate scan. Returns the orphan-edge count and the list
/// of stale code paths without touching either table.
fn scan(conn: &rusqlite::Connection) -> Result<Report> {
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
    let stale_code_files = stmt
        .query_map([], |r| {
            let repo: String = r.get(0)?;
            let path: String = r.get(1)?;
            Ok(format!("{repo}:{path}"))
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(Report {
        orphan_edges,
        stale_code_files,
    })
}

/// Apply the cleanup queries reported by [`scan`] in a single
/// transaction. Safe to call on a clean DB — all `DELETE` statements are
/// no-ops when no candidates exist.
///
/// `code_vec` and `code_fts` are vec0 / fts5 virtual tables and do not
/// participate in the SQLite FK cascade triggered by deleting `code_symbols`,
/// so we explicitly drop their rows first (keyed by the `id` of the about-
/// to-be-removed `code_symbols` row). Otherwise prune would leave orphan
/// vector and FTS rows that the KNN / BM25 path could still surface.
fn apply(conn: &mut rusqlite::Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM edges WHERE src_kind = 'memory' \
           AND NOT EXISTS(SELECT 1 FROM memories m \
                            WHERE m.id = src_id AND m.deleted_at IS NULL)",
        [],
    )?;
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
    // Drop `references_symbol` / `references_file` edges whose dst no longer
    // exists. `bundle::code_ref_lookup` already tolerates a dangling dst
    // (returns `None`), so leaving these would not produce a user-visible
    // bug — but the edge count grows monotonically across prune cycles and
    // every read-time lookup pays a wasted DB hit. The dst_id is the
    // textual qualified form (`<repo>:<path>:<symbol>` for symbols,
    // `<repo>:<path>` for files); both match the existence checks below.
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
    tx.commit()?;
    Ok(())
}
