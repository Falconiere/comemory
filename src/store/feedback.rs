//! `feedback` row CRUD: the per-memory `used`/`irrelevant` counter table,
//! plus the memory-tagged `feedback_events` provenance inserts.
//!
//! The provenance vocabulary (`stats::target`), the query-id contract, and
//! every transaction boundary stay in [`crate::stats::feedback`] — this
//! module owns only the SQL text and its parameter binding. See
//! [`crate::store::code_feedback`] for the code-side sibling table.
//!
//! [`used_query_ids`] and [`used_events_for_golden`] are `feedback_events`
//! reads behind `eval::mine`'s reformulation scan and `eval::golden`'s
//! feedback-harvest, moved here alongside the writers of the same table.

use rusqlite::{Connection, params};

use crate::prelude::*;

/// Upsert the `used` side of the per-memory counter row: insert with
/// `used_count = 1` or bump the existing count, refreshing `last_used` to
/// `now` either way. Accepts any [`Connection`] (a `rusqlite::Transaction`
/// derefs to one).
pub(crate) fn upsert_used(conn: &Connection, id: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used)
             VALUES (?1, 1, 0, ?2)
             ON CONFLICT(memory_id) DO UPDATE SET used_count = used_count + 1, last_used = ?2",
        params![id, now],
    )?;
    Ok(())
}

/// Upsert the `irrelevant` side of the per-memory counter row: insert with
/// `irrelevant_count = 1` or bump the existing count. `last_used` is left
/// untouched — a dismissal is not a use.
pub(crate) fn upsert_irrelevant(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count)
             VALUES (?1, 0, 1)
             ON CONFLICT(memory_id) DO UPDATE SET irrelevant_count = irrelevant_count + 1",
        params![id],
    )?;
    Ok(())
}

/// Insert one memory-tagged `feedback_events` row. `target_kind` and
/// `provenance` are the caller's `crate::stats::{target, feedback}`
/// vocabulary constants, passed explicitly rather than hardcoded (or, for
/// `provenance`, left to the column's `'manual'` default) so this helper
/// stays table-shaped, not domain-shaped. The one INSERT behind every
/// memory-target verdict: manual and HTTP-implicit
/// (`stats::feedback::record_with_provenance`) and the co-activation /
/// search→edit rewards (`stats::feedback::record_implicit_used`).
pub(crate) fn insert_event(
    conn: &Connection,
    query_id: &str,
    id: &str,
    verdict: &str,
    at: &str,
    target_kind: &str,
    provenance: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![query_id, id, verdict, at, target_kind, provenance],
    )?;
    Ok(())
}

/// Distinct `query_id`s carrying at least one `used` verdict of `target_kind`
/// and `provenance` — the "this query succeeded" set behind `eval::mine`'s
/// reformulation scan, which passes `stats::feedback::PROV_MANUAL` so an
/// HTTP-implicit `used` on a real query id never marks a rewording
/// successful.
pub(crate) fn used_query_ids(
    conn: &Connection,
    target_kind: &str,
    provenance: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT query_id FROM feedback_events
          WHERE verdict = 'used' AND target_kind = ?1 AND provenance = ?2",
    )?;
    let ids = stmt
        .query_map([target_kind, provenance], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// One `(query, repo, kind, memory_id)` row from a `used`-verdict
/// `feedback_events` row joined to its originating `retrieval_log` query and
/// a still-live `memories` row — the raw material for `eval::golden`'s
/// feedback harvest.
pub struct GoldenFeedbackRow {
    /// The originating query text.
    pub query: String,
    /// The originating search's repo filter, verbatim.
    pub repo: Option<String>,
    /// The originating search's kind filter, verbatim.
    pub kind: Option<String>,
    /// The memory id marked `used` for this query.
    pub memory_id: String,
}

/// Every `(query, repo, kind, memory_id)` row for a `used` verdict of
/// `target_kind` and `provenance`, excluding `retrieval_log` rows whose
/// `source` is `exclude_source`, restricted to still-live memories. Ordered
/// `(query, repo, kind, memory_id)`, `DISTINCT` (a query/memory pair can
/// carry more than one verdict row across retries). `eval::golden::harvest`
/// passes `stats::feedback::PROV_MANUAL`: only a human-stated verdict is
/// ground truth, so an HTTP-implicit `used` with a real query id — which
/// the JOIN would otherwise admit — never mints a golden pair.
pub fn used_events_for_golden(
    conn: &Connection,
    target_kind: &str,
    exclude_source: &str,
    provenance: &str,
) -> Result<Vec<GoldenFeedbackRow>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.query, r.repo, r.kind, e.memory_id
           FROM feedback_events e
           JOIN retrieval_log r ON r.query_id = e.query_id
           JOIN memories m ON m.id = e.memory_id AND m.deleted_at IS NULL
          WHERE e.verdict = 'used' AND e.target_kind = ?1
            AND r.source != ?2
            AND e.provenance = ?3
          ORDER BY r.query, r.repo, r.kind, e.memory_id",
    )?;
    let rows = stmt
        .query_map([target_kind, exclude_source, provenance], |r| {
            Ok(GoldenFeedbackRow {
                query: r.get(0)?,
                repo: r.get(1)?,
                kind: r.get(2)?,
                memory_id: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// `(total, implicit, used, irrelevant)` over `feedback_events` in one
/// scan, behind `api::learning::summary`'s console header tiles. The three
/// conditional sums are `NULL` on an empty table, read back as `0`.
pub fn event_counts(conn: &Connection) -> Result<(u64, u64, u64, u64)> {
    let row = conn.query_row(
        "SELECT COUNT(*), \
                SUM(CASE WHEN provenance != 'manual' THEN 1 ELSE 0 END), \
                SUM(CASE WHEN verdict = 'used' THEN 1 ELSE 0 END), \
                SUM(CASE WHEN verdict = 'irrelevant' THEN 1 ELSE 0 END) \
           FROM feedback_events",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        },
    )?;
    Ok((
        row.0 as u64,
        row.1.unwrap_or(0) as u64,
        row.2.unwrap_or(0) as u64,
        row.3.unwrap_or(0) as u64,
    ))
}

#[cfg(test)]
#[path = "tests/feedback.rs"]
mod tests;
