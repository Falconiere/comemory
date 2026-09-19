//! End-to-end memory search: route (candidates) → rerank (priors) →
//! diversify (dedup + MMR) → top-k, plus best-effort access tracking
//! and query logging (`retrieval_log`).

use time::OffsetDateTime;

use crate::config::Config;
use crate::domains::retrieval::rerank::Reranked;
use crate::domains::retrieval::router::CANDIDATE_POOL;
use crate::domains::retrieval::scope::Filters;
use crate::domains::retrieval::{diversify, rerank, router};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::retrieval_log::{self, NewLogRow};
use crate::store::{memory_row, memory_signals};
use crate::utilities::pagination::PageWindow;

/// Caller-facing knobs for one pipeline run.
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    /// Record access counts and write the `retrieval_log` row. CLI
    /// search/context set `true`; eval and tune set `false` so offline
    /// measurement cannot pollute its own training signal.
    ///
    /// The two writes are not symmetric once `window` leaves the head of the
    /// ranked list: a tracked page past the head still writes its
    /// `retrieval_log` row — so `comemory feedback <query_id>` reaches a
    /// result found on page three — but bumps no access counts. See
    /// [`search`] for why.
    pub track: bool,
    /// Query origin written verbatim to `retrieval_log.source` — one of
    /// the `crate::utilities::telemetry::source` consts (`SEARCH`, `CONTEXT`,
    /// `SEARCH_CODE`). Reformulation mining excludes `search-code` rows,
    /// which can only earn code-target feedback.
    pub source: &'static str,
    /// The `(offset, limit)` page of the bounded ranked window to return.
    /// Use [`PageWindow::top_k`] for the unpaginated first-page default.
    pub window: PageWindow,
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

/// Run the full retrieval pipeline for a memory query. `filters` narrows
/// candidates by repo, memory kind (canonical lowercase string, e.g.
/// `decision`), and created-date scope — [`Filters::none`] searches
/// everything. `opts.window` selects the `(offset, limit)` page of the
/// bounded ranked window (use [`PageWindow::top_k`] for the unpaginated
/// default). With `opts.track` set, the returned page is logged to
/// `retrieval_log` at every offset, but its access counts are bumped only
/// when the window starts at the head ([`PageWindow::is_head`]) — see
/// [`record_access`] for why only a prefix may be reinforced (#201).
///
/// An `--as-of` scope additionally reaches the rerank stage, where it
/// limits the supersede penalty to superseders that existed at the cutoff;
/// a plain `--until` filters candidates only.
pub fn search(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    filters: Filters<'_>,
    opts: SearchOptions,
) -> Result<SearchRun> {
    let started = std::time::Instant::now();
    let pool = pool_size(
        opts.window.offset,
        opts.window.limit,
        cfg.retrieval.max_page_window,
    );
    let ranked = rank(cfg, conn, query, vec, filters, pool)?;
    Ok(complete(cfg, conn, query, filters, opts, ranked, started))
}

/// The deterministic ranking for a memory query: route → rerank → diversify,
/// cut at `pool`.
///
/// The first half of [`search`], split out so a surface can insert the optional
/// learned ordering stage between the ranking and the page
/// (`retrieval::learned_rerank`). Diversification runs over the WHOLE pool (cut
/// at `pool`, not `top_k`) so the full ranked window is materialized before
/// anything slices it.
pub fn rank(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    filters: Filters<'_>,
    pool: usize,
) -> Result<Vec<Reranked>> {
    let candidates = router::route(cfg, conn, query, vec, filters, pool)?;
    let reranked = rerank::rerank(conn, cfg, &candidates, filters.scope.as_of_cutoff())?;
    Ok(diversify::diversify(
        reranked,
        cfg.rank.near_dup_hamming,
        cfg.rank.mmr_lambda,
        pool,
    ))
}

/// Slice a final memory ranking to `opts.window` and record best-effort
/// telemetry. The second half of [`search`], and the last step of every
/// memory-domain surface whether or not a learned stage reordered `ranked`.
///
/// `started` is the caller's, not this function's, so the logged duration
/// covers the whole request — including any model call between [`rank`] and
/// here.
pub fn complete(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    filters: Filters<'_>,
    opts: SearchOptions,
    ranked: Vec<Reranked>,
    started: std::time::Instant,
) -> SearchRun {
    let (page, has_more, total) = paginate(ranked, opts.window, cfg.retrieval.max_page_window);
    let query_id = if opts.track {
        record_telemetry(
            conn,
            query,
            filters.repo,
            filters.kind,
            opts,
            &page,
            started.elapsed(),
        )
    } else {
        None
    };
    SearchRun {
        hits: page,
        query_id,
        has_more,
        total,
    }
}

/// The candidate universe one run builds.
///
/// With a learned ordering stage active this is the configured maximum window,
/// independent of the requested page: a neural scorer is not prefix-stable, so
/// a page-proportional pool would let a deeper page rewrite a shallower one by
/// admitting a candidate that scores above the current head. Without a stage it
/// is [`pool_size`], unchanged.
pub fn candidate_pool(cfg: &Config, window: PageWindow) -> usize {
    if cfg.rerank.enabled {
        return cfg.retrieval.max_page_window.max(1);
    }
    pool_size(window.offset, window.limit, cfg.retrieval.max_page_window)
}

/// Best-effort telemetry for one tracked run: bump access counts and
/// write the `retrieval_log` row inside ONE transaction, so the pair
/// costs a single WAL fsync instead of two. The two writes have different
/// scopes: the log always covers the returned page, while the bump covers
/// it only on a head window ([`bumped_ids`]).
///
/// The contract stays best-effort end to end — search never fails on
/// telemetry: if the transaction cannot be opened the two writes fall back
/// to direct autocommit calls, and if the commit fails both writes are
/// dropped with a warning and no `query_id` is reported.
///
/// Takes the whole [`SearchOptions`] rather than `source` plus a window
/// because an eighth parameter would exceed `clippy::too_many_arguments`.
fn record_telemetry(
    conn: &Connection,
    query: &str,
    repo: Option<&str>,
    kind: Option<&str>,
    opts: SearchOptions,
    hits: &[Reranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    let source = opts.source;
    match conn.unchecked_transaction() {
        Ok(tx) => {
            record_access(&tx, &bumped_ids(opts.window, hits));
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
            record_access(conn, &bumped_ids(opts.window, hits));
            record_query(conn, query, repo, kind, source, hits, elapsed)
        }
    }
}

/// The ids this run may reinforce: every hit on a head window, none at all
/// once `window` skips past the head. Returning an empty slice rather than
/// branching at the call site reuses [`record_access`]'s own empty-input
/// guard, so there is exactly one place that decides "nothing to bump".
fn bumped_ids(window: PageWindow, hits: &[Reranked]) -> Vec<&str> {
    if window.is_head() {
        hits.iter().map(|h| h.memory_id.as_str()).collect()
    } else {
        Vec::new()
    }
}

/// Bump access tracking for returned hits. Best-effort: a failure must
/// never break the read path.
///
/// Search-path callers must pass a PREFIX of the ranked window, never a band
/// out of its middle. Activation only grows with `access_count` and recency,
/// so a bump weakly raises exactly the rows it touches: a prefix is a fixed
/// point of its own reinforcement, while a gapped set floats above the rows
/// it was behind and the next identical query returns a different band
/// (#201). This constrains the search path only — `graph::coactivate` bumps
/// co-activated memories outside any ranking and is not bound by it.
///
/// All ids are folded into one `UPDATE ... WHERE id IN (...)` statement so
/// the bump costs a single statement and waits on `busy_timeout` at most
/// once — per-row statements could block once per hit. The WAL fsync is
/// shared with the `retrieval_log` write via [`record_telemetry`]'s
/// transaction. The timestamp goes through [`memory_row::iso_format`] so
/// every `last_accessed` writer emits the same string format as
/// `created_at` / `updated_at`.
pub(crate) fn record_access(conn: &Connection, ids: &[&str]) {
    if ids.is_empty() {
        return;
    }
    let now = match memory_row::iso_format(OffsetDateTime::now_utc()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "access tracking skipped: timestamp format failed");
            return;
        }
    };
    let owned: Vec<String> = ids.iter().map(|id| (*id).to_string()).collect();
    if let Err(e) = memory_signals::bump_access(conn, &owned, &now) {
        tracing::warn!(error = %e, hit_count = ids.len(), "access tracking update failed");
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
/// and the query `source` (a `crate::utilities::telemetry::source` const). Best-effort
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
    let query_id = crate::utilities::query_id::generate_query_id(query, now);
    let returned = match serde_json::to_string(returned_ids) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "query logging skipped: id serialization failed");
            return None;
        }
    };
    let dur = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    match retrieval_log::insert(
        conn,
        &NewLogRow {
            query_id: &query_id,
            query,
            returned_ids: &returned,
            at: &at,
            duration_ms: dur,
            repo,
            kind,
            source,
        },
    ) {
        Ok(()) => Some(query_id),
        Err(e) => {
            tracing::warn!(error = %e, "query logging failed");
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/pipeline.rs"]
mod tests;
