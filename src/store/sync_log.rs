//! `sync_log` append + read — the append-only change journal both engines
//! keep for cloud sync. Written only at the API-layer callers
//! (`domains::memories::save` / `delete` / `restore` / `update` / `domains::sync::exchange::import`),
//! never inside `memory_row::insert` (rebuild reuses that writer).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::{
    orm,
    schema_sync::{SyncLog, sync_log as col},
};
use crate::prelude::*;
use toolu_orm::core::query_column::{CommonOps, NumericOps};

/// One sync-log operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOp {
    /// Memory created or frontmatter updated under the same content hash.
    Upsert,
    /// Soft-delete / tombstone.
    Tombstone,
    /// Soft-delete reversed.
    Restore,
}

impl SyncOp {
    /// Wire / SQL literal.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Tombstone => "tombstone",
            Self::Restore => "restore",
        }
    }

    /// Parse a stored literal.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "upsert" => Ok(Self::Upsert),
            "tombstone" => Ok(Self::Tombstone),
            "restore" => Ok(Self::Restore),
            other => Err(Error::Other(format!("unknown sync_log op: {other}"))),
        }
    }
}

/// Whether the entry was produced by a local write or by a sync pull/import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOrigin {
    /// Written by a local API caller — eligible for push.
    Local,
    /// Applied from a peer sync — must not be pushed back.
    Sync,
}

impl SyncOrigin {
    /// Wire / SQL literal.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Sync => "sync",
        }
    }

    /// Parse a stored literal.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "local" => Ok(Self::Local),
            "sync" => Ok(Self::Sync),
            other => Err(Error::Other(format!("unknown sync_log origin: {other}"))),
        }
    }
}

/// One `sync_log` row as returned by [`entries_since`].
#[derive(Debug, Clone, Serialize)]
pub struct SyncLogRow {
    /// Server-assigned sequence (monotonic per database).
    pub seq: i64,
    /// Operation kind.
    pub op: SyncOp,
    /// 8-hex memory id.
    pub memory_id: String,
    /// 64-hex content hash at the time of the op.
    pub content_hash: String,
    /// ISO-8601 timestamp.
    pub at: String,
    /// Local vs sync origin.
    pub origin: SyncOrigin,
}

/// Append one log entry; returns the new `seq`. Caller owns the transaction.
pub fn append(
    conn: &Connection,
    op: SyncOp,
    memory_id: &str,
    content_hash: &str,
    at: &str,
    origin: SyncOrigin,
) -> Result<i64> {
    orm::execute(
        conn,
        SyncLog::insert()
            .set(&col::op, op.as_str())
            .set(&col::memory_id, memory_id)
            .set(&col::content_hash, content_hash)
            .set(&col::at, at)
            .set(&col::origin, origin.as_str())
            .to_sql(),
    )?;
    Ok(conn.last_insert_rowid())
}

/// Highest `seq` in the log, or `0` when empty.
pub fn head_seq(conn: &Connection) -> Result<i64> {
    let seq: Option<i64> = orm::query_one(
        conn,
        SyncLog::select()
            .column_expr("MAX(seq)", "head_seq")
            .to_sql(),
        |r| r.get(0),
    )?;
    Ok(seq.unwrap_or(0))
}

/// How many local-origin entries sit above `since` — what a push still owes
/// the platform.
///
/// Counts `origin = 'local'` only: a pulled entry is journalled too, and
/// counting it would report work that was never this machine's to do.
pub fn pending_local(conn: &Connection, since: i64) -> Result<i64> {
    orm::query_one(
        conn,
        SyncLog::select()
            .filter(col::seq.gt(since))
            .filter(col::origin.eq("local"))
            .to_count_sql(),
        |r| r.get(0),
    )
}

/// Entries with `seq > since`, ordered ascending, capped at `limit`.
///
/// Ignores `origin` — pullers need every entry above the cursor, including
/// console deletes recorded as `local` on the server.
pub fn entries_since(conn: &Connection, since: i64, limit: usize) -> Result<Vec<SyncLogRow>> {
    read_entries(conn, since, limit, false)
}

/// Local-origin entries with `seq > since` (the push outbox), ascending.
pub fn local_entries_since(conn: &Connection, since: i64, limit: usize) -> Result<Vec<SyncLogRow>> {
    read_entries(conn, since, limit, true)
}

/// Read and decode one ordered feed page, optionally restricted to the outbox.
fn read_entries(
    conn: &Connection,
    since: i64,
    limit: usize,
    local_only: bool,
) -> Result<Vec<SyncLogRow>> {
    let query = SyncLog::select()
        .columns_typed(&[
            &col::seq,
            &col::op,
            &col::memory_id,
            &col::content_hash,
            &col::at,
            &col::origin,
        ])
        .filter(col::seq.gt(since))
        .order_by(col::seq.asc())
        .limit(limit as i64);
    let query = if local_only {
        query.filter(col::origin.eq("local"))
    } else {
        query
    };
    let rows = orm::query_all(conn, query.to_sql(), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    rows.into_iter()
        .map(|(seq, op, memory_id, content_hash, at, origin)| {
            Ok(SyncLogRow {
                seq,
                op: SyncOp::parse(&op)?,
                memory_id,
                content_hash,
                at,
                origin: SyncOrigin::parse(&origin)?,
            })
        })
        .collect()
}

/// Newest tombstone `seq` for `memory_id`, if any.
pub fn latest_tombstone_seq(conn: &Connection, memory_id: &str) -> Result<Option<i64>> {
    orm::query_one(
        conn,
        SyncLog::select()
            .column_expr("MAX(seq)", "latest_seq")
            .filter(col::memory_id.eq(memory_id))
            .filter(col::op.eq("tombstone"))
            .to_sql(),
        |r| r.get(0),
    )
}

/// Append local `upsert` rows for live memories that have no `sync_log`
/// entry yet.
///
/// Push only drains `sync_log`. Memories saved before the sync seam (or
/// restored from markdown without a log write) stay invisible until this
/// backfill runs. Idempotent: a memory that already has any log row is
/// left alone. Returns how many rows were inserted.
///
/// # Errors
/// Propagates SQLite failures.
pub fn backfill_missing_local(conn: &Connection) -> Result<u32> {
    let n = conn.execute(
        "INSERT INTO sync_log(op, memory_id, content_hash, at, origin) \
         SELECT 'upsert', m.id, m.content_hash, m.updated_at, 'local' \
         FROM memories m \
         WHERE m.deleted_at IS NULL \
           AND NOT EXISTS ( \
             SELECT 1 FROM sync_log s WHERE s.memory_id = m.id \
           )",
        [],
    )?;
    u32::try_from(n)
        .map_err(|_| Error::Other(format!("backfill row count not representable as u32: {n}")))
}

#[cfg(test)]
#[path = "tests/sync_log.rs"]
mod tests;
