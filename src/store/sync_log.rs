//! `sync_log` append + read — the append-only change journal both engines
//! keep for cloud sync. Written only at the API-layer callers
//! (`api::save` / `delete` / `restore` / `update` / `api::sync::import`),
//! never inside `memory_row::insert` (rebuild reuses that writer).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

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
    conn.execute(
        "INSERT INTO sync_log(op, memory_id, content_hash, at, origin) \
         VALUES(?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![op.as_str(), memory_id, content_hash, at, origin.as_str()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Highest `seq` in the log, or `0` when empty.
pub fn head_seq(conn: &Connection) -> Result<i64> {
    let seq: Option<i64> = conn.query_row("SELECT MAX(seq) FROM sync_log", [], |r| r.get(0))?;
    Ok(seq.unwrap_or(0))
}

/// Entries with `seq > since`, ordered ascending, capped at `limit`.
///
/// Ignores `origin` — pullers need every entry above the cursor, including
/// console deletes recorded as `local` on the server.
pub fn entries_since(conn: &Connection, since: i64, limit: usize) -> Result<Vec<SyncLogRow>> {
    let mut stmt = conn.prepare(
        "SELECT seq, op, memory_id, content_hash, at, origin \
         FROM sync_log WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![since, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, op, memory_id, content_hash, at, origin) = row?;
        out.push(SyncLogRow {
            seq,
            op: SyncOp::parse(&op)?,
            memory_id,
            content_hash,
            at,
            origin: SyncOrigin::parse(&origin)?,
        });
    }
    Ok(out)
}

/// Local-origin entries with `seq > since` (the push outbox), ascending.
pub fn local_entries_since(conn: &Connection, since: i64, limit: usize) -> Result<Vec<SyncLogRow>> {
    let mut stmt = conn.prepare(
        "SELECT seq, op, memory_id, content_hash, at, origin \
         FROM sync_log WHERE seq > ?1 AND origin = 'local' \
         ORDER BY seq ASC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![since, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, op, memory_id, content_hash, at, origin) = row?;
        out.push(SyncLogRow {
            seq,
            op: SyncOp::parse(&op)?,
            memory_id,
            content_hash,
            at,
            origin: SyncOrigin::parse(&origin)?,
        });
    }
    Ok(out)
}

/// Newest tombstone `seq` for `memory_id`, if any.
pub fn latest_tombstone_seq(conn: &Connection, memory_id: &str) -> Result<Option<i64>> {
    let seq: Option<i64> = conn.query_row(
        "SELECT MAX(seq) FROM sync_log \
         WHERE memory_id = ?1 AND op = 'tombstone'",
        rusqlite::params![memory_id],
        |r| r.get(0),
    )?;
    Ok(seq)
}

#[cfg(test)]
#[path = "tests/sync_log.rs"]
mod tests;
