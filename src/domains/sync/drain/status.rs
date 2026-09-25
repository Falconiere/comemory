//! The `exchange` block of `comemory sync --action status`: what the key
//! negotiated, its network state, where its cursor is, and every outbox row
//! and pull position it holds, by reason. Read offline — no request.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::domains::sync::AuthFile;
use crate::domains::sync::drain::negotiate::Protocol;
use crate::prelude::*;
use crate::store::replica_outbox_hold;
use crate::store::replica_pull_hold;
use crate::store::sync_exchange::{self, ExchangeKey, ExchangeRow};
use crate::store::{Connection, replica_cursor};

/// Every reason a pull position is held, reported even at zero.
const PULL_HOLDS: [&str; 5] = [
    "policy",
    "pending_local",
    "server_withheld",
    "secret",
    "id_collision",
];

/// The `exchange` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExchangeStatus {
    /// The key's API origin.
    pub api_url: String,
    /// The key's workspace.
    pub workspace: String,
    /// `legacy` or `replica-v1`, once negotiated.
    pub protocol: Option<String>,
    /// `full` or `partial`, once negotiated.
    pub coverage: Option<String>,
    /// Why coverage is partial.
    pub coverage_reason: Option<String>,
    /// `ok`, `backoff`, `auth_suspended` or `protocol_error`.
    pub network: String,
    /// When the next unattended request may go out.
    pub retry_at: Option<String>,
    /// The last failure.
    pub last_error: Option<String>,
    /// Failures in a row since the last success.
    pub consecutive_failures: i64,
    /// The cursor's stream epoch.
    pub stream_epoch: Option<String>,
    /// The cursor's position.
    pub applied_sequence: i64,
    /// The upstream head the last completed pass saw.
    pub upstream_head: Option<i64>,
    /// Whether nothing is owed in either direction (see [`caught_up`]).
    pub caught_up: bool,
    /// The push side.
    pub outbox: OutboxStatus,
    /// The pull side.
    pub pull: PullStatus,
}

/// The push side of the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutboxStatus {
    /// Eligible, never attempted.
    pub pending: i64,
    /// Eligible, attempted before without an answer.
    pub retryable: i64,
    /// Held, by reason.
    pub held: BTreeMap<String, i64>,
    /// Refused for good.
    pub rejected: i64,
}

/// The pull side of the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullStatus {
    /// Held positions, by reason.
    pub held: BTreeMap<String, i64>,
    /// The position the pull stalled before.
    pub stalled_at: Option<i64>,
    /// Why it stalled.
    pub stall_reason: Option<String>,
}

/// The status of the key `auth` names.
///
/// # Errors
/// Propagates SQLite failures.
pub fn status(conn: &Connection, auth: &AuthFile) -> Result<ExchangeStatus> {
    let key = ExchangeKey::new(&auth.api_url, &auth.workspace_id);
    let row = sync_exchange::load(conn, &key)?.unwrap_or_else(|| ExchangeRow::fresh(&key));
    let cursor = replica_cursor::load(conn, &key.api_url, &key.workspace_id)?;
    let outbox = outbox(conn)?;
    let pull = PullStatus {
        held: with_every(&PULL_HOLDS, replica_pull_hold::counts(conn, &key)?),
        stalled_at: row.stall_sequence,
        stall_reason: row.stall_reason.clone(),
    };
    let protocol = Protocol::parse(row.protocol.as_deref());
    let applied_sequence = cursor.as_ref().map_or(0, |c| c.applied_sequence);
    // A legacy key has no replica cursor or upstream head, so it is never
    // caught up in this sense; its legacy cursors are reported beside.
    let caught_up = protocol == Some(Protocol::Replica)
        && caught_up(&row, cursor.is_some().then_some(applied_sequence), &outbox);
    Ok(ExchangeStatus {
        api_url: key.api_url.clone(),
        workspace: key.workspace_id.clone(),
        protocol: protocol.map(|p| p.as_str().to_string()),
        coverage: protocol.map(|p| {
            if p == Protocol::Replica {
                "full"
            } else {
                "partial"
            }
            .to_string()
        }),
        coverage_reason: row.coverage_reason.clone(),
        network: row.network_state.clone(),
        retry_at: row.retry_at.clone(),
        last_error: row.last_error.clone(),
        consecutive_failures: row.consecutive_failures,
        stream_epoch: cursor.map(|c| c.stream_epoch),
        applied_sequence,
        upstream_head: row.upstream_head,
        caught_up,
        outbox,
        pull,
    })
}

/// `caught_up` on a replica key: the cursor equals the head the last
/// completed pass saw, no replay is in progress, nothing eligible is owed, no
/// pull is stalled and the network is `ok`. A key change starts a fresh row
/// and cursor, so it is never inherited; an epoch change restarts the cursor.
#[must_use]
pub fn caught_up(row: &ExchangeRow, applied: Option<i64>, outbox: &OutboxStatus) -> bool {
    row.network_state == "ok"
        && row.replay_state.is_none()
        && row.stall_sequence.is_none()
        && outbox.pending + outbox.retryable == 0
        && applied.is_some_and(|a| Some(a) == row.upstream_head)
}

/// Every outbox row by what the next pass does with it.
fn outbox(conn: &Connection) -> Result<OutboxStatus> {
    let counts = replica_outbox_hold::counts(conn)?;
    Ok(OutboxStatus {
        pending: counts.pending,
        retryable: counts.retryable,
        held: counts
            .held
            .into_iter()
            .map(|(hold, n)| (hold.as_str().to_string(), n))
            .collect(),
        rejected: counts.rejected,
    })
}

/// `counts` with every reason in `reasons` present, zero when absent.
fn with_every(reasons: &[&str], counts: Vec<(String, i64)>) -> BTreeMap<String, i64> {
    let mut map: BTreeMap<String, i64> = reasons.iter().map(|r| ((*r).to_string(), 0)).collect();
    map.extend(counts);
    map
}

#[cfg(test)]
#[path = "tests/status.rs"]
mod tests;
