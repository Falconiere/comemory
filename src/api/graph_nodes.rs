//! `api::graph_nodes` — `GET /api/v1/graph/nodes`, `GET /api/v1/graph/nodes/{id}`,
//! `GET /api/v1/graph/nodes/{id}/neighbors`, `GET /api/v1/graph/snapshot`
//! (console-api spec §5).
//!
//! Four read-only cores over the SAME file-level graph `comemory graph`
//! already exports, reusing its query layer rather than growing a second
//! one (Binding Rule 1): [`crate::cli::graph::nodes::fetch_nodes`] /
//! [`crate::cli::graph::nodes::fetch_node`] for node rows,
//! [`crate::cli::graph::nodes::build_graph`] for the `NodeRow → Node`
//! mapping, [`crate::cli::graph::build_graph_page`] for the snapshot, and
//! [`crate::graph::neighbors::file_neighbors`] — the query `comemory
//! context` reports its `neighbors` from — for the neighborhood (AC-9).
//!
//! Node ids are the canonical `file:<repo>:<path>`; the bare `<repo>:<path>`
//! form and (under an `X-Comemory-Repo` scope) a plain repo-relative path
//! are accepted too, so a console can pass through whichever id shape it
//! holds. See [`resolve_node_id`].

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::cli::graph::nodes::{
    NodeRow, build_graph, cites_file_predicate, fetch_node, fetch_nodes,
};
use crate::cli::graph::{Rel, build_graph_page, parse_id};
use crate::graph::neighbors::{self, DEFAULT_MIN_WEIGHT, NeighborRow};
use crate::output::graph::{Edge, Node};
use crate::output::page::Page;
use crate::output::search::title_of;
use crate::prelude::*;

/// Default page size for `GET /graph/nodes`, matching the `/api/v1`
/// pagination convention (spec §1 "Pagination"). `0` still means "all".
const DEFAULT_LIMIT: usize = 50;

/// Edge ceiling for `GET /graph/snapshot`: the console renders a whole
/// graph at once, so the snapshot is capped rather than paged, and reports
/// `truncated` when the cap bit.
const SNAPSHOT_EDGE_CAP: usize = 20_000;

/// How many symbols `GET /graph/nodes/{id}` lists for the selected file.
const TOP_SYMBOL_LIMIT: usize = 20;

/// `GET /api/v1/graph/nodes` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    /// Restrict to one repo label (as passed to `index-code --repo`).
    #[serde(default)]
    pub repo: Option<String>,
    /// `pagerank` (default) or `path` — see [`parse_sort`].
    #[serde(default)]
    pub sort: Option<String>,
    /// Page size; absent means [`DEFAULT_LIMIT`], `0` means "every node".
    #[serde(default)]
    pub limit: Option<usize>,
    /// Nodes to skip before the page starts.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// `GET /api/v1/graph/nodes/{id}/neighbors` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct NeighborsRequest {
    /// Drop edges below this accumulated weight; absent means
    /// [`DEFAULT_MIN_WEIGHT`] (keep every edge).
    #[serde(default)]
    pub min_weight: Option<i64>,
}

/// `GET /api/v1/graph/snapshot` request.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRequest {
    /// Restrict to one repo label.
    #[serde(default)]
    pub repo: Option<String>,
    /// CSV of edge kinds — see [`parse_edge_kinds`]. Absent means both.
    #[serde(default)]
    pub edge_kinds: Option<String>,
    /// Drop `co_changed` edges below this accumulated weight. Absent means
    /// `1`; values below `1` are clamped up rather than rejected, matching
    /// `api::graph`.
    #[serde(default)]
    pub min_weight: Option<i64>,
}

/// One row of `GET /api/v1/graph/nodes/{id}`'s `top_symbols`.
#[derive(Serialize, Debug)]
pub struct TopSymbol {
    /// `code_symbols` rowid.
    pub id: i64,
    /// Qualified symbol name.
    pub symbol: String,
    /// Symbol kind (`function`, `struct`, …).
    pub kind: String,
    /// Source language slug (`rust`, `typescript`, …).
    pub lang: String,
    /// First source line.
    pub line_start: i64,
    /// Last source line.
    pub line_end: i64,
    /// Materialized PageRank for this symbol.
    pub rank_score: f64,
}

/// One row of `GET /api/v1/graph/nodes/{id}`'s `cited_by`.
#[derive(Serialize, Debug)]
pub struct CitedBy {
    /// Memory id.
    pub id: String,
    /// The memory's first non-empty body line.
    pub title: String,
}

/// `GET /api/v1/graph/nodes/{id}` response.
#[derive(Serialize, Debug)]
pub struct NodeDetail {
    /// The file node itself, identical to its row in `GET /graph/nodes`.
    pub node: Node,
    /// The file's top-level symbols, strongest PageRank first (≤
    /// [`TOP_SYMBOL_LIMIT`]).
    pub top_symbols: Vec<TopSymbol>,
    /// Live memories citing this file, by `references_file` or by a
    /// `references_symbol` edge into one of its symbols.
    pub cited_by: Vec<CitedBy>,
}

/// `GET /api/v1/graph/snapshot` response: one whole (capped) graph.
#[derive(Serialize, Debug)]
pub struct Snapshot {
    /// The endpoints of `edges`, and only those.
    pub nodes: Vec<Node>,
    /// The graph's edges, strongest first, capped at [`SNAPSHOT_EDGE_CAP`].
    pub edges: Vec<Edge>,
    /// Whether the cap bit — more edges match the filters than were sent.
    pub truncated: bool,
    /// Total edges matching the filters, before the cap.
    pub total_edges: usize,
}

/// Which order `GET /graph/nodes` returns nodes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    /// `rank DESC, id ASC` — the default.
    Pagerank,
    /// `id ASC` (which is `file:<repo>:<path>` order).
    Path,
}

/// `GET /api/v1/graph/nodes` — page every indexed file as a graph node.
pub fn list(ctx: &mut Ctx<'_>, req: ListRequest) -> Result<Page<Node>> {
    let sort = parse_sort(req.sort.as_deref())?;
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT);
    let offset = req.offset.unwrap_or(0);
    let conn = ctx.conn()?;
    let mut nodes = nodes_from(fetch_nodes(conn, req.repo.as_deref())?);
    // `nodes_from` already returns id-ascending order, which IS the `path`
    // sort; the pagerank sort is a stable re-sort on top of it, so ties
    // keep that id order rather than falling back to fetch order.
    if sort == Sort::Pagerank {
        nodes.sort_by(|a, b| b.rank.total_cmp(&a.rank));
    }
    Ok(Page::from_slice(nodes, limit, offset))
}

/// `GET /api/v1/graph/nodes/{id}` — one node plus its top symbols and the
/// memories citing it. [`Error::NotFound`] when the id resolves to a file
/// with no top-level `code_symbols` rows.
pub fn detail(ctx: &mut Ctx<'_>, id: &str, repo_scope: Option<&str>) -> Result<NodeDetail> {
    let (repo, path) = resolve_node_id(id, repo_scope)?;
    let conn = ctx.conn()?;
    let Some(row) = fetch_node(conn, &repo, &path)? else {
        return Err(Error::NotFound(format!("graph node {id}")));
    };
    let node = nodes_from(vec![row])
        .into_iter()
        .next()
        .ok_or_else(|| Error::Other(format!("graph node {id} vanished during assembly")))?;
    let top_symbols = fetch_top_symbols(conn, &repo, &path)?;
    let cited_by = fetch_cited_by(conn, &repo, &path)?;
    Ok(NodeDetail {
        node,
        top_symbols,
        cited_by,
    })
}

/// `GET /api/v1/graph/nodes/{id}/neighbors` — the one-hop, undirected
/// `imports`/`co_changed` neighborhood of one file. The id need not name an
/// indexed file: a node with no edges simply has no neighbors, which is a
/// `200 []` rather than a `404`.
pub fn neighbors(
    ctx: &mut Ctx<'_>,
    id: &str,
    repo_scope: Option<&str>,
    req: NeighborsRequest,
) -> Result<Vec<NeighborRow>> {
    let (repo, path) = resolve_node_id(id, repo_scope)?;
    let min_weight = req.min_weight.unwrap_or(DEFAULT_MIN_WEIGHT);
    let conn = ctx.conn()?;
    neighbors::file_neighbors(conn, &[(repo.as_str(), path.as_str())], min_weight)
}

/// `GET /api/v1/graph/snapshot` — the whole graph for one scope, capped at
/// [`SNAPSHOT_EDGE_CAP`] edges.
pub fn snapshot(ctx: &mut Ctx<'_>, req: SnapshotRequest) -> Result<Snapshot> {
    let rel = parse_edge_kinds(req.edge_kinds.as_deref())?;
    let min_weight = req.min_weight.unwrap_or(1).max(1);
    let conn = ctx.conn()?;
    let page = build_graph_page(
        conn,
        req.repo.as_deref(),
        rel,
        min_weight,
        SNAPSHOT_EDGE_CAP,
        0,
    )?;
    Ok(Snapshot {
        nodes: page.nodes,
        edges: page.edges,
        truncated: page.has_more,
        total_edges: page.total,
    })
}

/// Map raw node rows into [`Node`]s through `build_graph` (over an empty
/// edge list) so the console's node shape is produced by exactly the code
/// `comemory graph` uses. Output is id-ascending — `build_graph` keys its
/// nodes by id in a `BTreeMap`.
fn nodes_from(rows: Vec<NodeRow>) -> Vec<Node> {
    build_graph(rows, Vec::new()).nodes
}

/// Split a node id into `(repo, path)`.
///
/// Accepted, in order: the canonical `file:<repo>:<path>`
/// ([`parse_id`]); the bare `<repo>:<path>` form `references_file` edges
/// store; and — only when a repo scope is in force and neither of the above
/// parsed — the whole id read as a repo-relative path under that scope.
/// Anything else is [`Error::BadRequest`].
fn resolve_node_id(id: &str, repo_scope: Option<&str>) -> Result<(String, String)> {
    if let Some((repo, path)) = parse_id(id) {
        return Ok((repo.to_string(), path.to_string()));
    }
    if let Some((repo, path)) = id.split_once(':')
        && !repo.is_empty()
        && !path.is_empty()
    {
        return Ok((repo.to_string(), path.to_string()));
    }
    match repo_scope {
        Some(repo) if !id.is_empty() => Ok((repo.to_string(), id.to_string())),
        _ => Err(Error::BadRequest(format!(
            "unparsable graph node id {id:?}: expected file:<repo>:<path> or <repo>:<path>"
        ))),
    }
}

/// Parse the `sort` field: `pagerank` (default) or `path`.
fn parse_sort(raw: Option<&str>) -> Result<Sort> {
    match raw {
        None | Some("pagerank") => Ok(Sort::Pagerank),
        Some("path") => Ok(Sort::Path),
        Some(other) => Err(Error::BadRequest(format!(
            "invalid sort {other:?}: expected pagerank or path"
        ))),
    }
}

/// Parse the `edge_kinds` CSV into a [`Rel`]. `imports` and the three
/// spellings of the co-change kind (`cochange`, `co_changed`, `co-changed`)
/// are accepted; naming both — or naming none — selects [`Rel::All`].
fn parse_edge_kinds(raw: Option<&str>) -> Result<Rel> {
    let Some(raw) = raw else {
        return Ok(Rel::All);
    };
    let (mut imports, mut cochange) = (false, false);
    for token in raw.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match token {
            "imports" => imports = true,
            "cochange" | "co_changed" | "co-changed" => cochange = true,
            other => {
                return Err(Error::BadRequest(format!(
                    "invalid edge_kinds entry {other:?}: expected imports or cochange"
                )));
            }
        }
    }
    Ok(match (imports, cochange) {
        (true, false) => Rel::Imports,
        (false, true) => Rel::CoChanged,
        _ => Rel::All,
    })
}

/// The file's top-level symbols, strongest PageRank first. Chunk children
/// (`parent_id IS NOT NULL`) are excluded so a split oversized symbol does
/// not crowd out its siblings.
fn fetch_top_symbols(conn: &Connection, repo: &str, path: &str) -> Result<Vec<TopSymbol>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, kind, lang, line_start, line_end, rank_score \
           FROM code_symbols \
          WHERE repo = ?1 AND path = ?2 AND parent_id IS NULL \
          ORDER BY rank_score DESC, symbol ASC, line_start ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![
                repo,
                path,
                i64::try_from(TOP_SYMBOL_LIMIT).unwrap_or(i64::MAX)
            ],
            |r| {
                Ok(TopSymbol {
                    id: r.get(0)?,
                    symbol: r.get(1)?,
                    kind: r.get(2)?,
                    lang: r.get(3)?,
                    line_start: r.get(4)?,
                    line_end: r.get(5)?,
                    rank_score: r.get(6)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Live memories citing this file, through the SAME predicate the node
/// row's `memories` count uses ([`cites_file_predicate`]) — so the count
/// and the list can never disagree.
fn fetch_cited_by(conn: &Connection, repo: &str, path: &str) -> Result<Vec<CitedBy>> {
    let sql = format!(
        "SELECT DISTINCT m.id, m.body FROM edges e JOIN memories m ON m.id = e.src_id \
          WHERE m.deleted_at IS NULL AND {} ORDER BY m.id",
        cites_file_predicate("?1")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![format!("{repo}:{path}")], |r| {
            Ok(CitedBy {
                id: r.get(0)?,
                title: title_of(&r.get::<_, String>(1)?),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/graph_nodes.rs"]
mod tests;
