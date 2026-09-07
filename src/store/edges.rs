//! SQLite-backed edge store. Replaces the v0.1 kuzu writer.

use rusqlite::{Connection, OptionalExtension, named_params, params};

use crate::prelude::*;

/// The `co_activated` relation label: a weighted memory→file edge minted
/// by the co-activation reward when commits touch files a memory
/// references. Named here so the writer ([`crate::graph::coactivate`]) and
/// the `edges.rel` CHECK in `0008_v8_reinforcement.sql` cannot drift on the
/// literal. The weight accumulates via [`insert_weighted`].
pub(crate) const CO_ACTIVATED: &str = "co_activated";

/// The `references_file` relation label: a memory→file edge written by
/// [`crate::graph::cross_link`] whose `dst_id` is the BARE `<repo>:<path>`
/// form (no `file:` kind prefix — see [`file_node_id`]'s divergence note).
/// Named here so the co-activation reverse query binds the same literal the
/// cross-link writer emits.
pub(crate) const REFERENCES_FILE: &str = "references_file";

/// The `references_symbol` relation label: a memory→symbol edge written by
/// [`crate::graph::cross_link`] whose `dst_id` is the BARE
/// `<repo>:<path>:<symbol>` form (no `symbol:` kind prefix — see
/// [`file_node_id`]'s divergence note). Named here so the cross-link writer
/// and the navigation-metadata reader bind the same literal.
pub(crate) const REFERENCES_SYMBOL: &str = "references_symbol";

/// `file → source` edge (bare `source_files.id` → bare `source_roots.id`),
/// written by [`crate::graph::doc_link`]. Filtering/explain only —
/// deliberately excluded from `retrieval::graph_route::ALLOWED_RELS`.
pub(crate) const MEMBER_OF_SOURCE: &str = "member_of_source";

/// Resolved `<repo>:<path>` reference to a `documents` row: `memory →
/// document` for a backtick mention, `document → document` for a resolved
/// Markdown link. Bare ids on both sides. See [`crate::graph::doc_link`].
pub(crate) const REFERENCES_DOCUMENT: &str = "references_document";

/// Addressing tuple for a single directed edge.
///
/// Node identifiers follow the v0.2 convention documented in
/// `src/store/sql/0002_v2_tables.sql`:
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

/// Graph node id for a file: `file:<repo>:<path>` — the addressing
/// convention pinned in `src/store/sql/0002_v2_tables.sql` and used by
/// every graph-side writer/reader (`materialize`, the working set, the
/// affinity prior).
///
/// KNOWN pre-existing divergence: [`crate::graph::cross_link`]'s
/// `extract_and_emit` writes `references_file` / `references_symbol`
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
/// wipe, because `prune::low_value::superseded_rule` compares the target's
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
    let w: Option<i64> = conn
        .query_row(
            "SELECT weight FROM edges \
              WHERE src_kind=?1 AND src_id=?2 AND dst_kind=?3 AND dst_id=?4 AND rel=?5",
            params![e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel],
            |r| r.get(0),
        )
        .optional()?;
    Ok(w.unwrap_or(0))
}

/// Outgoing neighbors of `(src_kind, src_id)` following `rel`.
pub fn outgoing(
    conn: &Connection,
    src_kind: &str,
    src_id: &str,
    rel: &str,
) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT dst_kind, dst_id FROM edges \
          WHERE src_kind = ?1 AND src_id = ?2 AND rel = ?3",
    )?;
    let rows = stmt
        .query_map(params![src_kind, src_id, rel], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
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
    conn.execute(
        "DELETE FROM edges WHERE src_kind = ?1 AND src_id = ?2",
        params![kind, id],
    )?;
    Ok(())
}

/// Delete every edge touching `(kind, id)`, either side. Used by
/// soft-delete.
pub fn delete_touching(conn: &Connection, kind: &str, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM edges WHERE (src_kind = ?1 AND src_id = ?2) OR (dst_kind = ?1 AND dst_id = ?2)",
        params![kind, id],
    )?;
    Ok(())
}

/// One raw `(src_id, dst_id, rel, weight)` row from the code-graph edge set
/// PageRank projects. See [`crate::graph::materialize::project_pagerank`].
pub(crate) struct GraphEdgeRow {
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
    conn.execute(
        "DELETE FROM edges WHERE src_kind='file' AND src_id = ?1 AND rel='imports'",
        [src_id],
    )?;
    Ok(())
}

/// Shared `(String, String, f64)` row fetch for the two memory-graph edge
/// queries below.
fn fetch_weighted_edges(conn: &Connection, sql: &str) -> Result<Vec<(String, String, f64)>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Direct memory→memory relation edges (`supersedes`/`conflicts_with`/
/// `derived_from`/`relates_to`), read in the direction they are stored (src
/// = the newer/building memory). See
/// [`crate::graph::memory_rank::derive_memory_graph`].
pub(crate) fn memory_direct_relation_edges(
    conn: &Connection,
) -> Result<Vec<(String, String, f64)>> {
    fetch_weighted_edges(
        conn,
        "SELECT src_id, dst_id, weight FROM edges \
          WHERE src_kind = 'memory' AND dst_kind = 'memory' \
            AND rel IN ('supersedes','conflicts_with','derived_from','relates_to') \
         ORDER BY rel, src_id, dst_id",
    )
}

/// Co-citation edges: one row per unordered memory pair referencing the
/// same target through the same rel, weighted by the number of shared
/// targets. See [`crate::graph::memory_rank::derive_memory_graph`].
pub(crate) fn memory_co_citation_edges(conn: &Connection) -> Result<Vec<(String, String, f64)>> {
    fetch_weighted_edges(
        conn,
        "SELECT a.src_id, b.src_id, CAST(COUNT(*) AS REAL) AS w \
           FROM edges a \
           JOIN edges b ON a.rel = b.rel AND a.dst_kind = b.dst_kind \
                       AND a.dst_id = b.dst_id AND a.src_id < b.src_id \
          WHERE a.src_kind = 'memory' AND b.src_kind = 'memory' \
            AND a.rel IN ('references_file','references_symbol','co_activated') \
          GROUP BY a.src_id, b.src_id \
          ORDER BY a.src_id, b.src_id",
    )
}

/// Every memory id whose `rel` edge points at `dst_id`
/// (`src_kind='memory'`, `dst_kind='file'`) — resolves a document's identity
/// against pre-existing memory mentions. See
/// [`crate::graph::doc_link::derive_after_document`].
pub(crate) fn memory_ids_referencing_file(
    conn: &Connection,
    rel: &str,
    dst_id: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT src_id FROM edges \
          WHERE rel = ?1 AND src_kind = 'memory' AND dst_kind = 'file' AND dst_id = ?2",
    )?;
    let ids = stmt
        .query_map(params![rel, dst_id], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(ids)
}

/// Reverse batch lookup: every `(src_id, dst_id)` edge with the given `rel`
/// and `dst_kind='file'`, `dst_id` one of `dst_ids` — the per-chunk query
/// behind [`crate::graph::coactivate::referencing_memories`].
pub(crate) fn src_ids_for_dst_ids(
    conn: &Connection,
    rel: &str,
    dst_ids: &[&str],
) -> Result<Vec<(String, String)>> {
    let qmarks = crate::store::qmarks(dst_ids.len());
    let sql = format!(
        "SELECT src_id, dst_id FROM edges \
          WHERE rel = ?1 AND dst_kind = 'file' AND dst_id IN ({qmarks}) \
          ORDER BY dst_id, src_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let bound = std::iter::once(rel).chain(dst_ids.iter().copied());
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bound), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The prefix every file node id carries, mirrored from [`file_node_id`].
const FILE_PREFIX: &str = "file:";

/// 1-based `substr` start that strips [`FILE_PREFIX`] off a file node id,
/// derived from the prefix itself rather than a literal offset that would
/// silently rot if the id grammar changed.
const ID_BODY_START: usize = FILE_PREFIX.len() + 1;

/// One-hop, undirected `imports`/`co_changed` graph query seeded from a set
/// of `file:<repo>:<path>` ids. Not recursive — a single query, self-joined
/// against both edge orientations so a file that imports a seed is found
/// exactly as one a seed imports. `:seeds` is a JSON array bound as a named
/// parameter (never interpolated). Multiple contributions to the same
/// `(repo, path, rel)` neighbor collapse to one row carrying the strongest
/// (`MAX`) weight. See [`crate::graph::neighbors::file_neighbors`].
static NEIGHBOR_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "\
    WITH seeds(id) AS (SELECT value FROM json_each(:seeds)),
    one_hop(rest, rel, weight) AS (
      SELECT substr(e.dst_id, {ID_BODY_START}), e.rel, e.weight FROM edges e JOIN seeds s ON s.id = e.src_id
       WHERE e.src_kind='file' AND e.dst_kind='file' AND e.rel IN ('imports','co_changed')
         AND e.weight >= :min_weight
         AND e.dst_id NOT IN (SELECT id FROM seeds)
      UNION ALL
      SELECT substr(e.src_id, {ID_BODY_START}), e.rel, e.weight FROM edges e JOIN seeds s ON s.id = e.dst_id
       WHERE e.src_kind='file' AND e.dst_kind='file' AND e.rel IN ('imports','co_changed')
         AND e.weight >= :min_weight
         AND e.src_id NOT IN (SELECT id FROM seeds)
    )
    SELECT substr(rest,1,instr(rest,':')-1) AS repo, substr(rest,instr(rest,':')+1) AS path,
           rel, MAX(weight) AS weight
      FROM one_hop WHERE instr(rest,':') > 0
     GROUP BY repo, path, rel ORDER BY weight DESC, rel ASC, path ASC"
    )
});

/// Raw `(repo, path, rel, weight)` rows from [`NEIGHBOR_SQL`]. `seeds_json`
/// must be a JSON array of `file:<repo>:<path>` ids; `min_weight` drops
/// edges below the floor on both orientations.
pub(crate) fn file_neighbor_rows(
    conn: &Connection,
    seeds_json: &str,
    min_weight: i64,
) -> Result<Vec<(String, String, String, i64)>> {
    let mut stmt = conn.prepare(&NEIGHBOR_SQL)?;
    let rows = stmt
        .query_map(
            named_params! { ":seeds": seeds_json, ":min_weight": min_weight },
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/edges.rs"]
mod tests;
