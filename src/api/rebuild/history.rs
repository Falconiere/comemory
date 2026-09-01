//! The run-history half of the preservation copy — `eval_runs` (v14, plus
//! v15's `discarded` flag), `gc_runs` (v14), and `index_runs` (v15) — called
//! from `copy.rs`'s learning-state pass. History is exactly what markdown
//! cannot reconstruct: a rebuild that dropped it would erase every recorded
//! eval, gc, and index run (and re-offer every discarded knob proposal).

use super::copy::{old_column_exists, old_table_exists};
use crate::prelude::*;

/// Copy the three run-history tables from the attached `old` database. A
/// pre-v15 `eval_runs` has no `discarded` column, so it is synthesized as
/// `0` (the migration's own default); a pre-v14 source has none of these
/// tables and each block is skipped.
pub(super) fn copy_history_tables(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "eval_runs")? {
        let discarded_expr = if old_column_exists(conn, "eval_runs", "discarded")? {
            "discarded"
        } else {
            "0"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.eval_runs(\
                 id, kind, at, golden_pairs, k, recall, mrr, knobs, applied, discarded) \
             SELECT id, kind, at, golden_pairs, k, recall, mrr, knobs, applied, {discarded_expr} \
             FROM old.eval_runs;"
        ))?;
    }
    if old_table_exists(conn, "gc_runs")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.gc_runs(\
                 id, at, removed, log_rows, event_rows, bytes_freed) \
             SELECT id, at, removed, log_rows, event_rows, bytes_freed \
             FROM old.gc_runs;",
        )?;
    }
    if old_table_exists(conn, "index_runs")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.index_runs(\
                 id, repo, root_path, mode, started_at, finished_at, duration_ms, \
                 files_indexed, symbols, outcome, error) \
             SELECT id, repo, root_path, mode, started_at, finished_at, duration_ms, \
                 files_indexed, symbols, outcome, error \
             FROM old.index_runs;",
        )?;
    }
    Ok(())
}
