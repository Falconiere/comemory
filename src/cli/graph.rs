//! `comemory graph` — export the file-level code-connection graph mined by
//! `index-code` (the `imports` + `co_changed` edges, with nodes weighted by
//! the materialized PageRank `code_symbols.rank_score`) as JSON, Graphviz
//! DOT, or an interactive HTML page (the viewer loads `sigma.js` from a
//! CDN, so rendering the page needs network access on first load).
//!
//! The graph is purely a read over `comemory.db`: it never re-indexes. Run
//! `comemory index-code` first so the `edges` table and `rank_score` are
//! populated. Nodes are files (`file:<repo>:<path>`); edge endpoints that
//! have no `code_symbols` rows (stale edges) still appear, with rank `0`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use rusqlite::Connection;

use std::collections::BTreeSet;

use crate::cli::pagination::PaginationArgs;
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::edges::{file_node_id, file_node_prefix};
use crate::output::graph as render;
use crate::output::graph::{CodeGraph, Edge, GraphPage, Node};
use crate::output::tty;
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Whole graph as JSON (every indexed repo)
  comemory graph

  # Interactive viewer for one repo
  comemory graph --repo myrepo --format html > graph.html && open graph.html

  # Graphviz DOT, imports only, piped to an SVG
  comemory graph --repo myrepo --rel imports --format dot | dot -Tsvg > graph.svg

  # Drop weak co-change links (accumulated weight < 3)
  comemory graph --rel co-changed --min-weight 3";

/// Output rendering for `comemory graph`.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Format {
    /// Machine-readable `{ nodes, edges }` JSON.
    Json,
    /// Graphviz DOT source (pipe to `dot`).
    Dot,
    /// Interactive HTML page (sigma.js, loaded from a CDN).
    Html,
}

/// Which edge relations to include.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Rel {
    /// Both `imports` and `co_changed`.
    All,
    /// Static import edges only.
    Imports,
    /// Git co-change edges only.
    CoChanged,
}

/// Arguments to `comemory graph`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Restrict to one repo label (as passed to `index-code --repo`).
    #[arg(long)]
    pub repo: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    pub format: Format,
    /// Which edge relations to include.
    #[arg(long, value_enum, default_value_t = Rel::All)]
    pub rel: Rel,
    /// Drop `co_changed` edges whose accumulated weight is below this floor
    /// (does not affect `imports`, which always carry weight 1). Must be >= 1.
    #[arg(
        long,
        default_value_t = 1,
        value_parser = clap::builder::RangedI64ValueParser::<i64>::new().range(1..)
    )]
    pub min_weight: i64,
    /// `--limit` / `--offset` window over the edges (in `weight DESC, rel,
    /// src, dst` order). Default limit 50; `--limit 0` exports the full graph.
    /// Applies to every format — DOT/HTML viz render only the current window.
    #[command(flatten)]
    pub page: PaginationArgs,
}

/// Build and emit the code-connection graph. The global `--json` flag forces
/// JSON output regardless of `--format`. The `--limit` / `--offset` window is
/// applied to the edges in every format: JSON emits the [`GraphPage`]
/// envelope, while DOT/HTML render only the windowed subgraph (the viz modes
/// show the current page, not the whole graph). A trailing pagination footer
/// is printed on stderr after the DOT/HTML payload so the human export still
/// reports the window without polluting the pipeable stdout.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let page = build_graph_page(
        &conn,
        a.repo.as_deref(),
        a.rel,
        a.min_weight,
        a.page.limit,
        a.page.offset,
    )?;

    let fmt = if json_flag { Format::Json } else { a.format };
    // JSON emits the envelope; DOT/HTML render the windowed subgraph and append
    // a window footer on stderr so it never corrupts a `| dot` / `>` pipeline.
    match fmt {
        Format::Json => render::write_json_page(&page),
        Format::Dot | Format::Html => render_viz(fmt, page),
    }
}

/// Render a paginated graph in a viz (`dot`/`html`) format: stream the windowed
/// subgraph to stdout, then write the pagination footer to stderr.
fn render_viz(fmt: Format, page: GraphPage) -> Result<()> {
    let (offset, total) = (page.offset, page.total);
    let graph = CodeGraph {
        nodes: page.nodes,
        edges: page.edges,
    };
    let edge_count = graph.edges.len();
    match fmt {
        Format::Dot => render::write_dot(&graph)?,
        // `Json` is dispatched in `run`; treat any non-Dot here as the HTML viz.
        _ => render::write_html(&graph)?,
    }
    tty::write_page_footer(
        &mut std::io::stderr().lock(),
        edge_count,
        offset,
        Some(total),
    )
}

/// Build the file-level [`CodeGraph`] for the selected repo / relations /
/// min-weight, returning the **full** graph (no edge window). Used by the
/// `comemory serve` graph handler's backward-compatible "no params" path.
pub(crate) fn build_code_graph(
    conn: &Connection,
    repo: Option<&str>,
    rel: Rel,
    min_weight: i64,
) -> Result<CodeGraph> {
    let (edges, _total) = fetch_edges(conn, repo, rels_of(rel), min_weight, 0, 0)?;
    let node_rows = fetch_nodes(conn, repo)?;
    Ok(build_graph(node_rows, edges))
}

/// Build the paginated [`GraphPage`] for the selected scope, windowing the
/// edges by `(limit, offset)` and deriving the page's nodes from only those
/// edges' endpoints. Shared by `cli::graph::run` and the `comemory serve`
/// graph handler's paginated path so the two cannot drift.
pub(crate) fn build_graph_page(
    conn: &Connection,
    repo: Option<&str>,
    rel: Rel,
    min_weight: i64,
    limit: usize,
    offset: usize,
) -> Result<GraphPage> {
    let (edges, total) = fetch_edges(conn, repo, rels_of(rel), min_weight, limit, offset)?;
    let node_rows = fetch_nodes_for_edges(conn, &edges)?;
    let graph = build_graph(node_rows, edges);
    Ok(GraphPage::new(graph, limit, offset, total))
}

/// The `edges.rel` values selected by a [`Rel`] choice.
fn rels_of(rel: Rel) -> &'static [&'static str] {
    match rel {
        Rel::All => &["co_changed", "imports"],
        Rel::Imports => &["imports"],
        Rel::CoChanged => &["co_changed"],
    }
}

/// Split a canonical file node id (`file:<repo>:<path>`) into `(repo, path)`.
/// Returns `None` for ids that do not follow the convention. Assumes repo
/// labels contain no `:` (the same assumption baked into `file_node_prefix`'s
/// `substr` predicate); a repo with a `:` would split on the wrong colon.
pub fn parse_id(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("file:")?.split_once(':')
}

/// Map one `edges` row into an [`Edge`].
fn map_edge(r: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        src: r.get(0)?,
        dst: r.get(1)?,
        rel: r.get(2)?,
        weight: r.get(3)?,
    })
}

/// Fetch a `(limit, offset)` window of file→file edges for the selected
/// relations, scoped to one repo's source side and dropping low-weight
/// `co_changed` links, plus the `total` count of edges matching those same
/// scope filters (pre-window) so the caller can compute an exact `has_more`.
///
/// Edges sort by the stable `weight DESC, rel ASC, src_id ASC, dst_id ASC`:
/// weight-desc is primary so a bounded export keeps the strongest links, with
/// the legacy `(rel, src, dst)` tiebreak making the window deterministic across
/// pages. `limit == 0` is the shared "all" sentinel — the `LIMIT` is dropped.
/// The `rels` values are fixed crate constants, never user input, so inlining
/// them in the `IN (...)` list is injection-safe.
fn fetch_edges(
    conn: &Connection,
    repo: Option<&str>,
    rels: &[&str],
    min_weight: i64,
    limit: usize,
    offset: usize,
) -> Result<(Vec<Edge>, usize)> {
    let in_list = rels
        .iter()
        .map(|r| format!("'{r}'"))
        .collect::<Vec<_>>()
        .join(",");
    let mut where_clause = format!(
        " WHERE rel IN ({in_list}) \
            AND (rel <> 'co_changed' OR weight >= ?1)"
    );
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(min_weight)];
    if let Some(r) = repo {
        // Gate both endpoints by the `file:<repo>:` prefix so SQLite rejects
        // cross-repo edges directly — no Rust-side post-filter, and same index.
        where_clause.push_str(
            " AND substr(src_id, 1, length(?2)) = ?2 AND substr(dst_id, 1, length(?2)) = ?2",
        );
        binds.push(Box::new(file_node_prefix(r)));
    }

    let total: usize = {
        let count_sql = format!("SELECT count(*) FROM edges{where_clause}");
        let mut stmt = conn.prepare(&count_sql)?;
        let n: i64 = stmt.query_row(
            rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
            |r| r.get(0),
        )?;
        usize::try_from(n).unwrap_or(0)
    };

    // SQLite forbids a bare `OFFSET`, so `limit == 0` ("all") uses its
    // `LIMIT -1` ("no limit") idiom while still honoring `offset`.
    let window = if limit == 0 {
        format!(" LIMIT -1 OFFSET {offset}")
    } else {
        format!(" LIMIT {limit} OFFSET {offset}")
    };
    let sql = format!(
        "SELECT src_id, dst_id, rel, weight FROM edges{where_clause} \
          ORDER BY weight DESC, rel ASC, src_id ASC, dst_id ASC{window}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
            map_edge,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((rows, total))
}

/// Fetch one [`NodeRow`] per distinct endpoint file referenced by `edges`, so
/// a paged subgraph carries exactly the nodes its windowed edges touch (and no
/// others). Endpoints whose ids don't parse, or that have no `code_symbols`
/// rows (stale edges), simply produce no row here — [`build_graph`] then
/// materializes them as zero-rank nodes so the edge is never orphaned.
fn fetch_nodes_for_edges(conn: &Connection, edges: &[Edge]) -> Result<Vec<NodeRow>> {
    // Dedup endpoints into a stable set so each file is fetched once and the
    // node list is deterministic.
    let pairs: BTreeSet<(String, String)> = edges
        .iter()
        .flat_map(|e| [e.src.as_str(), e.dst.as_str()])
        .filter_map(|id| parse_id(id).map(|(r, p)| (r.to_string(), p.to_string())))
        .collect();
    let mut rows = Vec::with_capacity(pairs.len());
    let mut stmt = conn.prepare(
        "SELECT repo, path, MAX(rank_score), COUNT(*) FROM code_symbols \
          WHERE parent_id IS NULL AND repo = ?1 AND path = ?2 \
          GROUP BY repo, path",
    )?;
    for (repo, path) in &pairs {
        let mut got = stmt.query_map(rusqlite::params![repo, path], map_node_row)?;
        if let Some(row) = got.next() {
            rows.push(row?);
        }
    }
    Ok(rows)
}

/// A raw per-file node row: `(repo, path, rank, symbol_count)`.
pub type NodeRow = (String, String, f64, u32);

/// Map one `code_symbols` aggregate row into a [`NodeRow`].
fn map_node_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<NodeRow> {
    Ok((
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, f64>(2)?,
        // Saturate rather than wrap: a `COUNT(*)` over a file's symbols is
        // always small and non-negative, so the fallback never actually
        // fires — but any out-of-range i64 (negative or > u32::MAX) maps to
        // u32::MAX instead of a silent truncating `as` cast that would lie.
        u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
    ))
}

/// Fetch one node row per indexed file, with its PageRank and top-level
/// symbol count. Only parent rows (`parent_id IS NULL`) are counted so AST
/// chunk children do not inflate the symbol tally.
fn fetch_nodes(conn: &Connection, repo: Option<&str>) -> Result<Vec<NodeRow>> {
    // MAX(rank_score) projects the file's most important symbol's PageRank
    // onto the file node (rather than SUM/AVG), so a file is sized by its
    // single most central symbol.
    let mut sql = String::from(
        "SELECT repo, path, MAX(rank_score), COUNT(*) FROM code_symbols \
          WHERE parent_id IS NULL",
    );
    // Borrow `repo` (the parameter, which outlives `binds`) rather than the
    // if-let local, so the `&&str` pushed here lives until `query_map`.
    let mut binds: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if let Some(r) = &repo {
        sql.push_str(" AND repo = ?1");
        binds.push(r);
    }
    sql.push_str(" GROUP BY repo, path ORDER BY repo, path");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(binds), map_node_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Assemble the [`CodeGraph`] from node rows and edges. Edge endpoints that
/// have no `code_symbols` row (e.g. a stale co-change link to a deleted file)
/// are still materialized as zero-rank nodes so the edge is not orphaned.
pub fn build_graph(node_rows: Vec<NodeRow>, edges: Vec<Edge>) -> CodeGraph {
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for (repo, path, rank, symbols) in node_rows {
        let id = file_node_id(&repo, &path);
        nodes.insert(
            id.clone(),
            Node {
                id,
                label: path,
                repo,
                rank,
                symbols,
            },
        );
    }
    for e in &edges {
        for id in [&e.src, &e.dst] {
            if nodes.contains_key(id) {
                continue;
            }
            if let Some((repo, path)) = parse_id(id) {
                nodes.insert(
                    id.clone(),
                    Node {
                        id: id.clone(),
                        label: path.to_string(),
                        repo: repo.to_string(),
                        rank: 0.0,
                        symbols: 0,
                    },
                );
            }
        }
    }
    CodeGraph {
        nodes: nodes.into_values().collect(),
        edges,
    }
}
