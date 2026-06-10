//! End-to-end memory search: route (candidates) → rerank (priors) →
//! diversify (dedup + MMR) → top-k, plus best-effort access tracking.

use rusqlite::Connection;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::rerank::Reranked;
use crate::retrieval::{diversify, rerank, router};

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
fn record_access(conn: &Connection, hits: &[Reranked]) {
    let now = match OffsetDateTime::now_utc().format(&Rfc3339) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "access tracking skipped: timestamp format failed");
            return;
        }
    };
    for hit in hits {
        if let Err(e) = conn.execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1 WHERE id = ?2",
            rusqlite::params![now, hit.memory_id],
        ) {
            tracing::warn!(error = %e, id = %hit.memory_id, "access tracking update failed");
        }
    }
}
