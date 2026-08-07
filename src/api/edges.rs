//! `api::edges::{Request, run}` — the shared middle of `comemory edges` /
//! `GET /api/v1/edges`: self-heal the `edge_fts` index when needed, then
//! page the triplet search. Moved out of `cli::edges::run` (Binding Rule 1).

use serde::Deserialize;

use crate::api::Ctx;
use crate::cli::page_window;
use crate::config::Config;
use crate::output::edges::EdgesResult;
use crate::prelude::*;
use crate::store::edge_fts;

/// `comemory edges` / `GET /api/v1/edges` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Free-form query — relation verb, memory slug words, path or symbol
    /// fragment, or any mix of them.
    pub query: String,
    /// Page size — overrides the configured `retrieval.top_k`. `0` means
    /// "all remaining within the `max_page_window`".
    #[serde(default)]
    pub k: Option<usize>,
    /// Number of leading ranked relations to skip (deep paging).
    #[serde(default)]
    pub offset: usize,
}

/// Run the shared edges middle: self-heal `edge_fts` when it is empty but
/// `edges` is not (see [`edge_fts::needs_refresh`]), then page the triplet
/// search. `allow_self_heal` gates the healing write — the CLI always
/// passes `true`; a `--read-only` HTTP server passes `false` (§Security:
/// "the one-time `edge_fts` self-heal is skipped on a read-only server")
/// and answers from whatever index already exists.
pub fn run(ctx: &mut Ctx<'_>, req: Request, allow_self_heal: bool) -> Result<EdgesResult> {
    let cfg = ctx.cfg;
    let conn = ctx.conn()?;
    if allow_self_heal && edge_fts::needs_refresh(conn)? {
        edge_fts::refresh(conn)?;
    }
    let window = page_window(cfg, req.k, req.offset);
    let (hits, has_more) = edge_fts::search_edges(
        conn,
        &req.query,
        fetch_size(cfg, window.limit),
        window.offset,
    )?;
    Ok(EdgesResult {
        hits,
        limit: window.limit,
        offset: window.offset,
        has_more,
    })
}

/// How many rows to actually fetch for a requested page size. `k == 0` is
/// the shared "all remaining" sentinel; for a SQL-sliced command that means
/// the configured `max_page_window` ceiling rather than an unbounded scan.
/// Moved verbatim from `cli::edges` (Binding Rule 1).
fn fetch_size(cfg: &Config, limit: usize) -> usize {
    if limit == 0 {
        cfg.retrieval.max_page_window
    } else {
        limit
    }
}
