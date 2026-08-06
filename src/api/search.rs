//! `api::search::{Request, run}` — the shared middle of `comemory search` /
//! `GET|POST /api/v1/memories/search`: parse the time scope, route → rerank
//! → diversify → top-k via [`pipeline::search`], then batch-fetch
//! navigation metadata for the returned page. Moved out of
//! `cli::search::run_memory` (Binding Rule 1).
//!
//! Scoped to the memory domain: `--only document`'s interim path
//! (`cli::search_only`) is not part of this shared core (document domain
//! has not joined the unified pipeline yet — see that module's docs) and
//! stays CLI-only, so `Request` carries no `--only`/`--path` fields.

use serde::Deserialize;

use crate::api::Ctx;
use crate::cli::{page_meta, page_window, when};
use crate::memory::Kind;
use crate::output::search::SearchResult;
use crate::prelude::*;
use crate::retrieval::pipeline::{self, SearchOptions};
use crate::retrieval::scope::{Domains, Filters};
use crate::store::memory_meta;

/// `comemory search` / `GET|POST /api/v1/memories/search` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Natural-language query string.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`. `0` means
    /// "all remaining within the `max_page_window`".
    #[serde(default)]
    pub k: Option<usize>,
    /// Number of leading ranked results to skip (deep paging).
    #[serde(default)]
    pub offset: usize,
    /// Optional repo filter.
    #[serde(default)]
    pub repo: Option<String>,
    /// Filter results to one memory kind.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Caller-supplied dense vector, replacing `--vector`/`--vector-stdin`.
    /// `GET` requests never set this (a 1024-float embedding does not fit
    /// in a query string) — only the `POST` form is vector-capable.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    /// Only search memories created at or after this instant (RFC3339 or a
    /// bare `YYYY-MM-DD` date).
    #[serde(default)]
    pub since: Option<String>,
    /// Only search memories created at or before this instant. Filters
    /// candidates only — the supersede penalty stays present-day.
    #[serde(default)]
    pub until: Option<String>,
    /// Search the corpus as it stood at this instant: `until` plus
    /// supersede-penalty scoping. Mutually exclusive with `until`.
    #[serde(default)]
    pub as_of: Option<String>,
}

/// Run the shared memory-search middle. `track` governs access tracking +
/// `retrieval_log` writes — the CLI passes `cli::track_searches()`, a
/// read-only HTTP server passes `false` unconditionally (§Security
/// "Read-only side-effect degradation").
pub fn run(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<SearchResult> {
    // Copied out before `ctx.conn()` so the later mutable borrow of `ctx`
    // (for the connection) doesn't also lock out this field.
    let cfg = ctx.cfg;
    let scope = when::scope_from_flags(
        req.since.as_deref(),
        req.until.as_deref(),
        req.as_of.as_deref(),
    )?;
    let window = page_window(cfg, req.k, req.offset);
    let kind = req.kind.map(Kind::as_str);
    let filters = Filters {
        repo: req.repo.as_deref(),
        kind,
        scope: &scope,
        domains: Domains::all(),
    };
    let conn: &rusqlite::Connection = ctx.conn()?;
    let run = pipeline::search(
        cfg,
        conn,
        &req.query,
        req.vector.as_deref(),
        filters,
        SearchOptions {
            track,
            source: crate::stats::source::SEARCH,
            window,
        },
    )?;
    let meta = page_meta(window, run.has_more, run.total);
    let ids: Vec<&str> = run.hits.iter().map(|h| h.memory_id.as_str()).collect();
    let nav = memory_meta::fetch_meta(conn, &ids)?;
    Ok(SearchResult {
        hits: run.hits,
        query_id: run.query_id,
        meta,
        nav,
        scope,
    })
}
