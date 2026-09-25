//! `replica_cursor` row CRUD — how far this machine has applied one session
//! key's upstream feed, under which stream epoch, and which entry sits at that
//! position.
//!
//! The epoch is stored with the position on purpose: a restored or replaced
//! upstream mints a new epoch, and a cursor taken under the old one must fail
//! loudly instead of reading an empty page as agreement. The anchor catches the
//! restore that kept its epoch: the same sequence naming another operation.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_replica::{ReplicaCursor, replica_cursor as col};
use crate::prelude::*;

/// The entry found at a cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// Upstream position of the entry.
    pub sequence: i64,
    /// The operation the upstream accepted there.
    pub operation_id: String,
}

/// One session key's applied position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// Platform API base, trailing `/` removed.
    pub api_url: String,
    /// Workspace the cursor belongs to.
    pub workspace_id: String,
    /// Upstream stream epoch it is valid under.
    pub stream_epoch: String,
    /// Highest upstream sequence applied here.
    pub applied_sequence: i64,
    /// The entry at `applied_sequence`, when one was seen there.
    pub anchor: Option<Anchor>,
}

/// The stored cursor for `(api_url, workspace_id)`, if this machine has one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn load(conn: &Connection, api_url: &str, workspace_id: &str) -> Result<Option<Cursor>> {
    let row: Option<(String, i64, Option<i64>, Option<String>)> = orm::query_optional(
        conn,
        ReplicaCursor::select()
            .columns_typed(&[
                &col::stream_epoch,
                &col::applied_sequence,
                &col::anchor_sequence,
                &col::anchor_operation_id,
            ])
            .filter(col::api_url.eq(api_url))
            .filter(col::workspace_id.eq(workspace_id))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(row.map(
        |(stream_epoch, applied_sequence, sequence, operation_id)| Cursor {
            api_url: api_url.to_string(),
            workspace_id: workspace_id.to_string(),
            stream_epoch,
            applied_sequence,
            anchor: sequence
                .zip(operation_id)
                .map(|(sequence, operation_id)| Anchor {
                    sequence,
                    operation_id,
                }),
        },
    ))
}

/// Write `cursor`, replacing any stored position for the same key.
///
/// # Errors
/// Propagates SQLite failures.
pub fn save(conn: &Connection, cursor: &Cursor, at: &str) -> Result<()> {
    let anchor_sequence = cursor.anchor.as_ref().map(|a| a.sequence);
    let anchor_operation = cursor.anchor.as_ref().map(|a| a.operation_id.as_str());
    let updated = orm::execute(
        conn,
        ReplicaCursor::update()
            .set(&col::stream_epoch, cursor.stream_epoch.as_str())
            .set(&col::applied_sequence, cursor.applied_sequence)
            .set(&col::anchor_sequence, anchor_sequence)
            .set(&col::anchor_operation_id, anchor_operation)
            .set(&col::updated_at, at)
            .filter(col::api_url.eq(cursor.api_url.as_str()))
            .filter(col::workspace_id.eq(cursor.workspace_id.as_str()))
            .to_sql(),
    )?;
    if updated > 0 {
        return Ok(());
    }
    orm::execute(
        conn,
        ReplicaCursor::insert()
            .set(&col::api_url, cursor.api_url.as_str())
            .set(&col::workspace_id, cursor.workspace_id.as_str())
            .set(&col::stream_epoch, cursor.stream_epoch.as_str())
            .set(&col::applied_sequence, cursor.applied_sequence)
            .set(&col::anchor_sequence, anchor_sequence)
            .set(&col::anchor_operation_id, anchor_operation)
            .set(&col::updated_at, at)
            .to_sql(),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/replica_cursor.rs"]
mod tests;
