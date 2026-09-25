//! `sync_exchange` row CRUD — one session key's negotiated protocol, network
//! state and resume markers.
//!
//! The client reads the whole row at the start of a pass, changes the fields
//! the pass decides, and writes the whole row back: every field is a fact about
//! the same key, and a partial writer per field would let two of them disagree
//! about which pass they belong to.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_exchange::{SyncExchange, sync_exchange as col};
use crate::prelude::*;

/// Every column [`decode`] reads, in declaration order.
const ALL: &[&dyn toolu_orm::core::query_column::ColumnRef] = &[
    &col::api_url,
    &col::workspace_id,
    &col::protocol,
    &col::coverage_reason,
    &col::selected_at,
    &col::upgrade_through,
    &col::replay_kind,
    &col::replay_state,
    &col::replay_scan_through,
    &col::replay_target,
    &col::network_state,
    &col::retry_at,
    &col::consecutive_failures,
    &col::last_error,
    &col::suspended_fingerprint,
    &col::upstream_head,
    &col::stall_sequence,
    &col::stall_reason,
    &col::last_session_at,
    &col::last_ok_at,
];

/// The session key every exchange-client row is filed under.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExchangeKey {
    /// Platform API base, trailing `/` removed.
    pub api_url: String,
    /// Workspace the credential is scoped to.
    pub workspace_id: String,
}

impl ExchangeKey {
    /// The key for a credential's API base and workspace, normalizing the
    /// base the same way every client call does.
    #[must_use]
    pub fn new(api_url: &str, workspace_id: &str) -> Self {
        Self {
            api_url: api_url.trim_end_matches('/').to_string(),
            workspace_id: workspace_id.to_string(),
        }
    }
}

/// One session key's exchange state. `None` fields were never set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExchangeRow {
    /// The session key the row belongs to.
    pub key: ExchangeKey,
    /// `legacy` or `replica-v1`.
    pub protocol: Option<String>,
    /// Why coverage is partial on a legacy selection.
    pub coverage_reason: Option<String>,
    /// When the current protocol was selected.
    pub selected_at: Option<String>,
    /// Captured head at the `replica-v1` selection.
    pub upgrade_through: Option<i64>,
    /// `rebootstrap` or `repair:<kind>` while a replay runs.
    pub replay_kind: Option<String>,
    /// `scanning` or `applying` while a replay runs.
    pub replay_state: Option<String>,
    /// Highest position the replay's scan has read.
    pub replay_scan_through: Option<i64>,
    /// Head the replay reads to.
    pub replay_target: Option<i64>,
    /// `ok`, `backoff`, `auth_suspended` or `protocol_error`.
    pub network_state: String,
    /// No request before this RFC3339 time while backing off.
    pub retry_at: Option<String>,
    /// Failed passes in a row.
    pub consecutive_failures: i64,
    /// Last failure detail.
    pub last_error: Option<String>,
    /// `auth.json` fingerprint a `401`/`403` suspended.
    pub suspended_fingerprint: Option<String>,
    /// Upstream head last seen.
    pub upstream_head: Option<i64>,
    /// Position the pull stalled before.
    pub stall_sequence: Option<i64>,
    /// Why the pull stalled.
    pub stall_reason: Option<String>,
    /// When the last pass under this key ended.
    pub last_session_at: Option<String>,
    /// When a pass last reached the network cleanly.
    pub last_ok_at: Option<String>,
}

impl ExchangeRow {
    /// A fresh row for a key that has never exchanged anything.
    #[must_use]
    pub fn fresh(key: &ExchangeKey) -> Self {
        Self {
            key: key.clone(),
            network_state: "ok".to_string(),
            ..Self::default()
        }
    }
}

/// The stored row for `(api_url, workspace_id)`, if any.
///
/// # Errors
/// Propagates SQLite failures.
pub fn load(conn: &Connection, key: &ExchangeKey) -> Result<Option<ExchangeRow>> {
    // A machine holds one row per key it ever synced with: a handful.
    Ok(all(conn)?.into_iter().find(|row| &row.key == key))
}

/// Every stored row, for the status report and the last-used-key rule.
///
/// # Errors
/// Propagates SQLite failures.
pub fn all(conn: &Connection) -> Result<Vec<ExchangeRow>> {
    orm::query_all(
        conn,
        SyncExchange::select().columns_typed(ALL).to_sql(),
        decode,
    )
}

/// Whether this engine is a `replica-v1` client of some upstream — the only
/// case in which an owed upload makes it refuse an import for that entity.
///
/// # Errors
/// Propagates SQLite failures.
pub fn has_replica_upstream(conn: &Connection) -> Result<bool> {
    let rows: i64 = orm::query_one(
        conn,
        SyncExchange::select()
            .filter(col::protocol.eq("replica-v1"))
            .to_count_sql(),
        |r| r.get(0),
    )?;
    Ok(rows > 0)
}

/// Write the whole row, replacing any stored one for the same key, in one
/// statement.
///
/// # Errors
/// Propagates SQLite failures, including a value the `CHECK`s refuse.
pub fn save(conn: &Connection, row: &ExchangeRow, at: &str) -> Result<()> {
    orm::execute(
        conn,
        SyncExchange::insert()
            .or_replace()
            .set(&col::api_url, row.key.api_url.as_str())
            .set(&col::workspace_id, row.key.workspace_id.as_str())
            .set(&col::protocol, row.protocol.as_deref())
            .set(&col::coverage_reason, row.coverage_reason.as_deref())
            .set(&col::selected_at, row.selected_at.as_deref())
            .set(&col::upgrade_through, row.upgrade_through)
            .set(&col::replay_kind, row.replay_kind.as_deref())
            .set(&col::replay_state, row.replay_state.as_deref())
            .set(&col::replay_scan_through, row.replay_scan_through)
            .set(&col::replay_target, row.replay_target)
            .set(&col::network_state, row.network_state.as_str())
            .set(&col::retry_at, row.retry_at.as_deref())
            .set(&col::consecutive_failures, row.consecutive_failures)
            .set(&col::last_error, row.last_error.as_deref())
            .set(
                &col::suspended_fingerprint,
                row.suspended_fingerprint.as_deref(),
            )
            .set(&col::upstream_head, row.upstream_head)
            .set(&col::stall_sequence, row.stall_sequence)
            .set(&col::stall_reason, row.stall_reason.as_deref())
            .set(&col::last_session_at, row.last_session_at.as_deref())
            .set(&col::last_ok_at, row.last_ok_at.as_deref())
            .set(&col::updated_at, at)
            .to_sql(),
    )?;
    Ok(())
}

/// Decode a full `SELECT *` row in declaration order.
fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExchangeRow> {
    Ok(ExchangeRow {
        key: ExchangeKey {
            api_url: r.get("api_url")?,
            workspace_id: r.get("workspace_id")?,
        },
        protocol: r.get("protocol")?,
        coverage_reason: r.get("coverage_reason")?,
        selected_at: r.get("selected_at")?,
        upgrade_through: r.get("upgrade_through")?,
        replay_kind: r.get("replay_kind")?,
        replay_state: r.get("replay_state")?,
        replay_scan_through: r.get("replay_scan_through")?,
        replay_target: r.get("replay_target")?,
        network_state: r.get("network_state")?,
        retry_at: r.get("retry_at")?,
        consecutive_failures: r.get("consecutive_failures")?,
        last_error: r.get("last_error")?,
        suspended_fingerprint: r.get("suspended_fingerprint")?,
        upstream_head: r.get("upstream_head")?,
        stall_sequence: r.get("stall_sequence")?,
        stall_reason: r.get("stall_reason")?,
        last_session_at: r.get("last_session_at")?,
        last_ok_at: r.get("last_ok_at")?,
    })
}

#[cfg(test)]
#[path = "tests/sync_exchange.rs"]
mod tests;
