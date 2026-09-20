//! Per-workspace sync cursors (`pulled_seq` / `pushed_seq`) and last-sync
//! timestamp. One row per workspace the device has synced against.

use rusqlite::Connection;
use serde::Serialize;

use super::{
    code_sync, orm, schema_meta,
    schema_sync::{SyncState, sync_state as col},
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

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
    orm::query_optional(
        conn,
        select_rows()
            .filter(col::workspace_id.eq(workspace_id))
            .to_sql(),
        read_row,
    )
}

/// Advance `pushed_seq` (and optionally stamp `last_sync_at`).
pub fn set_pushed(conn: &Connection, workspace_id: &str, pushed_seq: i64, at: &str) -> Result<()> {
    orm::execute(
        conn,
        SyncState::update()
            .set(&col::pushed_seq, pushed_seq)
            .set(&col::last_sync_at, at)
            .filter(col::workspace_id.eq(workspace_id))
            .to_sql(),
    )?;
    Ok(())
}

/// Advance `pulled_seq` (and stamp `last_sync_at`).
pub fn set_pulled(conn: &Connection, workspace_id: &str, pulled_seq: i64, at: &str) -> Result<()> {
    orm::execute(
        conn,
        SyncState::update()
            .set(&col::pulled_seq, pulled_seq)
            .set(&col::last_sync_at, at)
            .filter(col::workspace_id.eq(workspace_id))
            .to_sql(),
    )?;
    Ok(())
}

/// List every workspace this device has synced.
pub fn list(conn: &Connection) -> Result<Vec<SyncStateRow>> {
    orm::query_all(
        conn,
        select_rows().order_by(col::workspace_id.asc()).to_sql(),
        read_row,
    )
}

/// Persist a policy fingerprint and reset memory/code reconciliation state
/// atomically when the server revision or local repository identities change.
///
/// A reset zeroes both cursors and clears `last_sync_at` so a stale stamp
/// cannot outlive the reconciliation.
///
/// Returns `true` when a reset was applied.
pub fn reconcile_policy(
    conn: &mut Connection,
    workspace_id: &str,
    fingerprint: &str,
) -> Result<bool> {
    let key = format!("sync_policy:{workspace_id}");
    if schema_meta::get(conn, &key)?.as_deref() == Some(fingerprint) {
        return Ok(false);
    }
    let tx = conn.transaction()?;
    orm::execute(
        &tx,
        SyncState::update()
            .set(&col::pulled_seq, 0_i64)
            .set(&col::pushed_seq, 0_i64)
            .set_expr(&col::last_sync_at, "NULL")
            .filter(col::workspace_id.eq(workspace_id))
            .to_sql(),
    )?;
    schema_meta::upsert(&tx, &key, fingerprint)?;
    code_sync::clear_cursors(&tx)?;
    tx.commit()?;
    Ok(true)
}

/// Build the shared workspace/cursor projection.
fn select_rows() -> toolu_orm::query::select::SelectBuilder {
    SyncState::select().columns_typed(&[
        &col::workspace_id,
        &col::api_url,
        &col::pulled_seq,
        &col::pushed_seq,
        &col::last_sync_at,
    ])
}

/// Decode the shared workspace/cursor projection.
fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SyncStateRow> {
    Ok(SyncStateRow {
        workspace_id: r.get(0)?,
        api_url: r.get(1)?,
        pulled_seq: r.get(2)?,
        pushed_seq: r.get(3)?,
        last_sync_at: r.get(4)?,
    })
}

#[cfg(test)]
#[path = "tests/sync_state.rs"]
mod tests;
