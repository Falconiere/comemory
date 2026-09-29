//! `project_changes` (#324): the body-free change feed. One id-only row per
//! committed project mutation, appended on the mutation's own connection so it
//! commits or rolls back with it, read back by `seq` cursor.

use rusqlite::Connection;
use toolu_orm::core::query_column::NumericOps;

use super::orm;
use super::schema_projects::{ProjectChanges, project_changes as col};
use crate::prelude::*;

/// One feed row to append.
pub struct NewChange<'a> {
    /// The project that changed.
    pub project_id: &'a str,
    /// The activity event recorded with it; the project id for a deletion.
    pub event_id: &'a str,
    /// Kind of entity the event happened to.
    pub entity_type: &'a str,
    /// `changed` or `deleted`.
    pub op: &'a str,
    /// Epoch milliseconds.
    pub at_ms: i64,
}

/// One stored row, minus its timestamp and entity type, which no frame carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRow {
    /// Feed position.
    pub seq: i64,
    /// The project that changed.
    pub project_id: String,
    /// The event that changed it.
    pub event_id: String,
    /// `changed` or `deleted`.
    pub op: String,
}

/// Append `change`.
pub fn append(conn: &Connection, change: &NewChange<'_>) -> Result<()> {
    orm::execute(
        conn,
        ProjectChanges::insert()
            .set(&col::project_id, change.project_id)
            .set(&col::event_id, change.event_id)
            .set(&col::entity_type, change.entity_type)
            .set(&col::op, change.op)
            .set(&col::created_at, change.at_ms)
            .to_sql(),
    )?;
    Ok(())
}

/// Up to `limit` rows with `seq > after`, ascending.
pub fn page(conn: &Connection, after: i64, limit: i64) -> Result<Vec<ChangeRow>> {
    orm::query_all(
        conn,
        ProjectChanges::select()
            .columns_typed(&[&col::seq, &col::project_id, &col::event_id, &col::op])
            .filter(col::seq.gt(after))
            .order_by(col::seq.asc())
            .limit(limit)
            .to_sql(),
        |r| {
            Ok(ChangeRow {
                seq: r.get(0)?,
                project_id: r.get(1)?,
                event_id: r.get(2)?,
                op: r.get(3)?,
            })
        },
    )
}

/// The oldest and newest retained `seq`, or `None` for an empty feed.
pub fn bounds(conn: &Connection) -> Result<Option<(i64, i64)>> {
    let (oldest, head): (Option<i64>, Option<i64>) = orm::query_one(
        conn,
        ProjectChanges::select()
            .column_expr("MIN(seq)", "oldest")
            .column_expr("MAX(seq)", "head")
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(oldest.zip(head))
}

#[cfg(test)]
#[path = "tests/project_changes.rs"]
mod tests;
