//! A replaced stream: the upstream's epoch changed, its head fell below the
//! cursor, or the entry at the cursor names another operation.
//!
//! The cursor restarts at `(epoch, 0)`; every pull hold of the key is dropped
//! and every binding's synced position cleared (whatever the epoch — a restore
//! that kept its epoch reuses positions), keeping the digests for verify; and a
//! compacting replay of the new stream begins. Pending operations stay
//! pending. Nothing local is deleted.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding;
use crate::store::replica_cursor::{self, Cursor};
use crate::store::replica_pull_hold::{self, Which};
use crate::store::replica_replay;
use crate::store::sync_exchange::{ExchangeKey, ExchangeRow};

/// The replay kind a rebootstrap runs.
pub const REBOOTSTRAP: &str = "rebootstrap";

/// The key's cursor; a key's first has no epoch yet (the first page names it).
///
/// # Errors
/// Propagates SQLite failures.
pub fn cursor_of(conn: &Connection, key: &ExchangeKey) -> Result<Cursor> {
    Ok(
        replica_cursor::load(conn, &key.api_url, &key.workspace_id)?.unwrap_or_else(|| Cursor {
            api_url: key.api_url.clone(),
            workspace_id: key.workspace_id.clone(),
            stream_epoch: String::new(),
            applied_sequence: 0,
            anchor: None,
        }),
    )
}

/// Begin a rebootstrap onto `epoch`, replaying through `head`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn begin(
    conn: &Connection,
    row: &mut ExchangeRow,
    cursor: &mut Cursor,
    epoch: &str,
    head: i64,
    at: &str,
) -> Result<()> {
    replica_pull_hold::drop_holds(conn, &row.key, Which::Replica)?;
    replica_binding::clear_sequences(conn, &row.key)?;
    replica_replay::clear(conn, &row.key, None)?;
    cursor.stream_epoch = epoch.to_string();
    cursor.applied_sequence = 0;
    cursor.anchor = None;
    replica_cursor::save(conn, cursor, at)?;
    start(row, REBOOTSTRAP, head);
    Ok(())
}

/// Record a replay of `kind` through `target` as in progress, from 0.
pub fn start(row: &mut ExchangeRow, kind: &str, target: i64) {
    row.replay_kind = Some(kind.to_string());
    row.replay_state = Some("scanning".to_string());
    row.replay_scan_through = Some(0);
    row.replay_target = Some(target);
}

/// Clear a finished replay.
pub fn finish(row: &mut ExchangeRow) {
    row.replay_kind = None;
    row.replay_state = None;
    row.replay_scan_through = None;
    row.replay_target = None;
}

#[cfg(test)]
#[path = "tests/rebootstrap.rs"]
mod tests;
