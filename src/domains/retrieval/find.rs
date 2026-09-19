//! `retrieval::find::{Request, run}` — the shared middle of `comemory find` /
//! `GET|POST /api/v1/find`: one ranked list across memory, code, and
//! documents.
//!
//! Distinct from `retrieval::search` (memory only) and `retrieval::search_code` (code
//! only), which keep their domain-specific hit shapes. A single-domain
//! `find` orders identically to the matching dedicated command — see
//! `retrieval::unified` for why.

use serde::Deserialize;

use crate::config::Config;
use crate::domains::learning::evaluation::candidate_facts::{self, FactsByHit};
use crate::domains::learning::observation_capture::{self, CaptureInput};
use crate::domains::memories::Kind;
use crate::domains::retrieval::pipeline;
use crate::domains::retrieval::scope::{self, Domain, Domains, Filters};
use crate::domains::retrieval::unified::{self, fuse_domains::UnifiedHit};
use crate::prelude::*;
use crate::store::Connection;
use crate::utilities::context::Ctx;
use crate::utilities::pagination::PageMeta;
use crate::utilities::pagination::{PageWindow, page_meta, page_window};

/// `comemory find` / `GET|POST /api/v1/find` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Natural-language query string.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`.
    #[serde(default)]
    pub k: Option<usize>,
    /// Ranked results to skip (deep paging).
    #[serde(default)]
    pub offset: usize,
    /// Restrict to one domain: `all` (default), `memory`, `code`, or
    /// `document`.
    #[serde(default)]
    pub domain: Option<String>,
    /// Repo filter. Narrows the memory and code legs.
    #[serde(default)]
    pub repo: Option<String>,
    /// Memory-kind filter. Narrows the memory leg only.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Language filter. Narrows the code leg only.
    #[serde(default)]
    pub lang: Option<String>,
    /// Document path globs. Narrow the document leg only.
    #[serde(default)]
    pub path: Vec<String>,
    /// Caller-supplied dense vector (`POST` only — an embedding does not
    /// fit in a query string).
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    /// Only consider memories created at or after this instant.
    #[serde(default)]
    pub since: Option<String>,
    /// Only consider memories created at or before this instant.
    #[serde(default)]
    pub until: Option<String>,
    /// Search the corpus as it stood at this instant.
    #[serde(default)]
    pub as_of: Option<String>,
}

/// Everything the render layer needs from one `find` run.
#[derive(Debug)]
pub struct FindResult {
    /// The fused page.
    pub hits: Vec<UnifiedHit>,
    /// `retrieval_log` row id for this run, when tracking was on.
    pub query_id: Option<String>,
    /// `candidate_query_observations` row id for this run, when candidate
    /// capture was armed AND succeeded. `None` covers all three of "capture is
    /// off", "this run may not write telemetry", and "the capture failed" —
    /// deliberately one value, because a caller's response to each is the
    /// same: there is no observation to judge against.
    pub observation_id: Option<String>,
    /// Pagination cursor.
    pub meta: PageMeta,
}

/// Resolve `domain` into the leg selection. An unknown value is a usage
/// error naming the offender rather than a silent fall-through to `all`.
fn domains_of(domain: Option<&str>) -> Result<Domains> {
    match domain.unwrap_or("all") {
        "all" => Ok(Domains::all()),
        "memory" => Ok(Domains::of(&[Domain::Memory])),
        "code" => Ok(Domains::of(&[Domain::Code])),
        "document" => Ok(Domains::of(&[Domain::Document])),
        other => Err(Error::Usage(format!(
            "unknown --domain {other}: expected all, memory, code, or document"
        ))),
    }
}

/// Run the unified query. `track` governs the `retrieval_log` write, the
/// per-domain access bumps, and — with `observations.enabled` — whether the
/// candidate pool is captured, exactly as it does for `search`.
///
/// The three published steps of `unified::find` are composed here rather than
/// called through it, because capture needs each leg's rows *before*
/// `fuse_legs` drops their passage text, and the fused pool *before*
/// `paginate` cuts it to a page. Both capture calls are conditional; the
/// retrieval path itself is not, so there is one ordering through `find` and
/// not two.
pub fn run(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<FindResult> {
    let cfg = ctx.cfg;
    let scope = scope::scope_from_flags(
        req.since.as_deref(),
        req.until.as_deref(),
        req.as_of.as_deref(),
    )?;
    let window = page_window(cfg, req.k, req.offset);
    let domains = domains_of(req.domain.as_deref())?;
    let kind = req.kind.map(Kind::as_str);
    let filters = Filters {
        repo: req.repo.as_deref(),
        kind,
        scope: &scope,
        domains,
    };
    let domain_filters = unified::DomainFilters {
        lang: req.lang.as_deref(),
        path_globs: &req.path,
    };
    let query = unified::UnifiedQuery {
        text: &req.query,
        vector: req.vector.as_deref(),
        filters,
        domain_filters,
    };
    let capturing = observation_capture::armed(cfg, track);
    let conn: &Connection = ctx.conn()?;
    let started = std::time::Instant::now();
    let legs = unified::run_legs(cfg, conn, query, window)?;
    let pool_size = legs.pool;
    // Read while identity is still known: `fuse_legs` flattens the legs and
    // drops the passage text and the version anchors. A failure here costs the
    // observation, never the search.
    let facts = capturing
        .then(|| candidate_facts::collect(conn, &legs, cfg.observations.max_text_bytes))
        .transpose()
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "candidate text unavailable; capture skipped");
            None
        });
    let ranked = unified::fuse_legs(cfg, conn, legs)?;
    // Pagination consumes the ranking and hands back only the page, while an
    // observation records the whole pool. The clone is paid only when capture
    // is armed, so an ordinary search allocates exactly what it did before.
    let pooled = facts.is_some().then(|| ranked.clone());
    let (hits, has_more, total) = pipeline::paginate(ranked, window, cfg.retrieval.max_page_window);
    let query_id = if track {
        track_run(conn, &req.query, &hits, filters, window, started)
    } else {
        None
    };
    let observation_id = match (pooled.as_deref(), facts.as_ref()) {
        (Some(pool), Some(facts)) => {
            let captured = Captured {
                pool,
                facts,
                pool_size,
                window,
                query_id: query_id.as_deref(),
            };
            capture_pool(cfg, conn, &req, filters, domain_filters, captured)
        }
        _ => None,
    };
    let meta = page_meta(window, has_more, total);
    Ok(FindResult {
        hits,
        query_id,
        observation_id,
        meta,
    })
}

/// What one captured run contributes that is neither the request nor its
/// filters — bundled so [`capture_pool`] stays inside the argument ceiling.
struct Captured<'a> {
    /// The fused ranking, before pagination cut it to a page.
    pool: &'a [UnifiedHit],
    /// Identity, content version and bounded text, read before fusion.
    facts: &'a FactsByHit,
    /// The shared pool size every leg was fetched at.
    pool_size: usize,
    /// The window the page was sliced at.
    window: PageWindow,
    /// The `retrieval_log` id of the same run, when one was written.
    query_id: Option<&'a str>,
}

/// Persist this run's candidate pool. Best effort by construction: the
/// capture writer swallows its own failures, so a search that cannot record
/// an observation still returns its hits.
fn capture_pool(
    cfg: &Config,
    conn: &Connection,
    req: &Request,
    filters: Filters<'_>,
    domain_filters: unified::DomainFilters<'_>,
    captured: Captured<'_>,
) -> Option<String> {
    observation_capture::record(
        cfg,
        conn,
        CaptureInput {
            query: &req.query,
            query_id: captured.query_id,
            source: crate::utilities::telemetry::source::FIND,
            filters: observation_capture::find_filters(
                cfg,
                filters,
                domain_filters,
                req.vector.as_deref(),
            ),
            window: captured.window,
            pool_size: captured.pool_size,
            pool: captured.pool,
            facts: captured.facts,
        },
    )
}

/// Best-effort telemetry for one tracked run: one `retrieval_log` row for
/// the whole query (not one per leg), plus each domain's OWN access
/// tracker for the hits it contributed — `memories.access_count` for
/// memory hits, `code_feedback` for code hits, matching what `search` and
/// `search-code` each already do for their own domain.
///
/// The log is written at every offset; both access trackers fire only on a
/// head window, for the reason given in
/// `retrieval::pipeline::record_access` (#201).
fn track_run(
    conn: &Connection,
    query: &str,
    hits: &[UnifiedHit],
    filters: Filters<'_>,
    window: PageWindow,
    started: std::time::Instant,
) -> Option<String> {
    let memory_ids: Vec<String> = hits
        .iter()
        .filter(|h| h.domain == unified::fuse_domains::DOMAIN_MEMORY)
        .map(|h| h.id.clone())
        .collect();
    // Both bumps live inside the head check, so a deep page neither writes nor
    // walks the hits looking for something to write — and cannot emit the
    // parse warning below for ids it was never going to use.
    if window.is_head() {
        // A code hit's id is a `code_symbols` rowid that `fuse_domains`
        // stringified, so this parse cannot fail in practice — but a silent
        // drop here would mean access tracking quietly skipping a hit, so it
        // warns like the identical spot in `code_route::fuse_legs` does.
        let code_ids: Vec<i64> = hits
            .iter()
            .filter(|h| h.domain == unified::fuse_domains::DOMAIN_CODE)
            .filter_map(|h| match h.id.parse() {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::warn!(id = %h.id, error = %e, "skipping non-numeric code hit id");
                    None
                }
            })
            .collect();
        // Two shapes because the two consumers genuinely differ:
        // `record_access` takes `&[&str]` (it binds the ids into an `IN` list)
        // while `log_retrieval` takes `&[String]` (it serializes them to JSON).
        let memory_refs: Vec<&str> = memory_ids.iter().map(String::as_str).collect();
        pipeline::record_access(conn, memory_refs.as_slice());
        crate::store::code_row::record_access(conn, &code_ids);
    }
    pipeline::log_retrieval(
        conn,
        query,
        &memory_ids,
        started.elapsed(),
        filters.repo,
        filters.kind,
        crate::utilities::telemetry::source::FIND,
    )
}

#[cfg(test)]
#[path = "tests/find.rs"]
mod tests;
