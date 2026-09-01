//! Node assembly for the file-level code graph: the `code_symbols`
//! aggregate that turns indexed files into graph nodes, and the
//! [`build_graph`] pass that joins those rows to their edges.
//!
//! Split out of `cli::graph` when the node query grew the `memories` and
//! `blob` columns the console's selected-node panel needs — the donor file
//! was at the 300-line ceiling. `cli::graph` keeps the CLI surface, the edge
//! fetch, and the two `build_*` entry points; everything about turning a
//! `(repo, path)` pair into a [`Node`] lives here.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;

use crate::cli::graph::parse_id;
use crate::graph::edges::file_node_id;
use crate::output::graph::{CodeGraph, Edge, Node};
use crate::prelude::*;

/// A raw per-file node row, straight off the aggregate query.
pub struct NodeRow {
    /// Repo label the file was indexed under.
    pub repo: String,
    /// Repo-relative path.
    pub path: String,
    /// `MAX(rank_score)` over the file's top-level symbols.
    pub rank: f64,
    /// Count of top-level symbols in the file.
    pub symbols: u32,
    /// Distinct live memories referencing this file, by file or by symbol.
    pub memories: u64,
    /// Blob OID recorded for the file at index time, when it has one.
    pub blob: Option<String>,
}

/// The two columns added for the console's selected-node panel, expressed
/// once so the windowed and unwindowed queries cannot drift.
///
/// `blob` is a plain lookup. The memory count is a correlated subquery
/// rather than a join because the two reference shapes address the file
/// differently: `references_file` stores the BARE `<repo>:<path>` (no
/// `file:` prefix — see `graph::edges::file_node_id`), while
/// `references_symbol` stores `<repo>:<path>:<symbol>`, so a symbol
/// reference is matched by prefix — with `substr(...) = ...`, NOT `LIKE`.
/// A path is full of `_`, which LIKE reads as "any single character"
/// (`src/memory_list.rs` would also match `src/memoryXlist.rs`); the
/// substr form has no metacharacters to escape. Same technique as
/// `graph::edges::file_node_prefix`. `COUNT(DISTINCT src_id)` over both keeps
/// a memory that cites three symbols in one file counting once, and the
/// `memories` join drops soft-deleted rows.
const EXTRA_COLUMNS: &str = "\
    (SELECT f.blob_oid FROM indexed_files f \
      WHERE f.repo = c.repo AND f.path = c.path), \
    (SELECT COUNT(DISTINCT e.src_id) FROM edges e \
       JOIN memories m ON m.id = e.src_id \
      WHERE m.deleted_at IS NULL AND e.src_kind = 'memory' \
        AND ((e.rel = 'references_file' AND e.dst_id = c.repo || ':' || c.path) \
          OR (e.rel = 'references_symbol' \
              AND substr(e.dst_id, 1, length(c.repo || ':' || c.path || ':')) \
                  = c.repo || ':' || c.path || ':')))";

/// Max `(repo, path)` pairs per batched node fetch. Each pair binds two host
/// params, so `500 × 2 = 1000` stays far under bundled SQLite's
/// `SQLITE_MAX_VARIABLE_NUMBER` (32766).
const NODE_PAIR_CHUNK: usize = 500;

/// Map one aggregate row into a [`NodeRow`].
fn map_node_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<NodeRow> {
    Ok(NodeRow {
        repo: r.get(0)?,
        path: r.get(1)?,
        rank: r.get(2)?,
        // Saturate rather than wrap: a `COUNT(*)` over a file's symbols is
        // always small and non-negative, so the fallback never actually
        // fires — but any out-of-range i64 (negative or > u32::MAX) maps to
        // u32::MAX instead of a silent truncating `as` cast that would lie.
        symbols: u32::try_from(r.get::<_, i64>(3)?).unwrap_or(u32::MAX),
        blob: r.get(4)?,
        memories: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
    })
}

/// Fetch one node row per indexed file, with its PageRank, top-level symbol
/// count, referencing-memory count, and blob OID. Only parent rows
/// (`parent_id IS NULL`) are counted so AST chunk children do not inflate
/// the symbol tally.
pub fn fetch_nodes(conn: &Connection, repo: Option<&str>) -> Result<Vec<NodeRow>> {
    // MAX(rank_score) projects the file's most important symbol's PageRank
    // onto the file node (rather than SUM/AVG), so a file is sized by its
    // single most central symbol.
    let mut sql = format!(
        "SELECT c.repo, c.path, MAX(c.rank_score), COUNT(*), {EXTRA_COLUMNS} \
           FROM code_symbols c WHERE c.parent_id IS NULL"
    );
    // Borrow `repo` (the parameter, which outlives `binds`) rather than the
    // if-let local, so the `&&str` pushed here lives until `query_map`.
    let mut binds: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if let Some(r) = &repo {
        sql.push_str(" AND c.repo = ?1");
        binds.push(r);
    }
    sql.push_str(" GROUP BY c.repo, c.path ORDER BY c.repo, c.path");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(binds), map_node_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Fetch one [`NodeRow`] per distinct endpoint file referenced by `edges`, so
/// a paged subgraph carries exactly the nodes its windowed edges touch (and no
/// others). Endpoints whose ids don't parse, or that have no `code_symbols`
/// rows (stale edges), simply produce no row here — [`build_graph`] then
/// materializes them as zero-rank nodes so the edge is never orphaned.
///
/// All distinct `(repo, path)` endpoints are aggregated in ONE chunked query
/// (a `(repo, path)` `VALUES`-join), not one query per endpoint, so a page of
/// many edges costs a bounded number of round-trips.
pub fn fetch_nodes_for_edges(conn: &Connection, edges: &[Edge]) -> Result<Vec<NodeRow>> {
    // Dedup endpoints into a stable set so each file is fetched once and the
    // node list is deterministic.
    let pairs: BTreeSet<(String, String)> = edges
        .iter()
        .flat_map(|e| [e.src.as_str(), e.dst.as_str()])
        .filter_map(|id| parse_id(id).map(|(r, p)| (r.to_string(), p.to_string())))
        .collect();
    let pairs: Vec<(String, String)> = pairs.into_iter().collect();
    let mut rows = Vec::with_capacity(pairs.len());
    // Two host params per pair; stay well under SQLite's variable cap.
    for chunk in pairs.chunks(NODE_PAIR_CHUNK) {
        fetch_node_chunk(conn, chunk, &mut rows)?;
    }
    Ok(rows)
}

/// Aggregate one chunk of distinct `(repo, path)` endpoints in a single query.
/// Restricting to the wanted pairs with a row-value `(repo, path) IN (VALUES …)`
/// keeps the `parent_id IS NULL` filter and the per-pair `MAX(rank_score)` +
/// `COUNT(*)` aggregation identical to the unwindowed query; pairs with no
/// `parent_id IS NULL` rows simply produce no group. Endpoint order is
/// preserved (`ORDER BY repo, path` over the already-sorted input).
fn fetch_node_chunk(
    conn: &Connection,
    chunk: &[(String, String)],
    rows: &mut Vec<NodeRow>,
) -> Result<()> {
    if chunk.is_empty() {
        return Ok(());
    }
    // One `(?,?)` tuple per wanted pair, fed to a row-value `IN (VALUES …)`.
    let values = std::iter::repeat_n("(?,?)", chunk.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT c.repo, c.path, MAX(c.rank_score), COUNT(*), {EXTRA_COLUMNS} \
           FROM code_symbols c \
          WHERE c.parent_id IS NULL \
            AND (c.repo, c.path) IN (VALUES {values}) \
          GROUP BY c.repo, c.path \
          ORDER BY c.repo, c.path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params = chunk
        .iter()
        .flat_map(|(repo, path)| [repo.as_str(), path.as_str()]);
    let chunk_rows = stmt
        .query_map(rusqlite::params_from_iter(params), map_node_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.extend(chunk_rows);
    Ok(())
}

/// Assemble the [`CodeGraph`] from node rows and edges. Edge endpoints that
/// have no `code_symbols` row (e.g. a stale co-change link to a deleted file)
/// are still materialized as zero-rank nodes so the edge is not orphaned —
/// those carry no blob and no memory count, which is the honest answer for a
/// file the index has never seen.
pub fn build_graph(node_rows: Vec<NodeRow>, edges: Vec<Edge>) -> CodeGraph {
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for row in node_rows {
        let id = file_node_id(&row.repo, &row.path);
        nodes.insert(
            id.clone(),
            Node {
                id,
                label: row.path,
                repo: row.repo,
                rank: row.rank,
                symbols: row.symbols,
                memories: row.memories,
                blob: row.blob,
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
                        memories: 0,
                        blob: None,
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
