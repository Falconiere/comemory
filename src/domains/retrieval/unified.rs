//! Unified retrieval across memory, code, and documents — the `comemory
//! find` core.
//!
//! `search` answers the memory domain and `search-code` the code domain,
//! each with a hit shape of its own. Neither can produce the console's
//! Search screen, which is one ranked list with All / Memories / Code tabs.
//! This module runs the three existing legs unchanged and fuses their
//! *reranked* rankings, so a single-domain `find` orders identically to
//! that domain's dedicated command.
//!
//! ## Pagination
//!
//! Every leg fetches a pool sized by the same [`pipeline::pool_size`] the
//! single-domain commands use, and [`pipeline::paginate`] is applied ONCE,
//! to the fused list. `total` is the fused in-window count — not a sum of
//! per-domain totals. The legs must share one `pool_size` call rather than
//! each sizing its own: RRF is prefix-stable, so growing every leg by the
//! same rule appends tail candidates without reordering the head, and
//! divergent pools would let a deeper page reorder a shallower one.

use crate::config::Config;
use crate::domains::retrieval::pipeline;
use crate::domains::retrieval::scope::{Domain, Filters};
use crate::domains::retrieval::{code_rerank, code_search, diversify, doc_route, rerank, router};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::memory_meta;
use crate::utilities::pagination::PageWindow;

/// Weighted fusion and the domain-tagged hit shape.
pub mod fuse_domains;

/// The per-domain filters, which narrow exactly one leg each and mean
/// nothing to the others — which is why they are grouped here rather than
/// added as fields on the shared [`Filters`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DomainFilters<'a> {
    /// Source language; narrows the CODE leg only.
    pub lang: Option<&'a str>,
    /// Git-style path globs; narrow the DOCUMENT leg only.
    pub path_globs: &'a [String],
}

/// One unified run's outcome.
pub struct UnifiedRun {
    /// The requested page of the fused ranking.
    pub hits: Vec<fuse_domains::UnifiedHit>,
    /// Whether in-window results exist beyond this page.
    pub has_more: bool,
    /// Fused in-window ranked count the page was sliced from.
    pub total: usize,
}

/// Everything one unified run is asked for, bundled so [`find`] and
/// [`run_legs`] take a query rather than seven positional arguments — the same
/// reason [`Filters`] and [`DomainFilters`] exist. `Copy`, like `Filters`: call
/// sites pass by value and keep reading fields.
#[derive(Debug, Clone, Copy)]
pub struct UnifiedQuery<'a> {
    /// Natural-language query text, run verbatim.
    pub text: &'a str,
    /// Caller-supplied dense vector, when the run is a BYO-vector one.
    pub vector: Option<&'a [f32]>,
    /// The filters every leg narrows by.
    pub filters: Filters<'a>,
    /// The per-leg narrowing `filters` has no place for.
    pub domain_filters: DomainFilters<'a>,
}

/// The three legs' own reranked rows for one run, before
/// [`fuse_domains::fuse`] flattens them into [`fuse_domains::UnifiedHit`] and
/// drops their passage text and version anchors.
///
/// Returned by [`run_legs`] for the offline benchmark
/// (`domains::learning::evaluation::benchmark_runner`), which needs each
/// candidate's text and content version to build a candidate observation.
/// [`find`] itself consumes the same value and is the only other caller.
pub struct LegRows {
    /// Memory hits, reranked and diversified.
    pub memory: Vec<rerank::Reranked>,
    /// Code hits, reranked and coalesced onto their parents.
    pub code: Vec<code_rerank::CodeReranked>,
    /// Document hits, in the leg's own BM25 order.
    pub documents: Vec<doc_route::DocHit>,
    /// The one [`pipeline::pool_size`] every leg was fetched at.
    pub pool: usize,
}

/// Run every in-scope leg and return its own rows, unfused.
///
/// `filters.domains` selects the legs; a domain left out is skipped entirely
/// rather than run and discarded. [`DomainFilters`] carries the per-leg
/// narrowing (`lang` for code, `path_globs` for documents) that the shared
/// [`Filters`] has no place for.
///
/// This is [`find`] minus fusion and pagination, extracted so a caller that
/// needs the legs' text (the offline benchmark) reads it off the same rows
/// `find` fuses, instead of running a second retrieval path that could drift.
pub fn run_legs(
    cfg: &Config,
    conn: &Connection,
    query: UnifiedQuery<'_>,
    window: PageWindow,
) -> Result<LegRows> {
    let filters = query.filters;
    let pool = pipeline::pool_size(window.offset, window.limit, cfg.retrieval.max_page_window);

    // Every leg is gated HERE, uniformly. `memory_leg` and `route_documents`
    // also refuse their own excluded domain internally — `route_documents` is
    // shared with the document-only search path and has to — but relying on
    // that made this read like a missing guard to three separate reviewers.
    // One visible shape for all three is worth the duplicated condition.
    let memory = if filters.domains.contains(Domain::Memory) {
        memory_leg(cfg, conn, query.text, query.vector, filters, pool)?
    } else {
        Vec::new()
    };
    let code = if filters.domains.contains(Domain::Code) {
        code_search::search_code_hits(
            cfg,
            conn,
            query.text,
            query.vector,
            filters.repo,
            query.domain_filters.lang,
            pool,
        )?
    } else {
        Vec::new()
    };
    let documents = if filters.domains.contains(Domain::Document) {
        doc_route::route_documents(
            conn,
            query.text,
            filters,
            query.domain_filters.path_globs,
            pool,
        )?
    } else {
        Vec::new()
    };
    Ok(LegRows {
        memory,
        code,
        documents,
        pool,
    })
}

/// Fuse one run's legs into the ranked list, reading the batched navigation
/// metadata the memory hits need on the way. Split out of [`find`] so the
/// offline benchmark fuses through this exact code rather than restating it.
///
/// Skipped outright on an empty memory leg: `fetch_meta` already returns an
/// empty map for an empty id list, so this is about saying so at the call site
/// rather than about the round trip.
pub fn fuse_legs(
    cfg: &Config,
    conn: &Connection,
    legs: LegRows,
) -> Result<Vec<fuse_domains::UnifiedHit>> {
    let meta = if legs.memory.is_empty() {
        std::collections::HashMap::new()
    } else {
        let ids: Vec<&str> = legs.memory.iter().map(|h| h.memory_id.as_str()).collect();
        memory_meta::fetch_meta(conn, &ids)?
    };
    Ok(fuse_domains::fuse(
        fuse_domains::Legs {
            memory: legs.memory,
            memory_meta: &meta,
            code: legs.code,
            documents: legs.documents,
        },
        cfg.retrieval.document_leg_weight,
        legs.pool,
        cfg.retrieval.rrf_k,
    ))
}

/// Run every in-scope leg and fuse: [`run_legs`], then [`fuse_legs`], then ONE
/// [`pipeline::paginate`] over the fused list.
pub fn find(
    cfg: &Config,
    conn: &Connection,
    query: UnifiedQuery<'_>,
    window: PageWindow,
) -> Result<UnifiedRun> {
    let ranked = fuse_legs(cfg, conn, run_legs(cfg, conn, query, window)?)?;
    let (hits, has_more, total) = pipeline::paginate(ranked, window, cfg.retrieval.max_page_window);
    Ok(UnifiedRun {
        hits,
        has_more,
        total,
    })
}

/// The memory leg: the same route → rerank → diversify chain
/// [`pipeline::search`] runs, minus its telemetry (which the caller owns,
/// so one `find` writes one `retrieval_log` row rather than one per leg).
fn memory_leg(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    filters: Filters<'_>,
    pool: usize,
) -> Result<Vec<rerank::Reranked>> {
    if !filters.domains.contains(Domain::Memory) {
        return Ok(Vec::new());
    }
    let candidates = router::route(cfg, conn, query, vec, filters, pool)?;
    let reranked = rerank::rerank(conn, cfg, &candidates, filters.scope.as_of_cutoff())?;
    Ok(diversify::diversify(
        reranked,
        cfg.rank.near_dup_hamming,
        cfg.rank.mmr_lambda,
        pool,
    ))
}

#[cfg(test)]
#[path = "tests/unified.rs"]
mod tests;
