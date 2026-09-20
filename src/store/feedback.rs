//! `feedback` row CRUD: the per-memory `used`/`irrelevant` counter table,
//! plus the memory-tagged `feedback_events` provenance inserts and reads.
//! The provenance vocabulary lives in `crate::utilities::telemetry`; every
//! transaction boundary stays in
//! [`crate::domains::learning::feedback_tracking`] — this module owns only
//! SQL text and parameter binding. See [`crate::store::code_feedback`] for
//! the code-side sibling table. [`events_since`] backs
//! `domains::learning::recall_status`'s verdict count.

use rusqlite::{Connection, params};
use toolu_orm::core::query_column::CommonOps;

use super::{
    orm,
    schema_learning::{FeedbackEvents, feedback_events as col},
};
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
/// `provenance` are the shared vocabulary constants from
/// `crate::utilities::telemetry` (`target::*` and `PROV_*`), passed
/// explicitly by the caller rather than hardcoded (or, for `provenance`,
/// left to the column's `'manual'` default) so this helper stays
/// table-shaped, not domain-shaped. The one INSERT behind every
/// memory-target verdict: manual and HTTP-implicit
/// (`domains::learning::feedback_tracking::record_with_provenance`) and the
/// co-activation / search→edit rewards
/// (`domains::learning::feedback_tracking::record_implicit_used`).
pub(crate) fn insert_event(
    conn: &Connection,
    query_id: &str,
    id: &str,
    verdict: &str,
    at: &str,
    target_kind: &str,
    provenance: &str,
) -> Result<()> {
    orm::execute(
        conn,
        FeedbackEvents::insert()
            .set(&col::query_id, query_id)
            .set(&col::memory_id, id)
            .set(&col::verdict, verdict)
            .set(&col::at, at)
            .set(&col::target_kind, target_kind)
            .set(&col::provenance, provenance)
            .to_sql(),
    )?;
    Ok(())
}

/// Distinct `query_id`s carrying at least one `used` verdict of `target_kind`
/// and `provenance` — the "this query succeeded" set behind `eval::mine`'s
/// reformulation scan, which passes `utilities::telemetry::PROV_MANUAL` so an
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
/// passes `utilities::telemetry::PROV_MANUAL`: only a human-stated verdict is
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

/// Count of `feedback_events` rows whose `at >= since`, joined to their
/// originating `retrieval_log` row so an optional `repo` filter applies —
/// the verdict count behind `domains::learning::recall_status`'s window
/// report. A `LEFT JOIN`: a verdict with no matching `retrieval_log` row
/// (e.g. a co-activation sentinel query id) still counts when `repo` is
/// `None`, and is excluded — rather than assumed — once a `repo` filter
/// asks a question the row cannot answer. Text `>=` and the join are both
/// unsupported toolu-orm 0.7.0 capabilities, so this stays hand SQL — see
/// `docs/guides/runtime-orm.md`.
pub fn events_since(conn: &Connection, repo: Option<&str>, since: &str) -> Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM feedback_events fe \
           LEFT JOIN retrieval_log rl ON rl.query_id = fe.query_id \
          WHERE fe.at >= ?1 AND (?2 IS NULL OR rl.repo = ?2)",
        params![since, repo],
        |r| r.get(0),
    )?;
    Ok(count as u64)
}

/// One `feedback_events` row as an MCP or CLI caller needs to read it back:
/// which id was judged, how, and under which provenance.
pub struct FeedbackEventRow {
    /// The memory id (or, for code, the symbol identity) judged.
    pub memory_id: String,
    /// `used` or `irrelevant`.
    pub verdict: String,
    /// `manual` for a human-stated verdict, `implicit` (or one of the auto
    /// rewards) for anything inferred — the split the golden harvest keys on.
    pub provenance: String,
}

/// Every verdict recorded against `query_id`, ordered `(memory_id, verdict)`
/// so the read is deterministic. Owned rows: no cursor crosses the store
/// boundary. Backs the provenance assertion in `tests/cli_scenario_mcp.rs`,
/// which must read what actually landed rather than trust the echoed field.
pub fn events_for_query(conn: &Connection, query_id: &str) -> Result<Vec<FeedbackEventRow>> {
    orm::query_all(
        conn,
        FeedbackEvents::select()
            .columns_typed(&[&col::memory_id, &col::verdict, &col::provenance])
            .filter(col::query_id.eq(query_id))
            .order_by(col::memory_id.asc())
            .order_by(col::verdict.asc())
            .to_sql(),
        |row| {
            Ok(FeedbackEventRow {
                memory_id: row.get(0)?,
                verdict: row.get(1)?,
                provenance: row.get(2)?,
            })
        },
    )
}

/// `(total, implicit, used, irrelevant)` over `feedback_events` in one
/// scan, behind `domains::learning::console::summary`'s console header tiles. The three
/// conditional sums are `NULL` on an empty table, read back as `0`.
pub fn event_counts(conn: &Connection) -> Result<(u64, u64, u64, u64)> {
    let row = orm::query_one(
        conn,
        FeedbackEvents::select()
            .column_expr("COUNT(*)", "total")
            .column_expr(
                "SUM(CASE WHEN provenance != 'manual' THEN 1 ELSE 0 END)",
                "implicit",
            )
            .column_expr("SUM(CASE WHEN verdict = 'used' THEN 1 ELSE 0 END)", "used")
            .column_expr(
                "SUM(CASE WHEN verdict = 'irrelevant' THEN 1 ELSE 0 END)",
                "irrelevant",
            )
            .to_sql(),
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
