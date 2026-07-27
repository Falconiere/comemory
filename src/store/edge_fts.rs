//! FTS5 triplet index over `edges`: rendering, wholesale refresh, and the
//! two-tier lexical ladder behind `comemory edges`.
//!
//! Each edge is rendered as three searchable texts — `src_text`,
//! `rel_text`, `dst_text` — alongside UNINDEXED payload columns that carry
//! the raw edge back to the caller. Rendering lives here and only here (the
//! 0012 migration creates the table empty), so the SQL and Rust views of a
//! triplet cannot drift. Refresh is wholesale rather than write-through:
//! ten-plus edge writers exist, two of them raw `DELETE FROM edges` inside
//! `graph::materialize`, and per-site hooks would rot the moment the next
//! writer forgot one.

use rusqlite::{Connection, params};

use crate::prelude::*;
use crate::store::fts;

/// One `edge_fts` match: the raw edge from the payload columns plus the
/// rendered text that was actually indexed.
pub struct EdgeFtsHit {
    /// Node kind of the edge source (`memory`, `file`, `symbol`, …).
    pub src_kind: String,
    /// Source node id, exactly as stored in `edges`.
    pub src_id: String,
    /// Rendered, indexed source text.
    pub src_text: String,
    /// Relation kind (`supersedes`, `imports`, …).
    pub rel: String,
    /// Node kind of the edge target.
    pub dst_kind: String,
    /// Target node id, exactly as stored in `edges`.
    pub dst_id: String,
    /// Rendered, indexed target text.
    pub dst_text: String,
    /// Edge weight, as stored in `edges`.
    pub weight: i64,
    /// Negated BM25 — higher is better, matching the `RoutedHit` lexical
    /// convention (FTS5's own `bm25()` is negative, best-first ascending).
    pub score: f32,
}

/// Node-id kind prefixes stripped before indexing. The kind already rides
/// along in the `src_kind`/`dst_kind` payload column, so leaving the prefix
/// in the text would only plant `file`/`symbol`/`repo` as a high-frequency
/// term in nearly every triplet. Bare `references_*` targets
/// (`<repo>:<path>[:<symbol>]`) carry no prefix and fall through unchanged.
/// INVARIANT: compile-time constants only — entries are interpolated into
/// SQL text by [`strip_prefix_expr`]; a runtime- or user-supplied entry
/// would be a SQL injection vector.
const KIND_PREFIXES: [&str; 5] = ["file:", "symbol:", "repo:", "author:", "tag:"];

/// A SQLite expression rendering `col` with any leading [`KIND_PREFIXES`]
/// entry removed. `substr` is 1-based, so the surviving suffix starts one
/// past the prefix length.
fn strip_prefix_expr(col: &str) -> String {
    let mut expr = String::from("CASE");
    for prefix in KIND_PREFIXES {
        let rest = prefix.len() + 1;
        expr.push_str(&format!(
            " WHEN instr({col}, '{prefix}') = 1 THEN substr({col}, {rest})"
        ));
    }
    expr.push_str(&format!(" ELSE {col} END"));
    expr
}

/// The indexed-text expression for one endpoint. A live memory renders as
/// `kind || ' ' || slug` from the left-joined row under `alias`
/// ("decision queue-design-v1" — human-searchable); a dangling or
/// soft-deleted memory endpoint falls back to its bare id. Every other node
/// kind goes through [`strip_prefix_expr`], and the identifier tokenizer
/// then splits the remaining `:` / `/` / `.` / camelCase boundaries into
/// path and symbol terms.
fn endpoint_text_expr(kind_col: &str, id_col: &str, alias: &str) -> String {
    format!(
        "CASE WHEN {kind_col} = 'memory' \
              THEN COALESCE({alias}.kind || ' ' || {alias}.slug, {id_col}) \
              ELSE {stripped} END",
        stripped = strip_prefix_expr(id_col)
    )
}

/// The whole-table INSERT: one `edge_fts` row per `edges` row, rendered in
/// a single deterministic statement. `ORDER BY` pins the insertion order —
/// and therefore the rowids — to the logical edge key rather than to the
/// order rows happen to sit in `edges`, so two databases holding the same
/// edges index identically.
fn insert_sql() -> String {
    format!(
        "INSERT INTO edge_fts(src_text, rel_text, dst_text, \
                              src_kind, src_id, rel, dst_kind, dst_id, weight) \
         SELECT {src_text}, e.rel, {dst_text}, \
                e.src_kind, e.src_id, e.rel, e.dst_kind, e.dst_id, e.weight \
           FROM edges e \
           LEFT JOIN memories ms \
                  ON e.src_kind = 'memory' AND ms.id = e.src_id AND ms.deleted_at IS NULL \
           LEFT JOIN memories md \
                  ON e.dst_kind = 'memory' AND md.id = e.dst_id AND md.deleted_at IS NULL \
          ORDER BY e.src_kind, e.src_id, e.rel, e.dst_kind, e.dst_id",
        src_text = endpoint_text_expr("e.src_kind", "e.src_id", "ms"),
        dst_text = endpoint_text_expr("e.dst_kind", "e.dst_id", "md"),
    )
}

/// Rebuild `edge_fts` wholesale from `edges` in one transaction and report
/// the number of rows indexed. At personal-memory scale (thousands of
/// edges) this is a millisecond pass — the same economics that make
/// [`crate::graph::memory_rank`] a full recompute. Callers treat failure as
/// best-effort; see [`crate::graph::derived`].
pub fn refresh(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM edge_fts", [])?;
    let written = tx.execute(&insert_sql(), [])?;
    tx.commit()?;
    Ok(written)
}

/// True when `edges` has rows but `edge_fts` does not — the upgraded-database
/// state the 0012 migration deliberately leaves behind. `comemory edges`
/// calls this and refreshes once before querying, so an upgrade self-heals
/// with no flag and no migration backfill. The two counts are read without
/// a shared transaction on purpose: a concurrent writer can only make the
/// answer stale, and every stale outcome is benign — [`refresh`] is an
/// idempotent wholesale rebuild, so the worst cases are one redundant
/// refresh or one no-op refresh over an emptied table.
pub fn needs_refresh(conn: &Connection) -> Result<bool> {
    let indexed: i64 = conn.query_row("SELECT count(*) FROM edge_fts", [], |r| r.get(0))?;
    if indexed > 0 {
        return Ok(false);
    }
    let edges: i64 = conn.query_row("SELECT count(*) FROM edges", [], |r| r.get(0))?;
    Ok(edges > 0)
}

/// Search the triplet text and return one page of hits plus whether more
/// remain. Two tiers, no more: the edges corpus is small enough that strict
/// AND ([`fts::build_match_query`]) then word-OR ([`fts::build_or_query`])
/// exhaust the useful recall — the subtoken and learned-expansion tiers the
/// memory ladder adds would only blur an already tiny result set.
///
/// The tier is chosen once, from whether the strict expression matches
/// anything at all, so paging never silently switches ladders midway. Order
/// is BM25 then `(src_id, rel, dst_id)` ascending, which is total: no two
/// edges share that triple.
pub fn search_edges(
    conn: &Connection,
    query: &str,
    k: usize,
    offset: usize,
) -> Result<(Vec<EdgeFtsHit>, bool)> {
    if k == 0 {
        return Ok((Vec::new(), false));
    }
    let expr = pick_tier(conn, query)?;
    // One extra row probes for a further page without a second COUNT query.
    let mut hits = run_match(conn, &expr, k + 1, offset)?;
    let has_more = hits.len() > k;
    hits.truncate(k);
    Ok((hits, has_more))
}

/// The MATCH expression to page over: the strict AND query when it matches
/// any row, else the word-OR fallback. An empty strict expression (a query
/// of pure whitespace) yields an equally empty fallback, and [`run_match`]
/// short-circuits on it.
fn pick_tier(conn: &Connection, query: &str) -> Result<String> {
    let strict = fts::build_match_query(query);
    if !strict.is_empty() && matches_any(conn, &strict)? {
        return Ok(strict);
    }
    Ok(fts::build_or_query(query))
}

/// Whether `match_expr` hits at least one indexed triplet.
fn matches_any(conn: &Connection, match_expr: &str) -> Result<bool> {
    let probe = fts::run_fts_query(
        conn,
        "SELECT 1 FROM edge_fts WHERE edge_fts MATCH ?1 LIMIT 1",
        params![match_expr],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(!probe.is_empty())
}

/// Execute a prebuilt MATCH expression. Shared by the probe-selected tier
/// and any future caller so tiers cannot drift on SQL or error handling.
/// FTS5 `bm25()` is negative with best first, hence `ORDER BY bm` ascending
/// and the negation on the way out. Parse errors in a user-typed query
/// degrade to an empty page rather than failing the command.
fn run_match(
    conn: &Connection,
    match_expr: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<EdgeFtsHit>> {
    if match_expr.is_empty() {
        return Ok(Vec::new());
    }
    let sql = "SELECT src_kind, src_id, src_text, rel, dst_kind, dst_id, dst_text, weight, \
                      bm25(edge_fts) AS bm \
                 FROM edge_fts \
                WHERE edge_fts MATCH ?1 \
                ORDER BY bm, src_id, rel, dst_id \
                LIMIT ?2 OFFSET ?3";
    fts::run_fts_query(
        conn,
        sql,
        params![match_expr, limit as i64, offset as i64],
        |row| {
            Ok(EdgeFtsHit {
                src_kind: row.get(0)?,
                src_id: row.get(1)?,
                src_text: row.get(2)?,
                rel: row.get(3)?,
                dst_kind: row.get(4)?,
                dst_id: row.get(5)?,
                dst_text: row.get(6)?,
                weight: row.get(7)?,
                score: -row.get::<_, f32>(8)?,
            })
        },
    )
}
