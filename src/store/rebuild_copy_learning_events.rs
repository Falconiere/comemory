//! The `feedback_events`/`query_expansions`/`bandit_arms` third of
//! [`super::rebuild_copy_learning`]'s copy pass — split into its own file
//! so that module stays under the 300-line ceiling.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::rebuild_copy::{old_column_exists, old_table_exists};

/// Copy the `feedback_events` verdict log (v5), the mined
/// `query_expansions` (v5), and the `bandit_arms` state. On pre-migration
/// sources `target_kind` (v6) defaults to `'memory'` and `provenance` (v8)
/// to `'manual'`, the same values those migrations backfill — dropping
/// either would let code verdicts masquerade as memory verdicts in the
/// harvest, or relabel implicit reinforcement as a user verdict.
pub(crate) fn copy_event_and_mined_tables(conn: &Connection) -> Result<()> {
    if old_table_exists(conn, "feedback_events")? {
        let target_expr = if old_column_exists(conn, "feedback_events", "target_kind")? {
            "target_kind"
        } else {
            "'memory'"
        };
        let prov_expr = if old_column_exists(conn, "feedback_events", "provenance")? {
            "provenance"
        } else {
            "'manual'"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.feedback_events(\
                 id, query_id, memory_id, verdict, at, target_kind, provenance) \
             SELECT id, query_id, memory_id, verdict, at, {target_expr}, {prov_expr} \
             FROM old.feedback_events;"
        ))?;
    }
    if old_table_exists(conn, "query_expansions")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.query_expansions(\
                 term, expansion, support, last_mined) \
             SELECT term, expansion, support, last_mined \
             FROM old.query_expansions;",
        )?;
    }
    if old_table_exists(conn, "bandit_arms")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.bandit_arms(\
                 arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags, \
                 alpha, beta, pulls, last_mrr, updated_at) \
             SELECT arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags, \
                 alpha, beta, pulls, last_mrr, updated_at \
             FROM old.bandit_arms;",
        )?;
    }
    Ok(())
}
