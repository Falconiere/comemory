//! The `feedback_events`/`query_expansions`/`bandit_arms` third of
//! [`super::rebuild_copy_learning`]'s copy pass, plus the three
//! candidate-observation tables (#209) — split into its own file so that
//! module stays under the 300-line ceiling.

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
        // The v26 identity and origin columns: `NULL` from an older source,
        // which is what the migration leaves in every pre-existing row.
        let mut shared = Vec::new();
        for column in ["event_id", "device", "surface", "actor"] {
            shared.push(if old_column_exists(conn, "feedback_events", column)? {
                column
            } else {
                "NULL"
            });
        }
        let shared = shared.join(", ");
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.feedback_events(\
                 id, query_id, memory_id, verdict, at, target_kind, provenance, \
                 event_id, device, surface, actor) \
             SELECT id, query_id, memory_id, verdict, at, {target_expr}, {prov_expr}, {shared} \
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
    copy_candidate_observation_tables(conn)
}

/// Copy the candidate observations, their candidates and the reviewed
/// judgments resolved against them (v20).
///
/// These carry the strongest claim on this pass: a judgment is human review,
/// and the bounded passage it was made against is a snapshot of content that
/// may since have changed. A rebuild cannot rebuild either from markdown, the
/// code index, or anything else on disk. All three are guarded by
/// [`old_table_exists`] because a pre-v20 source database has none of them.
fn copy_candidate_observation_tables(conn: &Connection) -> Result<()> {
    if old_table_exists(conn, "candidate_query_observations")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.candidate_query_observations(\
                 observation_id, observation_version, query_id, query, source, \
                 filters_json, retrieval_json, knobs_hash, corpus_digest, decay_frozen, \
                 pool_size, page_limit, page_offset, candidate_count, truncated, at) \
             SELECT observation_id, observation_version, query_id, query, source, \
                 filters_json, retrieval_json, knobs_hash, corpus_digest, decay_frozen, \
                 pool_size, page_limit, page_offset, candidate_count, truncated, at \
             FROM old.candidate_query_observations;",
        )?;
    }
    if old_table_exists(conn, "candidate_observations")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.candidate_observations(\
                 observation_id, pool_position, domain, candidate_ref, content_version, \
                 unresolved, returned_position, retrieval_score, rank_in_domain, tier, \
                 text, text_sha256, text_full_bytes, text_truncated, locator_json) \
             SELECT observation_id, pool_position, domain, candidate_ref, content_version, \
                 unresolved, returned_position, retrieval_score, rank_in_domain, tier, \
                 text, text_sha256, text_full_bytes, text_truncated, locator_json \
             FROM old.candidate_observations;",
        )?;
    }
    if old_table_exists(conn, "candidate_judgments")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.candidate_judgments(\
                 observation_id, candidate_ref, domain, relevance, provenance, at) \
             SELECT observation_id, candidate_ref, domain, relevance, provenance, at \
             FROM old.candidate_judgments;",
        )?;
    }
    Ok(())
}
