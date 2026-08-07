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

use std::fmt::Write as _;

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
/// `document:`/`source:` are defensive, matching the file/symbol precedent,
/// even though today's writer stores bare document/source ids — see
/// [`endpoint_text_expr`] for how those two kinds actually render.
/// INVARIANT: compile-time constants only — entries are interpolated into
/// SQL text by [`strip_prefix_expr`]; a runtime- or user-supplied entry
/// would be a SQL injection vector.
const KIND_PREFIXES: [&str; 7] = [
    "file:",
    "symbol:",
    "repo:",
    "author:",
    "tag:",
    "document:",
    "source:",
];

/// A SQLite expression rendering `col` with any leading [`KIND_PREFIXES`]
/// entry removed. `substr` is 1-based, so the surviving suffix starts one
/// past the prefix length.
fn strip_prefix_expr(col: &str) -> String {
    let mut expr = String::from("CASE");
    for prefix in KIND_PREFIXES {
        let rest = prefix.len() + 1;
        let _ = write!(
            expr,
            " WHEN instr({col}, '{prefix}') = 1 THEN substr({col}, {rest})"
        );
    }
    let _ = write!(expr, " ELSE {col} END");
    expr
}

/// LEFT JOIN aliases [`endpoint_text_expr`] renders through for one
/// endpoint column — the same shape serves both `src_*` and `dst_*` since
/// each alias's `ON` clause already scopes it to that side.
struct Endpoint<'a> {
    memory: &'a str,
    document: &'a str,
    source: &'a str,
}

/// The indexed-text expression for one endpoint. A live memory renders as
/// `kind || ' ' || slug`; a live document as `<repo>:<path> — <title>` (or
/// just `<path> — <title>` when the document carries no repo label); a
/// live source as its canonical filesystem path — never the raw 128-bit
/// hex id, which would plant an opaque, high-frequency term in every
/// triplet touching it. A dangling/soft-deleted endpoint of any of these
/// three kinds falls back to [`strip_prefix_expr`]; every other kind goes
/// straight there, and the identifier tokenizer then splits the remaining
/// `:` / `/` / `.` / camelCase boundaries into path and symbol terms.
fn endpoint_text_expr(kind_col: &str, id_col: &str, ep: Endpoint<'_>) -> String {
    let Endpoint {
        memory,
        document,
        source,
    } = ep;
    let stripped = strip_prefix_expr(id_col);
    format!(
        "CASE \
           WHEN {kind_col} = 'memory' THEN COALESCE({memory}.kind || ' ' || {memory}.slug, {id_col}) \
           WHEN {kind_col} = 'document' THEN COALESCE( \
               CASE WHEN {document}.repo IS NOT NULL \
                    THEN {document}.repo || ':' || {document}.relative_path || ' \u{2014} ' || {document}.title \
                    ELSE {document}.relative_path || ' \u{2014} ' || {document}.title END, \
               {stripped}) \
           WHEN {kind_col} = 'source' THEN COALESCE({source}.canonical_path, {stripped}) \
           ELSE {stripped} END"
    )
}

/// `documents` joined to its owning `source_files` row for the relative
/// path — the two columns [`endpoint_text_expr`]'s document branch needs
/// alongside `title`/`repo`. A CTE so [`insert_sql`] can join it twice
/// (once per endpoint side) without repeating the join.
const DOCUMENT_IDENTITY_CTE: &str = "document_identity AS ( \
    SELECT d.id, d.title, d.repo, sf.relative_path \
      FROM documents d JOIN source_files sf ON sf.id = d.source_file_id)";

/// The whole-table INSERT: one `edge_fts` row per `edges` row, rendered in
/// a single deterministic statement. `ORDER BY` pins the insertion order —
/// and therefore the rowids — to the logical edge key rather than to the
/// order rows happen to sit in `edges`, so two databases holding the same
/// edges index identically.
fn insert_sql() -> String {
    format!(
        "WITH {cte} \
         INSERT INTO edge_fts(src_text, rel_text, dst_text, \
                              src_kind, src_id, rel, dst_kind, dst_id, weight) \
         SELECT {src_text}, e.rel, {dst_text}, \
                e.src_kind, e.src_id, e.rel, e.dst_kind, e.dst_id, e.weight \
           FROM edges e \
           LEFT JOIN memories ms \
                  ON e.src_kind = 'memory' AND ms.id = e.src_id AND ms.deleted_at IS NULL \
           LEFT JOIN memories md \
                  ON e.dst_kind = 'memory' AND md.id = e.dst_id AND md.deleted_at IS NULL \
           LEFT JOIN document_identity ds ON e.src_kind = 'document' AND ds.id = e.src_id \
           LEFT JOIN document_identity dd ON e.dst_kind = 'document' AND dd.id = e.dst_id \
           LEFT JOIN source_roots ss ON e.src_kind = 'source' AND ss.id = e.src_id \
           LEFT JOIN source_roots sd ON e.dst_kind = 'source' AND sd.id = e.dst_id \
          ORDER BY e.src_kind, e.src_id, e.rel, e.dst_kind, e.dst_id",
        cte = DOCUMENT_IDENTITY_CTE,
        src_text = endpoint_text_expr(
            "e.src_kind",
            "e.src_id",
            Endpoint {
                memory: "ms",
                document: "ds",
                source: "ss"
            }
        ),
        dst_text = endpoint_text_expr(
            "e.dst_kind",
            "e.dst_id",
            Endpoint {
                memory: "md",
                document: "dd",
                source: "sd"
            }
        ),
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

#[cfg(test)]
#[path = "tests/edge_fts.rs"]
mod tests;
