//! Per-workspace sync cursors (`pulled_seq` / `pushed_seq`) and last-sync
//! timestamp. One row per workspace the device has synced against.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::prelude::*;

/// One `sync_state` row.
#[derive(Debug, Clone, Serialize)]
pub struct SyncStateRow {
    /// Platform workspace id.
    pub workspace_id: String,
    /// API base URL used for this binding.
    pub api_url: String,
    /// Highest server `seq` successfully pulled.
    pub pulled_seq: i64,
    /// Highest local `seq` successfully pushed.
    pub pushed_seq: i64,
    /// ISO-8601 of the last successful sync, if any.
    pub last_sync_at: Option<String>,
}

/// Upsert the workspace row, creating it with zero cursors when absent.
pub fn ensure(conn: &Connection, workspace_id: &str, api_url: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state(workspace_id, api_url, pulled_seq, pushed_seq) \
         VALUES(?1, ?2, 0, 0) \
         ON CONFLICT(workspace_id) DO UPDATE SET api_url = excluded.api_url",
        rusqlite::params![workspace_id, api_url],
    )?;
    Ok(())
}

/// Load one workspace's cursors.
pub fn get(conn: &Connection, workspace_id: &str) -> Result<Option<SyncStateRow>> {
    conn.query_row(
        "SELECT workspace_id, api_url, pulled_seq, pushed_seq, last_sync_at \
         FROM sync_state WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
        |r| {
            Ok(SyncStateRow {
                workspace_id: r.get(0)?,
                api_url: r.get(1)?,
                pulled_seq: r.get(2)?,
                pushed_seq: r.get(3)?,
                last_sync_at: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Advance `pushed_seq` (and optionally stamp `last_sync_at`).
pub fn set_pushed(conn: &Connection, workspace_id: &str, pushed_seq: i64, at: &str) -> Result<()> {
    conn.execute(
        "UPDATE sync_state SET pushed_seq = ?2, last_sync_at = ?3 \
         WHERE workspace_id = ?1",
        rusqlite::params![workspace_id, pushed_seq, at],
    )?;
    Ok(())
}

/// Advance `pulled_seq` (and stamp `last_sync_at`).
pub fn set_pulled(conn: &Connection, workspace_id: &str, pulled_seq: i64, at: &str) -> Result<()> {
    conn.execute(
        "UPDATE sync_state SET pulled_seq = ?2, last_sync_at = ?3 \
         WHERE workspace_id = ?1",
        rusqlite::params![workspace_id, pulled_seq, at],
    )?;
    Ok(())
}

/// List every workspace this device has synced.
pub fn list(conn: &Connection) -> Result<Vec<SyncStateRow>> {
    let mut stmt = conn.prepare(
        "SELECT workspace_id, api_url, pulled_seq, pushed_seq, last_sync_at \
         FROM sync_state ORDER BY workspace_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SyncStateRow {
            workspace_id: r.get(0)?,
            api_url: r.get(1)?,
            pulled_seq: r.get(2)?,
            pushed_seq: r.get(3)?,
            last_sync_at: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/sync_state.rs"]
mod tests;
