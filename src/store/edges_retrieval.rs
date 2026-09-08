//! Retrieval-side `edges` reads, moved out of `retrieval::graph_route`,
//! `retrieval::bundle`, `retrieval::code_prior`, and `retrieval::rerank`
//! (spec `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`):
//! the memory graph-expansion walk, the context-bundle relation walk, the
//! working-set co-change affinity sum, and the live-supersede lookup. Kept
//! separate from [`crate::store::edges`] (the CRUD + graph-algorithm home)
//! because these four together would push that file past the 300-line
//! ceiling; every function here is a plain read with no writer counterpart.

use rusqlite::{Connection, OptionalExtension, named_params, params};

use crate::prelude::*;
use crate::store::edges::{REFERENCES_FILE, REFERENCES_SYMBOL};

/// Bound parameters for [`expand_memory_seeds`], bundled into a struct
/// rather than nine positional arguments (`clippy::too_many_arguments`).
pub struct SeedWalk<'a> {
    /// Quoted, comma-joined relation labels for the `rel IN (…)` predicate
    /// (e.g. `'supersedes','conflicts_with',…`) — rendered by the caller
    /// from its own compile-time allowlist and interpolated here exactly
    /// like an SQL `IN` list built from [`crate::store::qmarks`]; no
    /// caller ever puts unvalidated input in this field.
    pub rels_clause: &'a str,
    /// JSON array of seed memory ids, bound (never interpolated).
    pub seeds_json: &'a str,
    /// Maximum hop depth of the walk.
    pub hops: i64,
    /// Ceiling on rows the recursive walk itself may materialize.
    pub max_walk: i64,
    /// Optional exact `memories.repo` filter.
    pub repo: Option<&'a str>,
    /// Optional exact `memories.kind` filter.
    pub kind: Option<&'a str>,
    /// Inclusive `memories.created_at` lower bound.
    pub since: Option<&'a str>,
    /// Inclusive `memories.created_at` upper bound.
    pub cutoff: Option<&'a str>,
    /// Output row cap (`min(pool, MAX_EXPANDED)`, decided by the caller).
    pub cap: i64,
}

/// The expansion query: one recursive CTE walking outward from the seeds,
/// then a live-`memories` join shaping the survivors into ranked candidates.
/// `UNION` (not `UNION ALL`) makes the walk cycle-safe exactly as
/// [`crate::store::edges::supersedes_chain`] does. Returns `(memory_id,
/// hops)` rows ordered `(hops ASC, id ASC)`.
pub fn expand_memory_seeds(conn: &Connection, w: &SeedWalk<'_>) -> Result<Vec<(String, i64)>> {
    let rels = w.rels_clause;
    let sql = format!(
        "WITH RECURSIVE walk(kind, id, depth) AS (
             SELECT 'memory', value, 0 FROM json_each(:seeds)
           UNION
             SELECT n.dst_kind, n.dst_id, w.depth + 1
               FROM walk w
               JOIN (SELECT src_kind, src_id, dst_kind, dst_id, rel FROM edges
                     UNION ALL
                     SELECT dst_kind, dst_id, src_kind, src_id, rel FROM edges) n
                 ON n.src_kind = w.kind AND n.src_id = w.id
              WHERE w.depth < :hops AND n.rel IN ({rels})
              LIMIT :max_walk
         )
         SELECT w.id, MIN(w.depth) AS hops
           FROM walk w
           JOIN memories m ON m.id = w.id
          WHERE w.kind = 'memory' AND w.depth > 0
            AND m.deleted_at IS NULL
            AND (:repo IS NULL OR m.repo = :repo)
            AND (:kind IS NULL OR m.kind = :kind)
            AND (:since IS NULL OR datetime(m.created_at) >= datetime(:since))
            AND (:cutoff IS NULL OR datetime(m.created_at) <= datetime(:cutoff))
            AND w.id NOT IN (SELECT value FROM json_each(:seeds))
          GROUP BY w.id
          ORDER BY hops ASC, w.id ASC
          LIMIT :cap"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            named_params! {
                ":seeds": w.seeds_json,
                ":hops": w.hops,
                ":max_walk": w.max_walk,
                ":repo": w.repo,
                ":kind": w.kind,
                ":since": w.since,
                ":cutoff": w.cutoff,
                ":cap": w.cap,
            },
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One row returned by [`walk_context_edges`]: a directed edge from the graph.
pub struct ContextEdgeRow {
    /// Source node kind.
    pub src_kind: String,
    /// Source node identifier.
    pub src_id: String,
    /// Destination node kind.
    pub dst_kind: String,
    /// Destination node identifier.
    pub dst_id: String,
    /// Relation label.
    pub rel: String,
}

/// Walk `references_file`, `references_symbol`, `relates_to`, and `supersedes`
/// edges starting from `(memory, start_id)` up to `max_depth` hops using a
/// recursive CTE. Returns one [`ContextEdgeRow`] per traversed edge, ordered
/// `(rel, src_kind, src_id, dst_kind, dst_id)`.
pub fn walk_context_edges(
    conn: &Connection,
    start_id: &str,
    max_depth: u32,
) -> Result<Vec<ContextEdgeRow>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE walk(src_kind, src_id, dst_kind, dst_id, rel, depth) AS (
             SELECT e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, 1
               FROM edges e
              WHERE e.src_kind = 'memory' AND e.src_id = ?1
                AND e.rel IN ('references_file','references_symbol','relates_to','supersedes')
             UNION
             SELECT e.src_kind, e.src_id, e.dst_kind, e.dst_id, e.rel, w.depth + 1
               FROM edges e
               JOIN walk w ON e.src_kind = w.dst_kind AND e.src_id = w.dst_id
              WHERE e.rel IN ('references_file','references_symbol','relates_to','supersedes')
                AND w.depth < ?2
         )
         SELECT DISTINCT src_kind, src_id, dst_kind, dst_id, rel \
           FROM walk \
          ORDER BY rel, src_kind, src_id, dst_kind, dst_id",
    )?;
    let rows = stmt
        .query_map(params![start_id, i64::from(max_depth)], |r| {
            Ok(ContextEdgeRow {
                src_kind: r.get(0)?,
                src_id: r.get(1)?,
                dst_kind: r.get(2)?,
                dst_id: r.get(3)?,
                rel: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Total `co_changed` weight between `fid` and the working-set file ids, in
/// either direction (the miner stores one canonical row per undirected
/// pair). Numbered placeholders are reused across both `IN` lists so the
/// parameter vector binds once; `prepare_cached` caches one statement per
/// working-set arity.
///
/// Arity-keyed caching tradeoff: the SQL string (and thus the cache key)
/// embeds the working-set length, so each distinct arity compiles its own
/// statement, and rusqlite's default cache capacity of 16 means fluctuating
/// arities can evict older entries (re-prepare churn, never wrong results).
/// Fine unless affinity shows up in a profile — revisit with arity
/// bucketing (pad the `IN` list to fixed sizes) if it does.
pub fn co_change_weight(conn: &Connection, fid: &str, ws_files: &[String]) -> Result<f64> {
    let marks = (0..ws_files.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT COALESCE(SUM(weight), 0) FROM edges \
          WHERE rel = 'co_changed' AND src_kind = 'file' AND dst_kind = 'file' \
            AND ((src_id = ?1 AND dst_id IN ({marks})) \
              OR (dst_id = ?1 AND src_id IN ({marks})))"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let params =
        rusqlite::params_from_iter(std::iter::once(fid).chain(ws_files.iter().map(String::as_str)));
    let w: i64 = stmt.query_row(params, |r| r.get(0))?;
    Ok(w.max(0) as f64)
}

/// Find the *live* memory that supersedes `id`, if any — earliest by
/// `created_at` (ties on `id`) so repeated replacements report one stable
/// superseder. Soft-deleted superseders don't count; self-edges ignored as
/// defense-in-depth. `prepare_cached` for a per-hit rerank loop.
///
/// `as_of_cutoff` bounds the **superseder's own `created_at`**, never
/// `edges.created_at` — rebuild re-stamps edges, frontmatter `created`
/// survives. The cutoff is `memory_row::iso_format` output; that formatter
/// ↔ SQLite `datetime()` contract is pinned by the mixed-precision store
/// tests and the `--as-of` CLI e2e, so a format drift fails loudly, not
/// silently.
pub fn live_superseder(
    conn: &Connection,
    id: &str,
    as_of_cutoff: Option<&str>,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT e.src_id FROM edges e
           JOIN memories m ON m.id = e.src_id AND m.deleted_at IS NULL
          WHERE e.rel = 'supersedes'
            AND e.src_kind = 'memory' AND e.dst_kind = 'memory' AND e.dst_id = ?1
            AND e.src_id <> e.dst_id
            AND (?2 IS NULL OR datetime(m.created_at) <= datetime(?2))
          ORDER BY datetime(m.created_at) ASC, m.id ASC
          LIMIT 1",
    )?;
    stmt.query_row(params![id, as_of_cutoff], |r| r.get(0))
        .optional()
        .map_err(Error::from)
}

/// Every `(rel, dst_id)` reference edge directly off `memory_id`
/// (`references_file` / `references_symbol` only), ordered `(rel, dst_id)`
/// — `comemory show`'s depth-1 reference read. See [`walk_context_edges`]
/// for the multi-hop version `comemory context` uses.
pub fn direct_reference_edges(conn: &Connection, memory_id: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT rel, dst_id FROM edges \
          WHERE src_kind = 'memory' AND src_id = ?1 AND rel IN (?2, ?3) \
          ORDER BY rel, dst_id",
    )?;
    let rows = stmt
        .query_map(
            params![memory_id, REFERENCES_FILE, REFERENCES_SYMBOL],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/edges_retrieval.rs"]
mod tests;
