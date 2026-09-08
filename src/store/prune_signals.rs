//! The two SQL scans behind `prune::low_value`'s candidate detection:
//! [`quality_and_degree_candidates`] (the low-quality, zero-incoming-edge
//! scan) and [`superseded_and_forgotten`] (the superseded-and-untouched-since
//! scan). The activation/feedback scoring and the grace-day cutoff
//! computation stay in `prune::low_value` — this module owns only the SQL
//! text and its row mapping (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).
//!
//! **Every comparison operator below is load-bearing.** `prune --apply`
//! soft-deletes whatever these two queries return; a flipped `<`/`<=` here
//! silently changes which memories are eligible for deletion.

use rusqlite::Connection;

use crate::prelude::*;

/// One `memories` row scanned by the low-quality/zero-incoming-edge rule,
/// with counts exactly as stored (the caller clamps negatives to `0`).
pub struct SignalCandidate {
    /// Memory id.
    pub id: String,
    /// `memories.access_count`, as stored.
    pub access_count: i64,
    /// `COALESCE(last_accessed, created_at)`.
    pub last_or_created: String,
    /// `COALESCE(feedback.used_count, 0)`.
    pub used_count: i64,
    /// `COALESCE(feedback.irrelevant_count, 0)`.
    pub irrelevant_count: i64,
}

/// Live memories at or below `max_quality` (inclusive) with zero incoming
/// `edges` rows, each paired with the counts the caller's activation/Beta
/// scoring needs. See [`crate::prune::low_value::signal_rule`].
pub fn quality_and_degree_candidates(
    conn: &Connection,
    max_quality: u32,
) -> Result<Vec<SignalCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.access_count, COALESCE(m.last_accessed, m.created_at),
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
           FROM memories m
           LEFT JOIN feedback f ON f.memory_id = m.id
          WHERE m.deleted_at IS NULL
            AND m.quality <= ?1
            AND NOT EXISTS (SELECT 1 FROM edges e
                             WHERE e.dst_kind = 'memory' AND e.dst_id = m.id)",
    )?;
    let rows = stmt
        .query_map([max_quality], |r| {
            Ok(SignalCandidate {
                id: r.get(0)?,
                access_count: r.get(1)?,
                last_or_created: r.get(2)?,
                used_count: r.get(3)?,
                irrelevant_count: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// Ids of live memories superseded by another *live* memory whose supersede
/// edge is older than `cutoff` (`edges.created_at < cutoff`, strict) and
/// which have not been accessed since (`COALESCE(last_accessed, created_at)
/// < edges.created_at`, strict). See
/// [`crate::prune::low_value::superseded_rule`].
pub fn superseded_and_forgotten(conn: &Connection, cutoff: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT old.id FROM memories old
           JOIN edges e ON e.rel = 'supersedes'
                       AND e.src_kind = 'memory'
                       AND e.dst_kind = 'memory' AND e.dst_id = old.id
                       AND e.src_id <> e.dst_id
           JOIN memories newer ON newer.id = e.src_id AND newer.deleted_at IS NULL
          WHERE old.deleted_at IS NULL
            AND COALESCE(old.last_accessed, old.created_at) < e.created_at
            AND e.created_at < ?1",
    )?;
    let ids = stmt
        .query_map([cutoff], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(ids)
}

#[cfg(test)]
#[path = "tests/prune_signals.rs"]
mod tests;
