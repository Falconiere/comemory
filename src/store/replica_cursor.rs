//! `replica_cursor` row CRUD — how far this machine has applied one
//! workspace's upstream feed, and under which stream epoch.
//!
//! The epoch is stored with the position on purpose: a restored or replaced
//! upstream mints a new epoch, and a cursor taken under the old one must fail
//! loudly instead of reading an empty page as agreement.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_replica::{ReplicaCursor, replica_cursor as col};
use crate::prelude::*;

/// One workspace's applied position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// Workspace the cursor belongs to.
    pub workspace_id: String,
    /// Platform API base it was taken against.
    pub api_url: String,
    /// Upstream stream epoch it is valid under.
    pub stream_epoch: String,
    /// Highest upstream sequence applied here.
    pub applied_sequence: i64,
}

/// The stored cursor for `workspace_id`, if this machine has one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn load(conn: &Connection, workspace_id: &str) -> Result<Option<Cursor>> {
    let row: Option<(String, String, i64)> = orm::query_optional(
        conn,
        ReplicaCursor::select()
            .columns_typed(&[&col::api_url, &col::stream_epoch, &col::applied_sequence])
            .filter(col::workspace_id.eq(workspace_id))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(row.map(|(api_url, stream_epoch, applied_sequence)| Cursor {
        workspace_id: workspace_id.to_string(),
        api_url,
        stream_epoch,
        applied_sequence,
    }))
}

/// Write `cursor`, replacing any stored position for the same workspace.
///
/// # Errors
/// Propagates SQLite failures.
pub fn save(conn: &Connection, cursor: &Cursor, at: &str) -> Result<()> {
    let updated = orm::execute(
        conn,
        ReplicaCursor::update()
            .set(&col::api_url, cursor.api_url.as_str())
            .set(&col::stream_epoch, cursor.stream_epoch.as_str())
            .set(&col::applied_sequence, cursor.applied_sequence)
            .set(&col::updated_at, at)
            .filter(col::workspace_id.eq(cursor.workspace_id.as_str()))
            .to_sql(),
    )?;
    if updated > 0 {
        return Ok(());
    }
    orm::execute(
        conn,
        ReplicaCursor::insert()
            .set(&col::workspace_id, cursor.workspace_id.as_str())
            .set(&col::api_url, cursor.api_url.as_str())
            .set(&col::stream_epoch, cursor.stream_epoch.as_str())
            .set(&col::applied_sequence, cursor.applied_sequence)
            .set(&col::updated_at, at)
            .to_sql(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/replica_cursor.rs"]
mod tests;
