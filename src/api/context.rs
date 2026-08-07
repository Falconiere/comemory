//! `api::context::{Request, run}` — the shared middle of `comemory context` /
//! `GET|POST /api/v1/context`: route the query through the same memory
//! pipeline as `comemory search`, then assemble the cross-link bundle.
//! Moved out of `cli::context::run` (Binding Rule 1).
//!
//! The CLI's lazy-reindex trigger (`cli::lazy_reindex`) does **not** move
//! here — it stays a CLI-only affordance (spec Non-Goal 8).

use serde::Deserialize;

use crate::api::Ctx;
use crate::cli::{page_meta, page_window, when};
use crate::output::context::ContextResult;
use crate::prelude::*;
use crate::retrieval::code_rerank::WorkingSet;
use crate::retrieval::scope::{Domains, Filters};
use crate::retrieval::{bundle, pipeline};
use crate::store::code_row;

/// `comemory context` / `GET|POST /api/v1/context` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Free-form query — symbol name, file path fragment, or phrase.
    pub query: String,
    /// Page size for the bundle's memory list — overrides the configured
    /// `retrieval.top_k`. `0` means "all remaining within the window".
    #[serde(default)]
    pub k: Option<usize>,
    /// Number of leading ranked memories to skip (deep paging).
    #[serde(default)]
    pub offset: usize,
    /// Optional repo filter.
    #[serde(default)]
    pub repo: Option<String>,
    /// Caller-supplied dense vector, replacing `--vector`/`--vector-stdin`.
    /// `GET` requests never set this — only the `POST` form is
    /// vector-capable.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    /// Only consider memories created at or after this instant.
    #[serde(default)]
    pub since: Option<String>,
    /// Only consider memories created at or before this instant.
    #[serde(default)]
    pub until: Option<String>,
    /// Bundle the corpus as it stood at this instant. Mutually exclusive
    /// with `until`.
    #[serde(default)]
    pub as_of: Option<String>,
}

/// Run the shared context middle: route the query (memory pipeline), then
/// assemble a bundle covering every matched memory plus cross-link edges
/// walked to depth ≤ 2. `track` governs access tracking (memory + the
/// code-ref self-reinforcement below) — see `api::search::run`'s doc for
/// the CLI/HTTP split.
pub fn run(ctx: &mut Ctx<'_>, req: Request, track: bool) -> Result<ContextResult> {
    // Copied out before `ctx.conn()` so the later mutable borrow of `ctx`
    // (for the connection) doesn't also lock out this field.
    let cfg = ctx.cfg;
    let window = page_window(cfg, req.k, req.offset);
    let opts = pipeline::SearchOptions {
        track,
        source: crate::stats::source::CONTEXT,
        window,
    };
    let scope = when::scope_from_flags(
        req.since.as_deref(),
        req.until.as_deref(),
        req.as_of.as_deref(),
    )?;
    let filters = Filters {
        repo: req.repo.as_deref(),
        kind: None,
        scope: &scope,
        domains: Domains::all(),
    };
    let conn: &rusqlite::Connection = ctx.conn()?;
    let run = pipeline::search(cfg, conn, &req.query, req.vector.as_deref(), filters, opts)?;
    let meta = page_meta(window, run.has_more, run.total);
    let query_id = run.query_id;
    let ids: Vec<String> = run.hits.into_iter().map(|h| h.memory_id).collect();
    // Zero hits → no edges to walk, so the git discovery + status walk
    // behind `WorkingSet::from_cwd` is skipped.
    let ws = working_set_for(&ids, req.repo.as_deref());
    let bundle = bundle::assemble(conn, cfg, &req.query, &ids, &ws)?;
    // Self-reinforce the code refs the bundle actually surfaced, the
    // code-side twin of the memory access bump `pipeline::search` already
    // applied — gated by the same `track` flag.
    if track {
        code_row::record_access(conn, &bundle.resolved_code_ids);
    }
    Ok(ContextResult {
        bundle,
        query_id,
        meta,
        scope,
    })
}

/// The working set for the bundle's affinity prior: empty when there are no
/// hits (no edges to walk → skip the git discovery), else discovered from
/// the process cwd (see the module doc for the HTTP cwd caveat).
fn working_set_for(ids: &[String], repo: Option<&str>) -> WorkingSet {
    if ids.is_empty() {
        WorkingSet::default()
    } else {
        WorkingSet::from_cwd(repo)
    }
}
