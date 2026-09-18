//! `retrieval::search::{Request, run}` — the shared middle of `comemory search` /
//! `GET|POST /api/v1/memories/search`: parse the time scope, route → rerank
//! → diversify via [`pipeline::rank`], optionally reorder through the learned
//! stage, then slice the page and batch-fetch its navigation metadata.
//! Moved out of `cli::search::run_memory` (Binding Rule 1).
//!
//! Scoped to the memory domain: `--only document`'s interim path
//! (`cli::search_only`) is not part of this shared core (document domain
//! has not joined the unified pipeline yet — see that module's docs) and
//! stays CLI-only, so `Request` carries no `--only`/`--path` fields.

use std::time::Instant;

use serde::Deserialize;

use crate::domains::memories::Kind;
use crate::domains::retrieval::learned_report::LearnedOrdering;
use crate::domains::retrieval::learned_rerank::{self, Candidates, LearnedStage};
use crate::domains::retrieval::pipeline::{self, SearchOptions};
use crate::domains::retrieval::rerank::Reranked;
use crate::domains::retrieval::scope::{self, Domains, Filters, TimeScope};
use crate::domains::retrieval::search_result::SearchResult;
use crate::domains::retrieval::staged::{FinishStep, Paused, Staged, resolve};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::memory_meta;
use crate::utilities::context::Ctx;
use crate::utilities::pagination::{page_meta, page_window};

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

/// The filters this request narrows by. Rebuilt rather than carried, because
/// `Filters` borrows its `TimeScope` and the phase-three continuation owns one.
fn filters_of<'a>(req: &'a Request, scope: &'a TimeScope) -> Filters<'a> {
    Filters {
        repo: req.repo.as_deref(),
        kind: req.kind.map(Kind::as_str),
        scope,
        domains: Domains::all(),
    }
}

/// Run the shared memory-search middle. `track` governs access tracking +
/// `retrieval_log` writes — the CLI passes `cli::track_searches()`, a
/// read-only HTTP server passes `false` unconditionally (§Security
/// "Read-only side-effect degradation").
pub fn run(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<SearchResult> {
    let staged = begin(ctx, req, track)?;
    resolve(ctx, staged)
}

/// [`run`], stopping at the learned ordering stage.
///
/// `Staged::Ready` when no stage is configured — the default, and the case that
/// must cost nothing. `Staged::Paused` carries a scoring call that borrows no
/// connection, so `comemory serve` can release its shared lock before the model
/// runs.
pub fn begin(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<Staged<SearchResult>> {
    let cfg = ctx.cfg;
    let scope = scope::scope_from_flags(
        req.since.as_deref(),
        req.until.as_deref(),
        req.as_of.as_deref(),
    )?;
    let window = page_window(cfg, req.k, req.offset);
    let opts = SearchOptions {
        track,
        source: crate::utilities::telemetry::source::SEARCH,
        window,
    };
    let started = Instant::now();
    let pool = pipeline::candidate_pool(cfg, window);
    let stage = LearnedStage::from_config(cfg);
    let conn: &Connection = ctx.conn()?;
    let ranked = pipeline::rank(
        cfg,
        conn,
        &req.query,
        req.vector.as_deref(),
        filters_of(&req, &scope),
        pool,
    )?;
    let keys = learned_rerank::memory_keys(&ranked);
    let call = learned_rerank::plan_call(
        stage.as_ref(),
        conn,
        &req.query,
        Candidates {
            memory: &ranked,
            keys: &keys,
            ..Candidates::default()
        },
    )?;
    let Some(call) = call else {
        return Ok(Staged::Ready(finish(
            ctx, req, scope, opts, started, ranked, None,
        )?));
    };
    let plan = call.plan();
    Ok(Staged::Paused(Paused::new(
        call,
        FinishStep::new(move |ctx, outcome| {
            let (ranked, learned) = learned_rerank::apply(ranked, &plan, &outcome);
            finish(ctx, req, scope, opts, started, ranked, Some(learned))
        }),
    )))
}

/// Slice the page, record telemetry, and read the page's navigation metadata.
fn finish(
    ctx: &mut Ctx<'_>,
    req: Request,
    scope: TimeScope,
    opts: SearchOptions,
    started: Instant,
    ranked: Vec<Reranked>,
    learned: Option<LearnedOrdering>,
) -> Result<SearchResult> {
    let cfg = ctx.cfg;
    let conn: &Connection = ctx.conn()?;
    let run = pipeline::complete(
        cfg,
        conn,
        &req.query,
        filters_of(&req, &scope),
        opts,
        ranked,
        started,
    );
    let meta = page_meta(opts.window, run.has_more, run.total);
    let ids: Vec<&str> = run.hits.iter().map(|h| h.memory_id.as_str()).collect();
    let nav = memory_meta::fetch_meta(conn, &ids)?;
    Ok(SearchResult {
        hits: run.hits,
        query_id: run.query_id,
        meta,
        nav,
        scope,
        learned,
    })
}
