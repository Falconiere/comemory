//! One-hop file-neighbor graph query and its SQLite CTE.

use rusqlite::{Connection, named_params};

use crate::prelude::*;

/// The prefix every file node id carries, mirrored from [`super::edges::file_node_id`].
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
/// (`MAX`) weight. See [`crate::domains::graph::neighbors::file_neighbors`].
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
