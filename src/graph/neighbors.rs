//! `graph::neighbors` — the one-hop undirected `imports`/`co_changed` file
//! neighborhood query shared by `retrieval::bundle` and
//! `GET /api/v1/graph/nodes/{id}/neighbors`.
//!
//! Moved here from `retrieval::bundle`, where it was private, when the
//! console's node-detail panel needed the same walk seeded from ONE file
//! rather than from a memory's resolved code refs. Both callers bind the
//! same statement, so the two surfaces cannot report different neighbors
//! for the same file (console-api spec AC-9).
//!
//! Node-id note: `imports`/`co_changed` file nodes are addressed
//! `file:<repo>:<path>` ([`crate::graph::edges::file_node_id`]) — the BARE
//! `<repo>:<path>` form is what `references_file` memory edges use instead
//! (see that function's divergence doc). The seed ids built here use the
//! `file:` form to match the code-graph convention.

use std::collections::BTreeSet;

use rusqlite::{Connection, named_params};
use serde::Serialize;

use crate::graph::edges::file_node_id;
use crate::prelude::*;

/// The weight floor that keeps every edge: `imports` edges always carry
/// weight `1`, so this is "no filtering". It is what `retrieval::bundle`
/// passes, preserving `comemory context`'s pre-extraction behavior exactly,
/// and what `GET /graph/nodes/{id}/neighbors` falls back to when the caller
/// omits `min_weight`.
pub const DEFAULT_MIN_WEIGHT: i64 = 1;

/// One file reached from a seed file by a one-hop `imports` or
/// `co_changed` edge.
///
/// Re-exported as `retrieval::bundle::NeighborRow`, which is the name the
/// `comemory context` JSON contract was written against; the field set and
/// order (and therefore the serialized shape) are unchanged by the move.
#[derive(Serialize, Debug)]
pub struct NeighborRow {
    /// Repo-relative path of the neighboring file.
    pub path: String,
    /// Repo the neighboring file lives in.
    pub repo: String,
    /// Relation that reached it: `imports` or `co_changed`.
    pub rel: String,
    /// Edge weight (accumulated co-change count; `1` for imports); the
    /// strongest of possibly several contributing edges when more than one
    /// seed file reaches the same neighbor via the same relation.
    pub weight: i64,
}

/// The prefix every file node id carries, mirrored from
/// [`crate::graph::edges::file_node_id`].
const FILE_PREFIX: &str = "file:";

/// 1-based `substr` start that strips [`FILE_PREFIX`] off a file node id,
/// derived from the prefix itself rather than written out as a literal
/// offset that would silently rot if the id grammar changed.
const ID_BODY_START: usize = FILE_PREFIX.len() + 1;

/// One-hop, undirected `imports`/`co_changed` graph query seeded from a set
/// of `file:<repo>:<path>` ids. Not recursive — a single query, self-joined
/// against both edge orientations (the same undirected-walk idiom
/// [`crate::retrieval::graph_route`] and
/// [`crate::retrieval::code_prior::priors`]'s affinity lookup use) so a file
/// that imports a seed is found exactly as one a seed imports.
///
/// `:seeds` is a JSON array BOUND as a named parameter (never interpolated),
/// matching [`crate::retrieval::graph_route::expand_memory_seeds`]'s own
/// `json_each`-over-a-bound-string pattern. `:min_weight` drops edges below
/// the caller's floor on both orientations.
///
/// Multiple contributions to the same `(repo, path, rel)` neighbor (more than
/// one seed file reaching it via the same relation) collapse to one row
/// carrying the strongest (`MAX`) weight, so the output stays one row per
/// `(file, rel)`. Built once at first use, since [`ID_BODY_START`] is
/// computed rather than literal.
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

/// Query the [`NEIGHBOR_SQL`] one-hop neighborhood of `seeds`, a list of
/// `(repo, path)` pairs that is deduplicated here (callers may pass the
/// same file twice — a memory citing several symbols in one file does).
/// Returns `Ok(vec![])` without touching the database when `seeds` is empty:
/// an empty seed set has no neighborhood, and `json_each` over an empty
/// array yields no rows anyway.
///
/// `min_weight` drops edges whose accumulated weight is below the floor;
/// pass [`DEFAULT_MIN_WEIGHT`] to keep every edge.
pub fn file_neighbors(
    conn: &Connection,
    seeds: &[(&str, &str)],
    min_weight: i64,
) -> Result<Vec<NeighborRow>> {
    let distinct: BTreeSet<(&str, &str)> = seeds.iter().copied().collect();
    if distinct.is_empty() {
        return Ok(Vec::new());
    }
    let seed_ids: Vec<String> = distinct
        .into_iter()
        .map(|(repo, path)| file_node_id(repo, path))
        .collect();
    let seeds_json = serde_json::to_string(&seed_ids)?;
    let mut stmt = conn.prepare(&NEIGHBOR_SQL)?;
    let rows = stmt
        .query_map(
            named_params! { ":seeds": seeds_json, ":min_weight": min_weight },
            |r| {
                Ok(NeighborRow {
                    repo: r.get(0)?,
                    path: r.get(1)?,
                    rel: r.get(2)?,
                    weight: r.get(3)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/neighbors.rs"]
mod tests;
