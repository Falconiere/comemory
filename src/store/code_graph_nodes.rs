//! Node assembly for the file-level code graph behind `comemory graph` /
//! `GET /api/v1/graph`: the `code_symbols` aggregate that turns indexed
//! files into graph nodes ([`NodeRow`]), plus the shared "memory cites this
//! file" predicate ([`cites_file_predicate`]) `api::graph_nodes`'s
//! `cited_by` list reuses.
//!
//! Split out of `cli::graph`/`cli::graph::nodes` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`): this file
//! owns the SQL and row mapping, `cli::graph::nodes` keeps the pure
//! `(repo, path)` dedup and [`crate::output::graph::CodeGraph`] assembly
//! that consume it.

use rusqlite::Connection;

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

/// The `edges` predicate matching every memory citation of ONE file, over
/// an `edges e` row aliased `e`.
///
/// The two reference shapes address the file differently: `references_file`
/// stores the BARE `<repo>:<path>` (no `file:` prefix — see
/// `store::edges::file_node_id`), while `references_symbol` stores
/// `<repo>:<path>:<symbol>`, so a symbol reference is matched by prefix —
/// with `substr(...) = ...`, NOT `LIKE`. A path is full of `_`, which LIKE
/// reads as "any single character" (`src/memory_list.rs` would also match
/// `src/memoryXlist.rs`); the substr form has no metacharacters to escape.
/// Same technique as `store::edges::file_node_prefix`.
///
/// The file the predicate is about, as [`FileExpr`] — a closed set of two
/// SQL expressions rather than a `&str`, so no caller can splice text of
/// its own into the predicate whatever its provenance. Sharing one
/// predicate is what keeps a node's `memories` COUNT and its `cited_by`
/// list answering about the same set of memories (Binding Rule 1).
pub fn cites_file_predicate(file: FileExpr) -> String {
    let file_expr = file.sql();
    format!(
        "e.src_kind = 'memory' \
         AND ((e.rel = 'references_file' AND e.dst_id = {file_expr}) \
           OR (e.rel = 'references_symbol' \
               AND substr(e.dst_id, 1, length({file_expr} || ':')) = {file_expr} || ':'))"
    )
}

/// How [`cites_file_predicate`] names the file it is asking about. Both
/// variants are compile-time SQL: there is no third, caller-supplied one.
#[derive(Clone, Copy)]
pub enum FileExpr {
    /// The row of the enclosing query — a correlated subquery's view of
    /// `code_symbols` ([`extra_columns`]).
    CorrelatedRow,
    /// The first bound parameter, for a standalone query that binds the
    /// `<repo>:<path>` value itself (`api::graph_nodes`'s `cited_by`).
    FirstParam,
}

impl FileExpr {
    /// This variant's SQL expression.
    const fn sql(self) -> &'static str {
        match self {
            Self::CorrelatedRow => "c.repo || ':' || c.path",
            Self::FirstParam => "?1",
        }
    }
}

/// The two columns added for the console's selected-node panel, expressed
/// once so the windowed and unwindowed queries cannot drift.
///
/// `blob` is a plain lookup. The memory count is a correlated subquery
/// rather than a join, built from [`cites_file_predicate`];
/// `COUNT(DISTINCT src_id)` over both reference shapes keeps a memory that
/// cites three symbols in one file counting once, and the `memories` join
/// drops soft-deleted rows.
fn extra_columns() -> String {
    format!(
        "(SELECT f.blob_oid FROM indexed_files f \
          WHERE f.repo = c.repo AND f.path = c.path), \
        (SELECT COUNT(DISTINCT e.src_id) FROM edges e \
           JOIN memories m ON m.id = e.src_id \
          WHERE m.deleted_at IS NULL AND {})",
        cites_file_predicate(FileExpr::CorrelatedRow)
    )
}

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
        "SELECT c.repo, c.path, MAX(c.rank_score), COUNT(*), {} \
           FROM code_symbols c WHERE c.parent_id IS NULL",
        extra_columns()
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

/// Fetch one [`NodeRow`] per distinct `(repo, path)` pair in `pairs`, so a
/// paged subgraph carries exactly the nodes its windowed edges touch (and no
/// others). A pair with no `code_symbols` rows (stale edge endpoint) simply
/// produces no row here — the caller materializes it as a zero-rank node so
/// the edge is never orphaned.
///
/// All pairs are aggregated in ONE chunked query per [`NODE_PAIR_CHUNK`]
/// batch (a `(repo, path)` `VALUES`-join), not one query per pair, so a page
/// of many edges costs a bounded number of round-trips. `pairs` should
/// already be deduplicated — the caller (`cli::graph::nodes::
/// fetch_nodes_for_edges`) builds it from a `BTreeSet` for a stable order.
pub fn fetch_nodes_for_pairs(
    conn: &Connection,
    pairs: &[(String, String)],
) -> Result<Vec<NodeRow>> {
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
        "SELECT c.repo, c.path, MAX(c.rank_score), COUNT(*), {} \
           FROM code_symbols c \
          WHERE c.parent_id IS NULL \
            AND (c.repo, c.path) IN (VALUES {values}) \
          GROUP BY c.repo, c.path \
          ORDER BY c.repo, c.path",
        extra_columns()
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

/// Fetch the single [`NodeRow`] for one indexed file, or `None` when the
/// file has no top-level `code_symbols` rows (never indexed, or indexed only
/// as AST chunk children). Runs [`fetch_node_chunk`] over a one-pair chunk so
/// the console's node-detail lookup and the graph's node list report the same
/// aggregate — rank, symbol count, citing-memory count, blob (Binding Rule 1).
pub fn fetch_node(conn: &Connection, repo: &str, path: &str) -> Result<Option<NodeRow>> {
    let mut rows = Vec::with_capacity(1);
    fetch_node_chunk(conn, &[(repo.to_string(), path.to_string())], &mut rows)?;
    Ok(rows.into_iter().next())
}

/// One row of a file's top-level symbols, strongest PageRank first —
/// behind `api::graph_nodes`'s `top_symbols` list.
pub struct TopSymbolRow {
    /// `code_symbols` rowid.
    pub id: i64,
    /// Qualified symbol name.
    pub symbol: String,
    /// Symbol kind (`function`, `struct`, …).
    pub kind: String,
    /// Source language slug.
    pub lang: String,
    /// First source line.
    pub line_start: i64,
    /// Last source line.
    pub line_end: i64,
    /// Materialized PageRank for this symbol.
    pub rank_score: f64,
}

/// The file's top-level symbols, strongest PageRank first. Chunk children
/// (`parent_id IS NOT NULL`) are excluded so a split oversized symbol does
/// not crowd out its siblings.
pub fn top_symbols(
    conn: &Connection,
    repo: &str,
    path: &str,
    limit: usize,
) -> Result<Vec<TopSymbolRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, symbol, kind, lang, line_start, line_end, rank_score \
           FROM code_symbols \
          WHERE repo = ?1 AND path = ?2 AND parent_id IS NULL \
          ORDER BY rank_score DESC, symbol ASC, line_start ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![repo, path, i64::try_from(limit).unwrap_or(i64::MAX)],
            |r| {
                Ok(TopSymbolRow {
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

/// One live memory citing a file — the raw `(id, body)` pair, before the
/// caller derives a display title from `body`.
pub struct CitingMemoryRow {
    /// Memory id.
    pub id: String,
    /// The memory's full body, from which the caller derives a title.
    pub body: String,
}

/// Live memories citing `(repo, path)`, through the same
/// [`cites_file_predicate`] a node's `memories` count uses — so the count
/// and this list can never disagree.
pub fn citing_memories(conn: &Connection, repo: &str, path: &str) -> Result<Vec<CitingMemoryRow>> {
    let sql = format!(
        "SELECT DISTINCT m.id, m.body FROM edges e JOIN memories m ON m.id = e.src_id \
          WHERE m.deleted_at IS NULL AND {} ORDER BY m.id",
        cites_file_predicate(FileExpr::FirstParam)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![format!("{repo}:{path}")], |r| {
            Ok(CitingMemoryRow {
                id: r.get(0)?,
                body: r.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/code_graph_nodes.rs"]
mod tests;
