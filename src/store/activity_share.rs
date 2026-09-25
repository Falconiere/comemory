//! `activity_log` reads and writes sharing a run needs (#254): the batch the
//! capture sweep walks, and stamping the event id a journalled run earned.
//!
//! Split from [`super::activity`] to keep that module under the size ceiling.
//! Which run may be shared is `domains::sync::replica::activity_payload`'s
//! rule; this module only runs the SQL it asks for.

use rusqlite::Connection;
use toolu_orm::core::query_column::{CommonOps, NumericOps};

use super::activity::{ActivityRow, row_from_query, select_rows};
use super::orm;
use super::schema_history::{ActivityLog, activity_log as col};
use crate::prelude::*;

/// Up to `limit` runs recorded HERE above `after_id` and not yet shared,
/// ascending by id. An imported run is never returned: re-capturing one is how
/// a replication loop would start. Nor is a run that already carries an event
/// id: the cursor lives in `schema_meta`, which a rebuild does not carry, and
/// re-walking a shared run would mint it a second id peers count again.
///
/// # Errors
/// Propagates SQLite failures.
pub fn capture_batch_after(
    conn: &Connection,
    after_id: i64,
    limit: usize,
) -> Result<Vec<ActivityRow>> {
    orm::query_all(
        conn,
        select_rows()
            .filter(col::id.gt(after_id))
            .filter(col::device.is_null())
            .filter(col::event_id.is_null())
            .order_by(col::id.asc())
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .to_sql(),
        row_from_query,
    )
}

/// Record the replica event id a run was journalled under.
///
/// # Errors
/// Propagates SQLite failures, including a reused id (`uq_activity_log_event_id`).
pub fn stamp_event_id(conn: &Connection, id: i64, event_id: &str) -> Result<()> {
    orm::execute(
        conn,
        ActivityLog::update()
            .set(&col::event_id, event_id)
            .filter(col::id.eq(id))
            .to_sql(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/activity_share.rs"]
mod tests;
