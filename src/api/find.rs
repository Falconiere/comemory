//! `api::find::{Request, run}` — the shared middle of `comemory find` /
//! `GET|POST /api/v1/find`: one ranked list across memory, code, and
//! documents.
//!
//! Distinct from `api::search` (memory only) and `api::search_code` (code
//! only), which keep their domain-specific hit shapes. A single-domain
//! `find` orders identically to the matching dedicated command — see
//! `retrieval::unified` for why.

use serde::Deserialize;

use crate::api::Ctx;
use crate::cli::{page_meta, page_window, when};
use crate::memory::Kind;
use crate::output::search::PageMeta;
use crate::prelude::*;
use crate::retrieval::pipeline;
use crate::retrieval::scope::{Domain, Domains, Filters};
use crate::retrieval::unified::{self, fuse_domains::UnifiedHit};

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
pub struct FindResult {
    /// The fused page.
    pub hits: Vec<UnifiedHit>,
    /// `retrieval_log` row id for this run, when tracking was on.
    pub query_id: Option<String>,
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

/// Run the unified query. `track` governs the `retrieval_log` write and the
/// per-domain access bumps, exactly as it does for `search`.
pub fn run(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<FindResult> {
    let cfg = ctx.cfg;
    let scope = when::scope_from_flags(
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
    let conn: &rusqlite::Connection = ctx.conn()?;
    let started = std::time::Instant::now();
    let run = unified::find(
        cfg,
        conn,
        &req.query,
        req.vector.as_deref(),
        filters,
        unified::DomainFilters {
            lang: req.lang.as_deref(),
            path_globs: &req.path,
        },
        window,
    )?;
    let query_id = if track {
        track_run(conn, &req.query, &run.hits, filters, started)
    } else {
        None
    };
    let meta = page_meta(window, run.has_more, run.total);
    Ok(FindResult {
        hits: run.hits,
        query_id,
        meta,
    })
}

/// Best-effort telemetry for one tracked run: one `retrieval_log` row for
/// the whole query (not one per leg), plus each domain's OWN access
/// tracker for the hits it contributed — `memories.access_count` for
/// memory hits, `code_feedback` for code hits, matching what `search` and
/// `search-code` each already do for their own domain.
fn track_run(
    conn: &rusqlite::Connection,
    query: &str,
    hits: &[UnifiedHit],
    filters: Filters<'_>,
    started: std::time::Instant,
) -> Option<String> {
    let memory_ids: Vec<String> = hits
        .iter()
        .filter(|h| h.domain == unified::fuse_domains::DOMAIN_MEMORY)
        .map(|h| h.id.clone())
        .collect();
    // A code hit's id is a `code_symbols` rowid that `fuse_domains` stringified,
    // so this parse cannot fail in practice — but a silent drop here would mean
    // access tracking quietly skipping a hit, so it warns like the identical
    // spot in `code_route::fuse_legs` does.
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
    // Two shapes, one collection each, because the two consumers genuinely
    // differ: `record_access` takes `&[&str]` (it binds ids into an `IN` list)
    // while `log_retrieval` takes `&[String]` (it serializes them to JSON).
    // `as_slice()` rather than `&memory_refs` so the slice type is explicit at
    // the call site instead of resting on deref coercion.
    let memory_refs: Vec<&str> = memory_ids.iter().map(String::as_str).collect();
    pipeline::record_access(conn, memory_refs.as_slice());
    crate::store::code_row::record_access(conn, &code_ids);
    pipeline::log_retrieval(
        conn,
        query,
        &memory_ids,
        started.elapsed(),
        filters.repo,
        filters.kind,
        crate::stats::source::FIND,
    )
}
