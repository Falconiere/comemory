//! `memory_write_intent` row CRUD — the marker that says a memory write is in
//! flight, written before the markdown moves and cleared in the same
//! transaction as the mirror.
//!
//! The window it closes is the gap between `MemoryStore::write_atomic`
//! renaming the file into place and the mirror transaction committing. A
//! process killed there leaves a memory on disk the database has never seen,
//! owing an upload nobody recorded; the outstanding intent is what lets the
//! next open notice and finish it.
//!
//! [`clear`] takes the caller's connection rather than opening its own, so a
//! mirror transaction that rolls back also un-clears the intent — the write is
//! either finished and forgotten, or unfinished and still recorded.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_memory::{MemoryWriteIntent, memory_write_intent as col};
use crate::prelude::*;

/// What kind of write is in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    /// A save, update or restore placing markdown.
    Write,
    /// A soft delete moving markdown to `.trash/`.
    Delete,
}

impl IntentKind {
    /// The stored token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Delete => "delete",
        }
    }

    /// Parse a stored token.
    fn parse(value: &str) -> Result<Self> {
        match value {
            "write" => Ok(Self::Write),
            "delete" => Ok(Self::Delete),
            other => Err(Error::Other(format!(
                "memory_write_intent.kind holds an unknown value: {other}"
            ))),
        }
    }
}

/// One write the database has not yet seen finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    /// The memory id the write is for.
    pub entity_key: String,
    /// Whether markdown was being placed or trashed.
    pub kind: IntentKind,
    /// Markdown path the write was placing, relative to the data dir.
    pub md_path: String,
    /// The operation the finished write owes the journal.
    pub operation_id: String,
    /// RFC3339 time the intent was recorded.
    pub started_at: String,
}

/// Record that a write for `entity_key` is starting, replacing any earlier
/// intent for the same memory.
///
/// A second write to one id supersedes the first: the table holds work in
/// flight, never a history.
///
/// # Errors
/// Propagates SQLite failures.
pub fn record(conn: &Connection, intent: &Intent) -> Result<()> {
    let updated = orm::execute(
        conn,
        MemoryWriteIntent::update()
            .set(&col::kind, intent.kind.as_str())
            .set(&col::md_path, intent.md_path.as_str())
            .set(&col::operation_id, intent.operation_id.as_str())
            .set(&col::started_at, intent.started_at.as_str())
            .filter(col::entity_key.eq(intent.entity_key.as_str()))
            .to_sql(),
    )?;
    if updated > 0 {
        return Ok(());
    }
    orm::execute(
        conn,
        MemoryWriteIntent::insert()
            .set(&col::entity_key, intent.entity_key.as_str())
            .set(&col::kind, intent.kind.as_str())
            .set(&col::md_path, intent.md_path.as_str())
            .set(&col::operation_id, intent.operation_id.as_str())
            .set(&col::started_at, intent.started_at.as_str())
            .to_sql(),
    )?;
    Ok(())
}

/// Forget the intent for `entity_key`. A no-op when there is none.
///
/// Call this inside the mirror transaction, never after it: clearing in a
/// later transaction would open the same crash window one statement wide.
///
/// # Errors
/// Propagates SQLite failures.
pub fn clear(conn: &Connection, entity_key: &str) -> Result<()> {
    orm::execute(
        conn,
        MemoryWriteIntent::delete()
            .filter(col::entity_key.eq(entity_key))
            .to_sql(),
    )?;
    Ok(())
}

/// Every write still in flight, oldest first.
///
/// # Errors
/// Propagates SQLite failures and an unrecognized stored `kind`.
pub fn outstanding(conn: &Connection) -> Result<Vec<Intent>> {
    let rows: Vec<(String, String, String, String, String)> = orm::query_all(
        conn,
        MemoryWriteIntent::select()
            .columns_typed(&[
                &col::entity_key,
                &col::kind,
                &col::md_path,
                &col::operation_id,
                &col::started_at,
            ])
            .order_by(col::started_at.asc())
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    rows.into_iter()
        .map(|(entity_key, kind, md_path, operation_id, started_at)| {
            Ok(Intent {
                entity_key,
                kind: IntentKind::parse(&kind)?,
                md_path,
                operation_id,
                started_at,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/memory_intent.rs"]
mod tests;
