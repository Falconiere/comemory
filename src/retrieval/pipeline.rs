//! End-to-end memory search: route (candidates) → rerank (priors) →
//! diversify (dedup + MMR) → top-k, plus best-effort access tracking
//! and query logging (`retrieval_log`).

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::rerank::Reranked;
use crate::retrieval::{diversify, rerank, router};
use crate::store::memory_row;

/// Caller-facing knobs for one pipeline run.
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    /// Record access counts and write the `retrieval_log` row. CLI
    /// search/context set `true`; eval and tune set `false` so offline
    /// measurement cannot pollute its own training signal.
    pub track: bool,
}

/// Outcome of one pipeline run: the hits plus the logged query id
/// (`None` when `track` was off or logging failed best-effort).
#[derive(Debug)]
pub struct SearchRun {
    /// Final reranked + diversified hits.
    pub hits: Vec<Reranked>,
    /// Id of the `retrieval_log` row written for this run.
    pub query_id: Option<String>,
}

/// Run the full retrieval pipeline for a memory query. `kind` restricts
/// hits to one memory kind (canonical lowercase string, e.g. `decision`);
/// `None` searches every kind. With `opts.track` set, access counts are
/// bumped and the query is logged to `retrieval_log`.
pub fn search(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
    kind: Option<&str>,
    opts: SearchOptions,
) -> Result<SearchRun> {
    let started = std::time::Instant::now();
    let candidates = router::route(cfg, conn, query, vec, repo, kind)?;
    let reranked = rerank::rerank(conn, cfg, &candidates)?;
    let final_hits = diversify::diversify(
        reranked,
        cfg.rank.near_dup_hamming,
        cfg.rank.mmr_lambda,
        cfg.retrieval.top_k,
    );
    let query_id = if opts.track {
        record_telemetry(conn, query, &final_hits, started.elapsed())
    } else {
        None
    };
    Ok(SearchRun {
        hits: final_hits,
        query_id,
    })
}

/// Best-effort telemetry for one tracked run: bump access counts and
/// write the `retrieval_log` row inside ONE transaction, so the pair
/// costs a single WAL fsync instead of two. The contract stays
/// best-effort end to end — search never fails on telemetry: if the
/// transaction cannot be opened the two writes fall back to direct
/// autocommit calls, and if the commit fails both writes are dropped
/// with a warning and no `query_id` is reported.
fn record_telemetry(
    conn: &Connection,
    query: &str,
    hits: &[Reranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    match conn.unchecked_transaction() {
        Ok(tx) => {
            record_access(&tx, hits);
            let query_id = record_query(&tx, query, hits, elapsed);
            match tx.commit() {
                Ok(()) => query_id,
                Err(e) => {
                    tracing::warn!(error = %e, "telemetry commit failed; access counts and query log dropped");
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "telemetry transaction unavailable; falling back to direct writes");
            record_access(conn, hits);
            record_query(conn, query, hits, elapsed)
        }
    }
}

/// Bump access tracking for returned hits. Best-effort: a failure must
/// never break the read path.
///
/// All ids are folded into one `UPDATE ... WHERE id IN (...)` statement so
/// the bump costs a single statement and waits on `busy_timeout` at most
/// once — per-row statements could block once per hit. The WAL fsync is
/// shared with the `retrieval_log` write via [`record_telemetry`]'s
/// transaction. The timestamp goes through [`memory_row::iso_format`] so
/// every `last_accessed` writer emits the same string format as
/// `created_at` / `updated_at`.
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
    let qmarks = crate::store::qmarks(hits.len());
    let sql = format!(
        "UPDATE memories SET access_count = access_count + 1, last_accessed = ? \
         WHERE id IN ({qmarks})"
    );
    let params = std::iter::once(now.as_str()).chain(hits.iter().map(|h| h.memory_id.as_str()));
    if let Err(e) = conn.execute(&sql, rusqlite::params_from_iter(params)) {
        tracing::warn!(error = %e, hit_count = hits.len(), "access tracking update failed");
    }
}

/// Write the `retrieval_log` row for this run. Best-effort like
/// [`record_access`]: a logging failure warns and returns `None` —
/// the search result must never depend on telemetry.
fn record_query(
    conn: &Connection,
    query: &str,
    hits: &[Reranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    let now = OffsetDateTime::now_utc();
    let at = match memory_row::iso_format(now) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "query logging skipped: timestamp format failed");
            return None;
        }
    };
    let query_id = crate::stats::feedback::generate_query_id(query, now);
    let ids: Vec<&str> = hits.iter().map(|h| h.memory_id.as_str()).collect();
    let returned = match serde_json::to_string(&ids) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "query logging skipped: id serialization failed");
            return None;
        }
    };
    let dur = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    match conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![query_id, query, returned, at, dur],
    ) {
        Ok(_) => Some(query_id),
        Err(e) => {
            tracing::warn!(error = %e, "query logging failed");
            None
        }
    }
}
