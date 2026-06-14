//! End-to-end memory search: route (candidates) → rerank (priors) →
//! diversify (dedup + MMR) → top-k, plus best-effort access tracking
//! and query logging (`retrieval_log`).

use rusqlite::Connection;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::rerank::Reranked;
use crate::retrieval::router::CANDIDATE_POOL;
use crate::retrieval::{diversify, rerank, router};
use crate::store::memory_row;

/// Caller-facing knobs for one pipeline run.
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    /// Record access counts and write the `retrieval_log` row. CLI
    /// search/context set `true`; eval and tune set `false` so offline
    /// measurement cannot pollute its own training signal.
    pub track: bool,
    /// Query origin written verbatim to `retrieval_log.source` — one of
    /// the [`crate::stats::source`] consts (`SEARCH`, `CONTEXT`,
    /// `SEARCH_CODE`). Reformulation mining excludes `search-code` rows,
    /// which can only earn code-target feedback.
    pub source: &'static str,
    /// The `(offset, limit)` page of the bounded ranked window to return.
    /// Use [`PageWindow::top_k`] for the unpaginated first-page default.
    pub window: PageWindow,
}

/// The `(offset, limit)` slice a paginated retrieval should return from the
/// bounded ranked window. `limit == 0` is the "page size = remaining within
/// the window" sentinel (mirrors the shared [`crate::output::page::Page`]
/// "all" rule, bounded here by `max_page_window`).
#[derive(Debug, Clone, Copy)]
pub struct PageWindow {
    /// Leading ranked results to skip before the page starts.
    pub offset: usize,
    /// Page size; `0` means "everything remaining within the window".
    pub limit: usize,
}

impl PageWindow {
    /// The full first page sized to `top_k` — the unpaginated default that
    /// reproduces the pre-pagination behavior (`offset = 0`, `limit =
    /// top_k`).
    pub fn top_k(cfg: &Config) -> Self {
        Self {
            offset: 0,
            limit: cfg.retrieval.top_k,
        }
    }
}

/// Candidate-pool size for paging into a ranked result list with this
/// window: `clamp(offset + limit + buffer, CANDIDATE_POOL, max_window)`.
///
/// The `+ buffer` (one extra page, `limit`) headroom keeps near-dup /
/// MMR boundary collapse from truncating the requested page — a candidate
/// dropped during diversification must not shorten the slice. A `limit ==
/// 0` ("all within the window") request fetches the whole `max_window`.
///
/// Stability rests on this being a *prefix* fetch: RRF rank-fusion and
/// MMR/near-dup selection keep their top prefix stable as the pool grows
/// (adding lower-ranked tail candidates never reorders the higher-ranked
/// head), so paging deeper (a larger pool) does not shift earlier pages.
pub fn pool_size(offset: usize, limit: usize, max_window: usize) -> usize {
    let max_window = max_window.max(1);
    if limit == 0 {
        return max_window;
    }
    let want = offset
        .saturating_add(limit)
        .saturating_add(limit)
        .min(max_window);
    want.clamp(CANDIDATE_POOL.min(max_window), max_window)
}

/// Slice `ranked` to the `window` and report whether more in-window
/// results exist. Returns `(page, has_more, total)`:
/// - `total` is the in-window ranked count (`ranked.len()`), capped by
///   `max_window` — **not** a global match count.
/// - `has_more` is `true` iff ranked results exist beyond `offset + limit`
///   *and* that boundary is still inside `max_window`; once the window
///   ceiling is reached `has_more` is `false` (deeper results require
///   refining the query).
/// - `limit == 0` returns everything from `offset` onward (within the
///   window) with `has_more = false`.
pub fn paginate<T>(ranked: Vec<T>, window: PageWindow, max_window: usize) -> (Vec<T>, bool, usize) {
    let total = ranked.len();
    let start = window.offset.min(total);
    let mut page: Vec<T> = ranked.into_iter().skip(start).collect();
    let has_more = if window.limit == 0 {
        false
    } else {
        if page.len() > window.limit {
            page.truncate(window.limit);
        }
        let end = window.offset.saturating_add(window.limit);
        end < total && end < max_window
    };
    (page, has_more, total)
}

/// Outcome of one pipeline run: the page of hits plus the logged query id
/// (`None` when `track` was off or logging failed best-effort) and the
/// window metadata describing the slice.
#[derive(Debug)]
pub struct SearchRun {
    /// Final reranked + diversified hits for the requested page.
    pub hits: Vec<Reranked>,
    /// Id of the `retrieval_log` row written for this run.
    pub query_id: Option<String>,
    /// Whether in-window ranked results exist beyond this page.
    pub has_more: bool,
    /// In-window ranked count (diversified): the size of the ranked list
    /// the page was sliced from, capped by `max_page_window`. Not a global
    /// match count.
    pub total: usize,
}

/// Run the full retrieval pipeline for a memory query. `kind` restricts
/// hits to one memory kind (canonical lowercase string, e.g. `decision`);
/// `None` searches every kind. `opts.window` selects the `(offset, limit)`
/// page of the bounded ranked window (use [`PageWindow::top_k`] for the
/// unpaginated default). With `opts.track` set, access counts are bumped
/// and the query is logged to `retrieval_log` — for the RETURNED page only.
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
    let window = opts.window;
    let max_window = cfg.retrieval.max_page_window;
    let pool = pool_size(window.offset, window.limit, max_window);
    let candidates = router::route(cfg, conn, query, vec, repo, kind, pool)?;
    let reranked = rerank::rerank(conn, cfg, &candidates)?;
    // Diversify over the WHOLE pool (cut at `pool`, not `top_k`) so the
    // full ranked window is materialized before the page is sliced.
    let ranked = diversify::diversify(
        reranked,
        cfg.rank.near_dup_hamming,
        cfg.rank.mmr_lambda,
        pool,
    );
    let (page, has_more, total) = paginate(ranked, window, max_window);
    let query_id = if opts.track {
        record_telemetry(
            conn,
            query,
            repo,
            kind,
            opts.source,
            &page,
            started.elapsed(),
        )
    } else {
        None
    };
    Ok(SearchRun {
        hits: page,
        query_id,
        has_more,
        total,
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
    repo: Option<&str>,
    kind: Option<&str>,
    source: &'static str,
    hits: &[Reranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    match conn.unchecked_transaction() {
        Ok(tx) => {
            record_access(&tx, hits);
            let query_id = record_query(&tx, query, repo, kind, source, hits, elapsed);
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
            record_query(conn, query, repo, kind, source, hits, elapsed)
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

/// Thin id-mapping wrapper over [`log_retrieval`] for memory hits.
fn record_query(
    conn: &Connection,
    query: &str,
    repo: Option<&str>,
    kind: Option<&str>,
    source: &'static str,
    hits: &[Reranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    let ids: Vec<String> = hits.iter().map(|h| h.memory_id.clone()).collect();
    log_retrieval(conn, query, &ids, elapsed, repo, kind, source)
}

/// The single `retrieval_log` writer, shared by memory searches (via
/// [`record_query`]) and code searches (`cli::search_code`, which
/// text-encodes its symbol ids so the `returned_ids` column shape matches
/// the memory rows). Logs the repo/kind filters the caller searched with
/// (verbatim, `None` → NULL; `kind` carries `--lang` for code searches)
/// and the query `source` (a [`crate::stats::source`] const). Best-effort
/// like [`record_access`]: a logging failure warns and returns `None` —
/// the search result must never depend on telemetry.
pub(crate) fn log_retrieval(
    conn: &Connection,
    query: &str,
    returned_ids: &[String],
    elapsed: std::time::Duration,
    repo: Option<&str>,
    kind: Option<&str>,
    source: &'static str,
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
    let returned = match serde_json::to_string(returned_ids) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "query logging skipped: id serialization failed");
            return None;
        }
    };
    let dur = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    match conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms,
                                   repo, kind, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![query_id, query, returned, at, dur, repo, kind, source],
    ) {
        Ok(_) => Some(query_id),
        Err(e) => {
            tracing::warn!(error = %e, "query logging failed");
            None
        }
    }
}
