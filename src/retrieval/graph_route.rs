//! Graph-expansion retrieval leg — the third RRF candidate source for
//! memory search.
//!
//! A memory that is *lexically dark* for a query never enters the pool
//! from the lexical or vector legs, even when `edges` already links it to
//! a top hit. [`expand_memory_seeds`] walks outward from the provisional
//! top hits and returns what it reaches as [`Source::Graph`] candidates.
//!
//! The walk is **undirected** — every stored row is traversable both ways,
//! which is what makes `A →references_symbol→ S ←references_symbol← B` a
//! valid two-hop path — bounded by `cfg.retrieval.graph_hops` and
//! restricted to [`ALLOWED_RELS`].

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, named_params};

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::fuse::{self, RankedHit};
use crate::retrieval::router::{RoutedHit, Source};
use crate::retrieval::scope::Filters;
use crate::retrieval::score::LegScores;

/// Relation labels the expansion walk may traverse.
///
/// The hub relations `in_repo`, `authored_by` and `tagged` are deliberately
/// absent: a repo, author or tag node is adjacent to *every* memory
/// carrying it, so admitting one would connect the whole corpus at depth 2
/// and the leg would return noise instead of neighbors. The code-graph
/// relations `co_changed` and `imports` join symbol to symbol and are
/// unreachable from a memory node at the supported depths; the code-side
/// expansion leg is a separate change.
pub const ALLOWED_RELS: [&str; 7] = [
    "supersedes",
    "conflicts_with",
    "derived_from",
    "relates_to",
    "references_file",
    "references_symbol",
    "co_activated",
];

/// Ceiling on the rows the recursive walk itself may materialize. SQLite
/// evaluates a recursive CTE from a queue (breadth-first), so truncation
/// drops the deepest frontier first and stays deterministic for a given
/// database state.
const MAX_WALK: i64 = 4096;

/// Ceiling on the candidates the leg returns before the caller's `pool` is
/// applied. On overflow the nearest hops win, then the lexicographically
/// smallest ids.
const MAX_EXPANDED: usize = 256;

/// [`RoutedHit::tier`] carried by every graph candidate. The tier column
/// describes which *lexical ladder* rung produced a hit (1..=4); a graph
/// candidate never went through the ladder, so it reports 0.
const GRAPH_TIER: u8 = 0;

/// Quoted, comma-joined [`ALLOWED_RELS`] for the `rel IN (…)` predicate.
/// Interpolated into the SQL rather than bound because SQLite cannot
/// parameterize an `IN` list; every value is a compile-time constant of
/// this module, never user input. INVARIANT: [`ALLOWED_RELS`] must never
/// be extended with runtime- or user-supplied strings — interpolation
/// would turn any such value into a SQL injection vector. The list is
/// `const` precisely so the query text stays stable (and cacheable).
fn rel_list() -> String {
    ALLOWED_RELS
        .iter()
        .map(|rel| format!("'{rel}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// The expansion query: one recursive CTE walking outward from the seeds,
/// then a live-`memories` join shaping the survivors into ranked
/// candidates.
///
/// `UNION` (not `UNION ALL`) makes the walk cycle-safe exactly as
/// [`crate::graph::edges::supersedes_chain`] does. The inline `edges`
/// union presents every row in both orientations, turning two forward
/// `references_symbol` edges into one two-hop memory→memory path.
/// `GROUP BY … MIN(depth)` collapses multi-path arrivals to one row at the
/// *shortest* distance; the `memories` join drops dangling destinations.
fn expansion_sql() -> String {
    let rels = rel_list();
    format!(
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
    )
}

/// Walk `edges` outward from `seed_ids` and return the memories reached,
/// ranked `(hops ASC, memory_id ASC)`.
///
/// `seed_ids` are the provisional top hits, already truncated to
/// `cfg.retrieval.graph_seeds` by the caller; they are excluded from the
/// result, so the leg contributes only *new* candidates. Only live
/// `memories` rows survive, `filters` (repo / kind / created-date scope)
/// narrow them as in the other legs, and output is capped at
/// `min(pool, MAX_EXPANDED)`. The walk itself is unfiltered — it may
/// *reach* an out-of-scope memory, and the final join drops it. Score is
/// `1 / (1 + hops)` — informational, since RRF consumes rank order. Empty
/// seeds or a disabled walk return empty without touching the database.
pub fn expand_memory_seeds(
    conn: &Connection,
    cfg: &Config,
    seed_ids: &[String],
    filters: Filters<'_>,
    pool: usize,
) -> Result<Vec<RoutedHit>> {
    if seed_ids.is_empty() || cfg.retrieval.graph_hops == 0 || pool == 0 {
        return Ok(Vec::new());
    }
    // The JSON array is BOUND as the `:seeds` named parameter — never
    // interpolated into the SQL text — so quoting/escaping is entirely
    // serde_json's job and `json_each` only ever parses a bound string.
    // Seed ids are internal 8-hex content hashes from the provisional
    // ranking, not caller-supplied text.
    let seeds = serde_json::to_string(seed_ids)?;
    let window = filters.window();
    let mut stmt = conn.prepare(&expansion_sql())?;
    let rows = stmt
        .query_map(
            named_params! {
                ":seeds": seeds,
                ":hops": i64::from(cfg.retrieval.graph_hops),
                ":max_walk": MAX_WALK,
                ":repo": filters.repo,
                ":kind": filters.kind,
                ":since": window.since,
                ":cutoff": window.cutoff,
                ":cap": pool.min(MAX_EXPANDED) as i64,
            },
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().map(to_routed).collect())
}

/// Shape one `(memory_id, hops)` walk row into a graph-leg [`RoutedHit`].
fn to_routed((memory_id, hops): (String, i64)) -> RoutedHit {
    RoutedHit {
        memory_id,
        score: 1.0 / (1.0 + hops as f32),
        source: Source::Graph,
        tier: GRAPH_TIER,
        // A walk-only candidate never went through a lexical or vector
        // leg — that absence IS the signal, not a gap.
        legs: LegScores::none(),
    }
}

/// Everything [`expand_and_fuse`] needs from a routing arm: the ranking it
/// produces today plus the legs behind it.
pub struct GraphFuse<'a> {
    /// Today's provisional ranking — returned verbatim when the leg is
    /// disabled or expands to nothing.
    pub base: Vec<RoutedHit>,
    /// The non-graph legs, in the order they fuse today.
    pub legs: &'a [&'a [RankedHit]],
    /// [`RoutedHit::source`] every id already present in `legs` keeps.
    pub source: Source,
    /// [`RoutedHit::tier`] every id already present in `legs` keeps.
    pub tier: u8,
    /// Raw per-leg scores by memory id, so a hit the base legs produced
    /// keeps them across the graph re-fusion. An id only the walk found is
    /// absent here and reports [`LegScores::none`] — which is exactly the
    /// "lexically dark" signal the graph leg exists to surface.
    pub leg_scores: &'a HashMap<String, LegScores>,
    /// Repo / kind / created-date filters, applied to the expansion
    /// exactly as to the other legs.
    pub filters: Filters<'a>,
    /// Candidate-pool size; also the fused output's truncation point.
    pub pool: usize,
}

/// Expand from `f.base`'s top hits and fuse the result back in.
///
/// Both early returns hand back `f.base` untouched, so `graph_hops = 0` and
/// an empty expansion take today's code path bit-for-bit — the leg can only
/// ever *add* candidates.
pub fn expand_and_fuse(
    conn: &Connection,
    cfg: &Config,
    f: GraphFuse<'_>,
) -> Result<Vec<RoutedHit>> {
    if cfg.retrieval.graph_hops == 0 {
        return Ok(f.base);
    }
    let seeds: Vec<String> = f
        .base
        .iter()
        .take(cfg.retrieval.graph_seeds)
        .map(|h| h.memory_id.clone())
        .collect();
    let graph = expand_memory_seeds(conn, cfg, &seeds, f.filters, f.pool)?;
    if graph.is_empty() {
        return Ok(f.base);
    }
    Ok(fuse_graph_leg(&f, &graph, cfg.retrieval.rrf_k))
}

/// RRF the graph leg in as one more list, then label: an id any base leg
/// already produced keeps that arm's `(source, tier)`; an id only the walk
/// found is [`Source::Graph`] at [`GRAPH_TIER`].
fn fuse_graph_leg(f: &GraphFuse<'_>, graph: &[RoutedHit], rrf_k: f32) -> Vec<RoutedHit> {
    let known: HashSet<&str> = f
        .legs
        .iter()
        .flat_map(|leg| leg.iter())
        .map(|h| h.memory_id.as_str())
        .collect();
    let graph_ranked: Vec<RankedHit> = graph.iter().map(routed_to_ranked).collect();
    let mut legs: Vec<&[RankedHit]> = f.legs.to_vec();
    legs.push(&graph_ranked);
    fuse::rrf_multi(&legs, f.pool, rrf_k)
        .into_iter()
        .map(|h| {
            let seen = known.contains(h.memory_id.as_str());
            let legs = f
                .leg_scores
                .get(&h.memory_id)
                .copied()
                .unwrap_or_else(LegScores::none);
            RoutedHit {
                source: if seen { f.source } else { Source::Graph },
                tier: if seen { f.tier } else { GRAPH_TIER },
                memory_id: h.memory_id,
                score: h.score,
                legs,
            }
        })
        .collect()
}

/// Strip a [`RoutedHit`] down to the id + score [`fuse`] consumes.
fn routed_to_ranked(h: &RoutedHit) -> RankedHit {
    RankedHit {
        memory_id: h.memory_id.clone(),
        score: h.score,
    }
}

#[cfg(test)]
#[path = "tests/graph_route.rs"]
mod tests;
