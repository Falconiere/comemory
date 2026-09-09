//! The learning-loop half of [`super::rebuild_copy`]'s preservation copy:
//! `feedback`, `code_feedback`, `retrieval_log`, `feedback_events`,
//! `query_expansions`, `bandit_arms`, plus (via `rebuild_copy_history`) the
//! run-history and sync tables. These rows exist only in SQLite — there is
//! no markdown to rebuild them from — so dropping them would silently reset
//! the Beta feedback rerank priors to neutral and erase mined expansions.
//!
//! Same schema-evolution guards as `rebuild_copy_code`: each table is
//! copied only when it exists on the attached DB, and columns added by
//! later migrations are probed and defaulted per callee.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::rebuild_copy::{old_column_exists, old_table_exists};
use crate::store::rebuild_copy_history;
use crate::store::rebuild_copy_learning_events::copy_event_and_mined_tables;

/// Copy the feedback counters, retrieval log, event/mined tables, and (via
/// [`rebuild_copy_history`]) the run-history and sync tables, in that order.
pub(crate) fn copy_learning_tables_inner(conn: &Connection) -> Result<()> {
    copy_feedback_tables(conn)?;
    copy_retrieval_log(conn)?;
    copy_event_and_mined_tables(conn)?;
    rebuild_copy_history::copy_history_tables(conn)
}

/// Copy the aggregated feedback counters: memory-side `feedback` (v2) and
/// symbol-side `code_feedback` (v6). The `repo` column probe also covers the
/// brief dev-era rowid-keyed `code_feedback` shape (never released): skip
/// rather than abort.
fn copy_feedback_tables(conn: &Connection) -> Result<()> {
    if old_table_exists(conn, "feedback")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.feedback(\
                 memory_id, used_count, irrelevant_count, last_used) \
             SELECT memory_id, used_count, irrelevant_count, last_used \
             FROM old.feedback;",
        )?;
    }
    if old_table_exists(conn, "code_feedback")? && old_column_exists(conn, "code_feedback", "repo")?
    {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.code_feedback(\
                 repo, path, symbol, used_count, irrelevant_count, last_used) \
             SELECT repo, path, symbol, used_count, irrelevant_count, last_used \
             FROM old.code_feedback;",
        )?;
    }
    Ok(())
}

/// Copy the `retrieval_log` telemetry (v3). `duration_ms` (v5) and the
/// `repo`/`kind`/`source` filter columns (v6, probed together via `source`)
/// default to NULL/NULL/NULL/`'search'` when the source predates them —
/// without that, old `search-code` rows would re-enter reformulation mining
/// as memory queries.
fn copy_retrieval_log(conn: &Connection) -> Result<()> {
    if old_table_exists(conn, "retrieval_log")? {
        let duration_expr = if old_column_exists(conn, "retrieval_log", "duration_ms")? {
            "duration_ms"
        } else {
            "NULL"
        };
        let (repo_expr, kind_expr, source_expr) =
            if old_column_exists(conn, "retrieval_log", "source")? {
                ("repo", "kind", "source")
            } else {
                ("NULL", "NULL", "'search'")
            };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.retrieval_log(\
                 query_id, query, returned_ids, at, duration_ms, repo, kind, source) \
             SELECT query_id, query, returned_ids, at, {duration_expr}, \
                 {repo_expr}, {kind_expr}, {source_expr} \
             FROM old.retrieval_log;"
        ))?;
    }
    Ok(())
}
