//! SQLite-backed edge store. Replaces the v0.1 kuzu writer.

use super::{
    orm,
    schema_graph::{Edges, edges as c},
};
use rusqlite::{Connection, params};
use toolu_orm::core::query_column::CommonOps;

use crate::prelude::*;
use crate::store::MemoryLinks;

/// The `co_activated` relation label: a weighted memory→file edge minted
/// by the co-activation reward when commits touch files a memory
/// references. Named here so the writer ([`crate::domains::graph::coactivate`]) and
/// the `edges.rel` CHECK in `0008_v8_reinforcement.sql` cannot drift on the
/// literal. The weight accumulates via [`insert_weighted`].
pub(crate) const CO_ACTIVATED: &str = "co_activated";

/// The `references_file` relation label: a memory→file edge written by
/// [`crate::domains::graph::cross_link`] whose `dst_id` is the BARE `<repo>:<path>`
/// form (no `file:` kind prefix — see [`file_node_id`]'s divergence note).
/// Named here so the co-activation reverse query binds the same literal the
/// cross-link writer emits.
pub(crate) const REFERENCES_FILE: &str = "references_file";

/// The `references_symbol` relation label: a memory→symbol edge written by
/// [`crate::domains::graph::cross_link`] whose `dst_id` is the BARE
/// `<repo>:<path>:<symbol>` form (no `symbol:` kind prefix — see
/// [`file_node_id`]'s divergence note). Named here so the cross-link writer
/// and the navigation-metadata reader bind the same literal.
pub(crate) const REFERENCES_SYMBOL: &str = "references_symbol";

/// `file → source` edge (bare `source_files.id` → bare `source_roots.id`),
/// written by [`crate::domains::graph::doc_link`]. Filtering/explain only —
/// deliberately excluded from `retrieval::graph_route::ALLOWED_RELS`.
pub(crate) const MEMBER_OF_SOURCE: &str = "member_of_source";

/// Resolved `<repo>:<path>` reference to a `documents` row: `memory →
/// document` for a backtick mention, `document → document` for a resolved
/// Markdown link. Bare ids on both sides. See [`crate::domains::graph::doc_link`].
pub(crate) const REFERENCES_DOCUMENT: &str = "references_document";

/// Addressing tuple for a single directed edge.
///
/// Node identifiers follow the v0.2 convention documented in
/// `migrations/0002_v2_tables.sql`:
/// `memory:<id>`, `file:<repo>:<path>`, `symbol:<symbol_id>`,
/// `repo:<repo>`, `author:<name>`, `tag:<name>`.
#[derive(Clone, Copy)]
pub struct EdgeKey<'a> {
    /// Source node kind (e.g. `"memory"`, `"file"`).
    pub src_kind: &'a str,
    /// Source node identifier.
    pub src_id: &'a str,
    /// Destination node kind.
    pub dst_kind: &'a str,
    /// Destination node identifier.
    pub dst_id: &'a str,
    /// Relation label; must match the `edges.rel` CHECK constraint.
    pub rel: &'a str,
}

/// Emit one memory's reference edges in the order their derivation runs: the
/// `<repo>:<path>` and `<repo>:<path>:<symbol>` mentions harvested from the
/// body, then the `documents` rows those mentions already resolve to. Node
/// addressing matches `migrations/0002_v2_tables.sql` — bare qualified ids on
/// the destination side, no kind prefix.
///
/// [`MemoryLinks`] is destructured rather than read field by field so a fourth
/// link kind cannot be added without this mapping failing to compile; the node
/// kind and relation for each list are only knowable here.
pub(crate) fn insert_memory_references(
    conn: &Connection,
    memory_id: &str,
    links: &MemoryLinks<'_>,
) -> Result<()> {
    let MemoryLinks {
        files,
        symbols,
        documents,
    } = *links;
    let targets = [
        ("file", REFERENCES_FILE, files),
        ("symbol", REFERENCES_SYMBOL, symbols),
        ("document", REFERENCES_DOCUMENT, documents),
    ]
    .into_iter()
    .flat_map(|(dst_kind, rel, ids)| ids.iter().map(move |id| (dst_kind, rel, id.as_str())));
    for (dst_kind, rel, dst_id) in targets {
        let key = EdgeKey {
            src_kind: "memory",
            src_id: memory_id,
            dst_kind,
            dst_id,
            rel,
        };
        insert(conn, key)?;
    }
    Ok(())
}

/// Graph node id for a file: `file:<repo>:<path>` — the addressing
/// convention pinned in `migrations/0002_v2_tables.sql` and used by
/// every graph-side writer/reader (`materialize`, the working set, the
/// affinity prior).
///
/// KNOWN pre-existing divergence: the `references_file` /
/// `references_symbol` edges `memory_row::insert` writes from its
/// [`crate::store::MemoryLinks`] input carry
/// destination ids WITHOUT the `file:` / `symbol:` kind prefix (bare
/// `<repo>:<path>` / `<repo>:<path>:<symbol>`), and its reader
/// `retrieval::bundle::code_ref_lookup` matches that bare form. This works
/// because every query filters by `rel` first, so the two id grammars never
/// meet. Do NOT change the stored formats here — reconciling them needs a
/// data migration (M4 reconcile candidate).
pub(crate) fn file_node_id(repo: &str, path: &str) -> String {
    format!("file:{repo}:{path}")
}

/// Prefix shared by every [`file_node_id`] of `repo`: `file:<repo>:`.
/// Used by repo-scoped `substr`-prefix SQL predicates (injection-proof —
/// a repo label containing `%`/`_` cannot widen a `LIKE`).
pub(crate) fn file_node_prefix(repo: &str) -> String {
    format!("file:{repo}:")
}

/// Insert (or no-op if already present) one edge stamped with the current
/// UTC time.
pub fn insert(conn: &Connection, e: EdgeKey<'_>) -> Result<()> {
    insert_at(conn, e, None)
}

/// Insert (or no-op if already present) one edge, stamped with the given
/// `created_at` string when `Some`, or the current UTC time when `None`.
///
/// The explicit-timestamp form exists for `store::memory_row`, which wipes
/// and re-emits a memory's outgoing edges on every re-save: relation edges
/// (`supersedes` / …) must keep their original `created_at` across the
/// wipe, because `maintenance::retention::low_value::superseded_rule`
/// compares the target's
/// `last_accessed` against the edge timestamp — a refreshed stamp would
/// re-arm the rule on every re-save of the superseder.
pub fn insert_at(conn: &Connection, e: EdgeKey<'_>, created_at: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES(?1,?2,?3,?4,?5, COALESCE(?6, strftime('%Y-%m-%dT%H:%M:%fZ','now')))",
        params![
            e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, created_at
        ],
    )?;
    Ok(())
}

/// Insert one weighted edge, ADDING `weight` to the stored weight when the
/// edge already exists (`ON CONFLICT ... weight = weight + excluded.weight`).
///
/// This is the accumulating writer for `co_changed` edges: each mining run
/// walks only commits newer than the `repo_marker.last_mined_commit` cursor,
/// so the per-run pair counts are deltas to add, not totals to overwrite.
/// State-like relations (e.g. `imports`) should keep using [`insert`], whose
/// `INSERT OR IGNORE` leaves the existing row untouched.
pub fn insert_weighted(conn: &Connection, e: EdgeKey<'_>, weight: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at) \
         VALUES(?1,?2,?3,?4,?5,?6, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         ON CONFLICT(src_kind,src_id,dst_kind,dst_id,rel) \
         DO UPDATE SET weight = weight + excluded.weight",
        params![e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, weight],
    )?;
    Ok(())
}

/// Current accumulated weight of the edge addressed by `e`, or `0` when the
/// edge does not yet exist. The co-activation reward reads this BEFORE
/// [`insert_weighted`] so it can detect the Beta crossing-once boundary
/// (`old < 2 && old + delta >= 2`) as a pure function of the stored state.
pub(crate) fn current_weight(conn: &Connection, e: EdgeKey<'_>) -> Result<i64> {
    let w = orm::query_optional(
        conn,
        Edges::select()
            .columns_typed(&[&c::weight])
            .filter(source_node(e.src_kind, e.src_id))
            .filter(c::dst_kind.eq(e.dst_kind))
            .filter(c::dst_id.eq(e.dst_id))
            .filter(c::rel.eq(e.rel))
            .to_sql(),
        |r| r.get::<_, i64>(0),
    )?;
    Ok(w.unwrap_or(0))
}

/// `COUNT(*)` over `edges` for one relation kind. `rel` is bound as a
/// parameter, never interpolated. Behind `maintenance::overview`'s code-graph edge
/// totals (`co_changed` / `imports`).
pub fn count_by_rel(conn: &Connection, rel: &str) -> Result<u64> {
    let n: i64 = orm::query_one(
        conn,
        Edges::select().filter(c::rel.eq(rel)).to_count_sql(),
        |r| r.get(0),
    )?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// Outgoing neighbors of `(src_kind, src_id)` following `rel`.
pub fn outgoing(
    conn: &Connection,
    src_kind: &str,
    src_id: &str,
    rel: &str,
) -> Result<Vec<(String, String)>> {
    orm::query_all(
        conn,
        Edges::select()
            .columns_typed(&[&c::dst_kind, &c::dst_id])
            .filter(source_node(src_kind, src_id))
            .filter(c::rel.eq(rel))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Transitive `supersedes` chain starting at `start` memory id,
/// depth-bounded by `max_depth`.
///
/// Uses `UNION` (not `UNION ALL`) in the recursive CTE so SQLite deduplicates
/// `(id, depth)` tuples on the fly and terminates immediately when a cycle
/// would re-visit an already-seen node. This prevents exponential blowup on
/// graphs with back-edges (e.g. a→b, b→a).
pub fn supersedes_chain(conn: &Connection, start: &str, max_depth: u32) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE walk(id, depth) AS (
             SELECT ?1, 0
             UNION
             SELECT e.dst_id, w.depth + 1
               FROM edges e
               JOIN walk w ON e.src_id = w.id
              WHERE e.src_kind = 'memory' AND e.dst_kind = 'memory'
                AND e.rel = 'supersedes'
                AND w.depth < ?2
         )
         SELECT id FROM walk WHERE depth > 0 ORDER BY depth",
    )?;
    let rows = stmt
        .query_map(params![start, i64::from(max_depth)], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete every edge originating at `(kind, id)` (source side only). Used
/// by the re-insert cleanup in `store::memory_row`: a re-save or rebuild of
/// a memory must refresh the edges it *emits* without destroying incoming
/// edges such as another memory's `supersedes` pointing at it — rebuild
/// replays memories newest-first, so the superseder's edge is already in
/// place when the superseded memory is inserted.
pub fn delete_outgoing(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    orm::execute(conn, Edges::delete().filter(source_node(kind, id)).to_sql())?;
    Ok(())
}

/// Delete every edge touching `(kind, id)`, either side. Used by
/// soft-delete.
pub fn delete_touching(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    orm::execute(
        conn,
        Edges::delete()
            .filter(source_node(kind, id).or(c::dst_kind.eq(kind).and(c::dst_id.eq(id))))
            .to_sql(),
    )?;
    Ok(())
}

/// One raw `(src_id, dst_id, rel, weight)` row from the code-graph edge set.
/// Read by [`crate::domains::graph::materialize::project_pagerank`] (PageRank input)
/// and [`crate::store::code_graph_edges::fetch_page`] (`comemory graph`'s
/// paginated edge window).
pub struct GraphEdgeRow {
    /// Source node id (`file:<repo>:<path>`).
    pub src_id: String,
    /// Destination node id (`file:<repo>:<path>`).
    pub dst_id: String,
    /// Edge relation: `co_changed` or `imports`.
    pub rel: String,
    /// Accumulated edge weight.
    pub weight: i64,
}

/// Every `co_changed`/`imports` edge whose `src_id` starts with `prefix`
/// (a repo's [`file_node_prefix`]), ordered `(rel, src_id, dst_id)` so the
/// caller's f64 accumulation order is a function of the logical graph, not
/// rowid insertion order.
pub(crate) fn co_changed_and_imports_edges(
    conn: &Connection,
    prefix: &str,
) -> Result<Vec<GraphEdgeRow>> {
    let mut stmt = conn.prepare(
        "SELECT src_id, dst_id, rel, weight FROM edges \
          WHERE rel IN ('co_changed','imports') \
            AND substr(src_id, 1, length(?1)) = ?1 \
          ORDER BY rel, src_id, dst_id",
    )?;
    let rows = stmt
        .query_map([prefix], |r| {
            Ok(GraphEdgeRow {
                src_id: r.get(0)?,
                dst_id: r.get(1)?,
                rel: r.get(2)?,
                weight: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete every `co_changed` edge whose `src_id` starts with `prefix` — used
/// when the co-change miner's stored cursor no longer resolves (history
/// rewrite + gc), so a bounded re-mine does not double-count pairs an
/// earlier run already accumulated.
pub(crate) fn delete_co_changed_for_repo(conn: &Connection, prefix: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM edges \
          WHERE rel = 'co_changed' AND src_kind = 'file' \
            AND substr(src_id, 1, length(?1)) = ?1",
        [prefix],
    )?;
    Ok(())
}

/// Delete every outgoing `imports` edge from `src_id`, leaving any other
/// relation sourced at the same node untouched — unlike [`delete_outgoing`],
/// which deletes every outgoing edge regardless of `rel`.
pub(crate) fn delete_imports_from(conn: &Connection, src_id: &str) -> Result<()> {
    orm::execute(
        conn,
        Edges::delete()
            .filter(source_node("file", src_id))
            .filter(c::rel.eq("imports"))
            .to_sql(),
    )?;
    Ok(())
}

/// Direct memory→memory relation edges (`supersedes`/`conflicts_with`/
/// `derived_from`/`relates_to`), read in the direction they are stored (src
/// = the newer/building memory). See
/// [`crate::domains::graph::memory_rank::derive_memory_graph`].
pub(crate) fn memory_direct_relation_edges(
    conn: &Connection,
) -> Result<Vec<(String, String, f64)>> {
    orm::query_all(
        conn,
        Edges::select()
            .columns_typed(&[&c::src_id, &c::dst_id, &c::weight])
            .filter(c::src_kind.eq("memory"))
            .filter(c::dst_kind.eq("memory"))
            .filter(c::rel.in_list(&[
                "supersedes".into(),
                "conflicts_with".into(),
                "derived_from".into(),
                "relates_to".into(),
            ]))
            .order_by(c::rel.asc())
            .order_by(c::src_id.asc())
            .order_by(c::dst_id.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

/// Co-citation edges: one row per unordered memory pair referencing the
/// same target through the same rel, weighted by the number of shared
/// targets. See [`crate::domains::graph::memory_rank::derive_memory_graph`].
pub(crate) fn memory_co_citation_edges(conn: &Connection) -> Result<Vec<(String, String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT a.src_id, b.src_id, CAST(COUNT(*) AS REAL) AS w \
           FROM edges a \
           JOIN edges b ON a.rel = b.rel AND a.dst_kind = b.dst_kind \
                       AND a.dst_id = b.dst_id AND a.src_id < b.src_id \
          WHERE a.src_kind = 'memory' AND b.src_kind = 'memory' \
            AND a.rel IN ('references_file','references_symbol','co_activated') \
          GROUP BY a.src_id, b.src_id \
          ORDER BY a.src_id, b.src_id",
    )?;
    Ok(stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every memory id whose `rel` edge points at `dst_id`
/// (`src_kind='memory'`, `dst_kind='file'`) — resolves a document's identity
/// against pre-existing memory mentions. See
/// [`crate::domains::graph::doc_link::derive_after_document`].
pub(crate) fn memory_ids_referencing_file(
    conn: &Connection,
    rel: &str,
    dst_id: &str,
) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        Edges::select()
            .columns_typed(&[&c::src_id])
            .filter(c::rel.eq(rel))
            .filter(c::src_kind.eq("memory"))
            .filter(c::dst_kind.eq("file"))
            .filter(c::dst_id.eq(dst_id))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Reverse batch lookup: every `(src_id, dst_id)` edge with the given `rel`
/// and `dst_kind='file'`, `dst_id` one of `dst_ids` — the per-chunk query
/// behind [`crate::domains::graph::coactivate::referencing_memories`].
pub(crate) fn src_ids_for_dst_ids(
    conn: &Connection,
    rel: &str,
    dst_ids: &[&str],
) -> Result<Vec<(String, String)>> {
    let ids = dst_ids.iter().copied().map(Into::into).collect::<Vec<_>>();
    orm::query_all(
        conn,
        Edges::select()
            .columns_typed(&[&c::src_id, &c::dst_id])
            .filter(c::rel.eq(rel))
            .filter(c::dst_kind.eq("file"))
            .filter(c::dst_id.in_list(&ids))
            .order_by(c::dst_id.asc())
            .order_by(c::src_id.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Raw `(repo, path, rel, weight)` neighbors from the file-graph CTE.
/// `seeds_json` is a JSON array of file-node ids; `min_weight` is inclusive.
pub(crate) fn file_neighbor_rows(
    conn: &Connection,
    seeds_json: &str,
    min_weight: i64,
) -> Result<Vec<(String, String, String, i64)>> {
    super::edges_neighbors::file_neighbor_rows(conn, seeds_json, min_weight)
}

/// Match one source node by both its kind and identifier.
fn source_node(kind: &str, id: &str) -> toolu_orm::core::expr::Expr {
    c::src_kind.eq(kind).and(c::src_id.eq(id))
}

#[cfg(test)]
#[path = "tests/edges.rs"]
mod tests;
