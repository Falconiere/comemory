//! `feedback_events` reads and writes that sharing a verdict needs (#254):
//! stamping the event id a journalled verdict earned, the legacy rows the
//! one-time backfill walks, and the ids a purge must erase.
//!
//! Split from [`super::feedback`] to keep that module under the size ceiling.
//! Which verdict may be shared is `domains::learning::feedback_share`'s rule;
//! this module only runs the SQL it asks for.

use rusqlite::Connection;
use toolu_orm::core::query_column::{CommonOps, NumericOps};

use super::orm;
use super::schema_learning::{FeedbackEvents, feedback_events as col};
use crate::prelude::*;
use crate::utilities::telemetry::target;

/// Record the replica event id a verdict was journalled under.
///
/// # Errors
/// Propagates SQLite failures, including a reused id (`uq_feedback_events_event_id`).
pub(crate) fn stamp_event_id(conn: &Connection, row_id: i64, event_id: &str) -> Result<()> {
    orm::execute(
        conn,
        FeedbackEvents::update()
            .set(&col::event_id, event_id)
            .filter(col::id.eq(row_id))
            .to_sql(),
    )?;
    Ok(())
}

/// One verdict recorded here before sharing existed, as the backfill reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVerdict {
    /// Rowid — the backfill cursor.
    pub id: i64,
    /// The `q-…` id the verdict cites.
    pub query_id: String,
    /// Memory id judged.
    pub memory_id: String,
    /// `used` or `irrelevant`.
    pub verdict: String,
    /// ISO-8601 time of the verdict.
    pub at: String,
    /// Stored provenance.
    pub provenance: String,
}

/// Up to `limit` memory-target verdicts above `after_id` that were recorded
/// here and never journalled, ascending by rowid. Provenance is left to the
/// caller's rule: this is the table-shaped half of the walk.
///
/// # Errors
/// Propagates SQLite failures.
pub fn unshared_legacy_after(
    conn: &Connection,
    after_id: i64,
    limit: usize,
) -> Result<Vec<LegacyVerdict>> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    orm::query_all(
        conn,
        FeedbackEvents::select()
            .columns_typed(&[
                &col::id,
                &col::query_id,
                &col::memory_id,
                &col::verdict,
                &col::at,
                &col::provenance,
            ])
            .filter(col::id.gt(after_id))
            .filter(col::event_id.is_null())
            .filter(col::device.is_null())
            .filter(col::target_kind.eq(target::MEMORY))
            .order_by(col::id.asc())
            .limit(limit)
            .to_sql(),
        |r| {
            Ok(LegacyVerdict {
                id: r.get(0)?,
                query_id: r.get(1)?,
                memory_id: r.get(2)?,
                verdict: r.get(3)?,
                at: r.get(4)?,
                provenance: r.get(5)?,
            })
        },
    )
}

/// The event ids of every journalled verdict on memory `memory_id`, local or
/// imported — what a purge of that memory must erase from the journal.
///
/// # Errors
/// Propagates SQLite failures.
pub(crate) fn event_ids_for_memory(conn: &Connection, memory_id: &str) -> Result<Vec<String>> {
    orm::query_all(
        conn,
        FeedbackEvents::select()
            .columns_typed(&[&col::event_id])
            .filter(col::memory_id.eq(memory_id))
            .filter(col::target_kind.eq(target::MEMORY))
            .filter(col::event_id.is_not_null())
            .to_sql(),
        |r| r.get(0),
    )
}

#[cfg(test)]
#[path = "tests/feedback_share.rs"]
mod tests;
