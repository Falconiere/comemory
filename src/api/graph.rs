//! `api::graph::{Request, run}` — the shared middle behind
//! `GET /api/v1/graph`: the full `{nodes, edges}` graph or a
//! `(limit, offset)`-windowed [`GraphPage`], reusing
//! [`crate::cli::graph::build_code_graph`] / [`crate::cli::graph::build_graph_page`]
//! directly (Binding Rule 1) — the same pair the legacy `GET /api/graph`
//! handler already calls, so there is exactly one graph query path.
//!
//! A directory module: `cli::graph`'s donor file already sits close to the
//! 300-code-line budget, so this stays a thin caller rather than relocating
//! the builders. `comemory graph`'s CLI is not rewired onto this module — it
//! already calls `build_graph_page` directly and always wants a page, so
//! routing it through the full/page [`Response`] switch would add pure
//! indirection. `--format` (`dot`/`html`) has no HTTP counterpart (documented
//! exclusion from the parity test) — the versioned surface is always JSON.

use clap::ValueEnum;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::cli::graph::{Rel, build_code_graph, build_graph_page};
use crate::output::graph::{CodeGraph, GraphPage};
use crate::prelude::*;

/// `GET /api/v1/graph` request. `format` is CLI-only (documented exclusion
/// — HTTP is always JSON).
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Restrict to one repo label (as passed to `index-code --repo`).
    #[serde(default)]
    pub repo: Option<String>,
    /// Which edge relations to include (`all` | `imports` | `co-changed`,
    /// the same value syntax as `--rel`). Absent means "all".
    #[serde(default)]
    pub rel: Option<String>,
    /// Drop `co_changed` edges below this accumulated weight (`imports`
    /// always carries weight 1). Absent means the CLI default, 1; values
    /// below 1 are clamped up rather than rejected.
    #[serde(default)]
    pub min_weight: Option<i64>,
    /// Edge-window size. Absent together with `offset` requests the full,
    /// unwindowed graph — the same backward-compatible switch the legacy
    /// `GET /api/graph` handler applies.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Edges to skip before the window starts.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// The full graph or one windowed page — see [`Request::limit`] /
/// [`Request::offset`] for the switch.
#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum Response {
    /// No window requested: the whole `{nodes, edges}` graph.
    Full(CodeGraph),
    /// A `(limit, offset)`-windowed page.
    Paged(GraphPage),
}

/// Run the shared graph middle. Absent `limit` **and** `offset` return the
/// full graph via [`build_code_graph`]; either present returns a windowed
/// [`GraphPage`] via [`build_graph_page`] (missing bounds default to the
/// CLI's own defaults, 50 and 0).
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let rel = parse_rel(req.rel.as_deref())?;
    let min_weight = req.min_weight.unwrap_or(1).max(1);
    let conn: &Connection = ctx.conn()?;
    if req.limit.is_none() && req.offset.is_none() {
        let graph = build_code_graph(conn, req.repo.as_deref(), rel, min_weight)?;
        return Ok(Response::Full(graph));
    }
    let limit = req.limit.unwrap_or(50);
    let offset = req.offset.unwrap_or(0);
    let page = build_graph_page(conn, req.repo.as_deref(), rel, min_weight, limit, offset)?;
    Ok(Response::Paged(page))
}

/// Parse the `rel` field through [`Rel`]'s own `ValueEnum` string table
/// (`"all"` / `"imports"` / `"co-changed"`, the same forms `--rel` accepts)
/// rather than hand-rolling a second match arm (Binding Rule 1). Absent
/// means "all".
fn parse_rel(raw: Option<&str>) -> Result<Rel> {
    match raw {
        None => Ok(Rel::All),
        Some(s) => {
            Rel::from_str(s, true).map_err(|e| Error::BadRequest(format!("invalid rel {s:?}: {e}")))
        }
    }
}
