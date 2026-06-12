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

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::edges::{file_node_id, file_node_prefix};
use crate::output::graph as render;
use crate::output::graph::{CodeGraph, Edge, Node};
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
}

/// Build and emit the code-connection graph. The global `--json` flag forces
/// JSON output regardless of `--format`.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let conn = connection::open(paths.db_path())?;

    let graph = build_code_graph(&conn, a.repo.as_deref(), a.rel, a.min_weight)?;

    let fmt = if json_flag { Format::Json } else { a.format };
    match fmt {
        Format::Json => render::write_json(&graph),
        Format::Dot => render::write_dot(&graph),
        Format::Html => render::write_html(&graph),
    }
}

/// Build the file-level [`CodeGraph`] for the selected repo / relations /
/// min-weight by reading `edges` + `code_symbols`. Shared by `cli::graph::run`
/// (the static exports) and the `comemory serve` graph handler so the two
/// renderers cannot drift on which nodes and edges the graph contains.
pub(crate) fn build_code_graph(
    conn: &Connection,
    repo: Option<&str>,
    rel: Rel,
    min_weight: i64,
) -> Result<CodeGraph> {
    let edges = fetch_edges(conn, repo, rels_of(rel), min_weight)?;
    let node_rows = fetch_nodes(conn, repo)?;
    Ok(build_graph(node_rows, edges))
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

/// Fetch file→file edges for the selected relations, optionally scoped to one
/// repo's source side, dropping low-weight `co_changed` links. The `rels`
/// values are fixed crate constants, never user input, so inlining them in
/// the `IN (...)` list is injection-safe.
fn fetch_edges(
    conn: &Connection,
    repo: Option<&str>,
    rels: &[&str],
    min_weight: i64,
) -> Result<Vec<Edge>> {
    let in_list = rels
        .iter()
        .map(|r| format!("'{r}'"))
        .collect::<Vec<_>>()
        .join(",");
    let mut sql = format!(
        "SELECT src_id, dst_id, rel, weight FROM edges \
          WHERE rel IN ({in_list}) \
            AND (rel <> 'co_changed' OR weight >= ?1)"
    );
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(min_weight)];
    if let Some(r) = repo {
        // Gate both endpoints by the `file:<repo>:` prefix so SQLite rejects
        // cross-repo edges directly — no Rust-side post-filter, and same index.
        sql.push_str(
            " AND substr(src_id, 1, length(?2)) = ?2 AND substr(dst_id, 1, length(?2)) = ?2",
        );
        binds.push(Box::new(file_node_prefix(r)));
    }
    sql.push_str(" ORDER BY rel, src_id, dst_id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
            map_edge,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
