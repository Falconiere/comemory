//! `api::search_code::{Request, run}` — the shared middle of `comemory
//! search-code` / `GET|POST /api/v1/code/search`: validate `lang`, route +
//! rerank via [`code_search::search_code_hits`], paginate, and record
//! best-effort telemetry. Moved out of `cli::search_code::run` (Binding
//! Rule 1).
//!
//! The CLI's lazy-reindex trigger (`cli::lazy_reindex`) does **not** move
//! here — it stays a CLI-only affordance (spec Non-Goal 8: HTTP callers
//! reindex explicitly via `POST /api/v1/code/index`).

use rusqlite::Connection;
use serde::Deserialize;

use crate::api::Ctx;
use crate::ast::languages::{self, Lang};
use crate::cli::{page_meta, page_window};
use crate::output::search_code::SearchCodeResult;
use crate::prelude::*;
use crate::retrieval::code_rerank::CodeReranked;
use crate::retrieval::{code_search, pipeline};
use crate::store::code_row;

/// `comemory search-code` / `GET|POST /api/v1/code/search` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Natural-language or identifier query string.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`. `0` means
    /// "all remaining within the `max_page_window`".
    #[serde(default)]
    pub k: Option<usize>,
    /// Number of leading ranked results to skip (deep paging).
    #[serde(default)]
    pub offset: usize,
    /// Restrict hits to one repo label (as passed to `index-code --repo`).
    #[serde(default)]
    pub repo: Option<String>,
    /// Restrict hits to one language: `rust`, `typescript`, `javascript`,
    /// `python`, `go` (short aliases like `rs`/`ts`/`py` accepted).
    #[serde(default)]
    pub lang: Option<String>,
    /// Caller-supplied dense vector, replacing `--vector`/`--vector-stdin`.
    /// `GET` requests never set this (a 768-float embedding does not fit
    /// in a query string) — only the `POST` form is vector-capable.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
}

/// Run the shared code-search middle: validate `lang`, route + rerank +
/// cut to the requested page via [`code_search::search_code_hits`] and
/// [`pipeline::paginate`], and record best-effort telemetry (access bump +
/// `retrieval_log` row, `source='search-code'`) when `track` is set.
/// `track` mirrors `api::search::run`'s CLI/HTTP split — the CLI passes
/// `cli::track_searches()`, a read-only HTTP server passes `false`
/// unconditionally (§Security "Read-only side-effect degradation").
pub fn run(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<SearchCodeResult> {
    let lang = canonical_lang(req.lang.as_deref())?;
    let cfg = ctx.cfg;
    let window = page_window(cfg, req.k, req.offset);
    let max_window = cfg.retrieval.max_page_window;
    let pool = pipeline::pool_size(window.offset, window.limit, max_window);
    let started = std::time::Instant::now();
    let conn: &Connection = ctx.conn()?;
    let ranked = code_search::search_code_hits(
        cfg,
        conn,
        &req.query,
        req.vector.as_deref(),
        req.repo.as_deref(),
        lang,
        pool,
    )?;
    let (hits, has_more, total) = pipeline::paginate(ranked, window, max_window);
    let query_id = if track {
        record_code_telemetry(
            conn,
            &req.query,
            req.repo.as_deref(),
            lang,
            &hits,
            started.elapsed(),
        )
    } else {
        None
    };
    // The empty-index probe only matters for the zero-hit TTY hint, so it
    // is skipped entirely whenever there are hits.
    let index_empty = hits.is_empty() && !code_index_populated(conn)?;
    let meta = page_meta(window, has_more, total);
    Ok(SearchCodeResult {
        hits,
        query_id,
        meta,
        index_empty,
    })
}

/// Validate and canonicalize the `lang` field through [`Lang::parse`], so
/// aliases (`rs`, `py`, ...) match the canonical names stored in
/// `code_symbols.lang` instead of silently filtering everything out. The
/// canonical form is also what lands in `retrieval_log.kind`. Moved
/// verbatim from `cli::search_code` (Binding Rule 1).
fn canonical_lang(raw: Option<&str>) -> Result<Option<&'static str>> {
    match raw {
        None => Ok(None),
        Some(s) => Lang::parse(s).map(|l| Some(l.as_str())).ok_or_else(|| {
            Error::Config(format!(
                "unsupported lang {:?}; supported: {}",
                s,
                languages::supported().join(", ")
            ))
        }),
    }
}

/// `true` when at least one `code_symbols` row exists — distinguishes
/// "query missed" from "nothing was ever indexed" for the TTY hint.
fn code_index_populated(conn: &Connection) -> Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM code_symbols)", [], |r| {
        r.get(0)
    })
    .map_err(Error::from)
}

/// Best-effort telemetry for one tracked code search, the code-side
/// sibling of `retrieval::pipeline::record_telemetry` (same one-tx /
/// fall-back-to-autocommit / never-fail contract; the tx scaffold is
/// deliberately a thin local twin rather than a shared closure-taking
/// helper — the two access-bump + log sequences differ in target table
/// and id mapping, and a generic scaffold would contort both callers):
/// bump `access_count`/`last_accessed` on the returned (parent) symbol
/// ids and insert the `retrieval_log` row in a single transaction via the
/// shared [`pipeline::log_retrieval`] writer, with the symbol ids
/// text-encoded so the `returned_ids` column shape matches the memory
/// rows and the `repo`/`kind` columns carrying the `repo`/`lang` filters
/// verbatim. Returns the logged query id, or `None` when logging failed.
fn record_code_telemetry(
    conn: &Connection,
    query: &str,
    repo: Option<&str>,
    lang: Option<&str>,
    hits: &[CodeReranked],
    elapsed: std::time::Duration,
) -> Option<String> {
    let ids: Vec<String> = hits.iter().map(|h| h.symbol_id.to_string()).collect();
    let log = |conn: &Connection| {
        pipeline::log_retrieval(
            conn,
            query,
            &ids,
            elapsed,
            repo,
            lang,
            crate::stats::source::SEARCH_CODE,
        )
    };
    match conn.unchecked_transaction() {
        Ok(tx) => {
            record_code_access(&tx, hits);
            let query_id = log(&tx);
            match tx.commit() {
                Ok(()) => query_id,
                Err(e) => {
                    tracing::warn!(error = %e, "code telemetry commit failed; access counts and query log dropped");
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "code telemetry transaction unavailable; falling back to direct writes");
            record_code_access(conn, hits);
            log(conn)
        }
    }
}

/// Bump `code_symbols.access_count`/`last_accessed` for the returned
/// symbol ids (the coalesced parent ids — the feedback-able identities)
/// via the shared [`code_row::record_access`] writer, so `search-code`
/// and `context` cannot drift on the bump SQL or its best-effort
/// contract.
fn record_code_access(conn: &Connection, hits: &[CodeReranked]) {
    let ids: Vec<i64> = hits.iter().map(|h| h.symbol_id).collect();
    code_row::record_access(conn, &ids);
}
