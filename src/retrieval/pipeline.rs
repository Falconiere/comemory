//! End-to-end memory search: route (candidates) → rerank (priors) →
//! diversify (dedup + MMR) → top-k, plus best-effort access tracking.

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::rerank::Reranked;
use crate::retrieval::{diversify, rerank, router};
use crate::store::memory_row;

/// Run the full retrieval pipeline for a memory query.
pub fn search(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
) -> Result<Vec<Reranked>> {
    let candidates = router::route(cfg, conn, query, vec, repo)?;
    let reranked = rerank::rerank(conn, cfg, &candidates)?;
    let final_hits = diversify::diversify(reranked, cfg.rank.mmr_lambda, cfg.retrieval.top_k);
    record_access(conn, &final_hits);
    Ok(final_hits)
}

/// Bump access tracking for returned hits. Best-effort: a failure must
/// never break the read path.
///
/// All ids are folded into one `UPDATE ... WHERE id IN (...)` statement so
/// the bump costs a single autocommit transaction (one WAL fsync) and
/// waits on `busy_timeout` at most once — per-row statements would fsync
/// and potentially block once per hit. The timestamp goes through
/// [`memory_row::iso_format`] so every `last_accessed` writer emits the
/// same string format as `created_at` / `updated_at`.
fn record_access(conn: &Connection, hits: &[Reranked]) {
    if hits.is_empty() {
        return;
    }
    let now = match memory_row::iso_format(OffsetDateTime::now_utc()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "access tracking skipped: timestamp format failed");
            return;
        }
    };
    let qmarks = vec!["?"; hits.len()].join(",");
    let sql = format!(
        "UPDATE memories SET access_count = access_count + 1, last_accessed = ? \
         WHERE id IN ({qmarks})"
    );
    let params = std::iter::once(now.as_str()).chain(hits.iter().map(|h| h.memory_id.as_str()));
    if let Err(e) = conn.execute(&sql, rusqlite::params_from_iter(params)) {
        tracing::warn!(error = %e, hit_count = hits.len(), "access tracking update failed");
    }
}
