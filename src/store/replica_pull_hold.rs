//! `replica_pull_hold` row CRUD — upstream positions a pull passed without
//! applying, and why.
//!
//! A hold is what lets the cursor keep meaning "durably handled": the position
//! was not applied, but its reason was recorded in the same pass that moved the
//! cursor past it, so the client can come back for it when the reason is gone.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_exchange::{ReplicaPullHold, replica_pull_hold as col};
use super::sync_exchange::ExchangeKey;
use crate::prelude::*;

/// The `stream_epoch` a legacy key's holds are filed under.
pub const LEGACY_EPOCH: &str = "legacy";

/// One held position or range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullHold {
    /// First held position.
    pub from_sequence: i64,
    /// Last held position.
    pub to_sequence: i64,
    /// `policy`, `pending_local`, `server_withheld`, `secret` or `id_collision`.
    pub reason: String,
    /// Entity kind, for an entry hold.
    pub entity_kind: Option<String>,
    /// Entity key, for an entry hold.
    pub entity_key: Option<String>,
    /// Repository the entry named, for a `policy` hold.
    pub repository: Option<String>,
    /// Policy revision a `server_withheld` range was read under.
    pub policy_revision: Option<i64>,
}

/// Record `hold` for `key` under `epoch`, replacing one at the same position.
///
/// # Errors
/// Propagates SQLite failures, including a reason the schema refuses.
pub fn record(
    conn: &Connection,
    key: &ExchangeKey,
    epoch: &str,
    hold: &PullHold,
    at: &str,
) -> Result<()> {
    orm::execute(
        conn,
        ReplicaPullHold::insert()
            .or_replace()
            .set(&col::api_url, key.api_url.as_str())
            .set(&col::workspace_id, key.workspace_id.as_str())
            .set(&col::stream_epoch, epoch)
            .set(&col::from_sequence, hold.from_sequence)
            .set(&col::to_sequence, hold.to_sequence)
            .set(&col::reason, hold.reason.as_str())
            .set(&col::entity_kind, hold.entity_kind.as_deref())
            .set(&col::entity_key, hold.entity_key.as_deref())
            .set(&col::repository, hold.repository.as_deref())
            .set(&col::policy_revision, hold.policy_revision)
            .set(&col::recorded_at, at)
            .to_sql(),
    )?;
    Ok(())
}

/// Every column [`decode`] reads, in order.
const COLUMNS: &[&dyn toolu_orm::core::query_column::ColumnRef] = &[
    &col::from_sequence,
    &col::to_sequence,
    &col::reason,
    &col::entity_kind,
    &col::entity_key,
    &col::repository,
    &col::policy_revision,
];

/// Every hold `key` has under `epoch`, lowest position first.
///
/// # Errors
/// Propagates SQLite failures.
pub fn list(conn: &Connection, key: &ExchangeKey, epoch: &str) -> Result<Vec<PullHold>> {
    orm::query_all(
        conn,
        ReplicaPullHold::select()
            .columns_typed(COLUMNS)
            .filter(col::api_url.eq(key.api_url.as_str()))
            .filter(col::workspace_id.eq(key.workspace_id.as_str()))
            .filter(col::stream_epoch.eq(epoch))
            .order_by(col::from_sequence.asc())
            .to_sql(),
        decode,
    )
}

/// One hold, in [`list`]'s column order.
fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<PullHold> {
    Ok(PullHold {
        from_sequence: r.get(0)?,
        to_sequence: r.get(1)?,
        reason: r.get(2)?,
        entity_kind: r.get(3)?,
        entity_key: r.get(4)?,
        repository: r.get(5)?,
        policy_revision: r.get(6)?,
    })
}

/// Which holds [`drop_holds`] removes.
#[derive(Debug, Clone, Copy)]
pub enum Which<'a> {
    /// The one starting at `from_sequence` under `epoch` — a hold that came due.
    At {
        /// Epoch the hold was recorded under.
        epoch: &'a str,
        /// Its first position.
        from_sequence: i64,
    },
    /// Every replica hold, whatever its epoch — a rebootstrap, since a replaced
    /// stream may reuse the positions. Legacy holds stay.
    Replica,
}

/// Remove the holds `which` names; returns how many.
///
/// # Errors
/// Propagates SQLite failures.
pub fn drop_holds(conn: &Connection, key: &ExchangeKey, which: Which<'_>) -> Result<usize> {
    let delete = ReplicaPullHold::delete()
        .filter(col::api_url.eq(key.api_url.as_str()))
        .filter(col::workspace_id.eq(key.workspace_id.as_str()));
    let delete = match which {
        Which::At {
            epoch,
            from_sequence,
        } => delete
            .filter(col::stream_epoch.eq(epoch))
            .filter(col::from_sequence.eq(from_sequence)),
        Which::Replica => delete.filter(col::stream_epoch.ne(LEGACY_EPOCH)),
    };
    orm::execute(conn, delete.to_sql())
}

/// `(reason, count)` for every reason `key` holds positions for.
///
/// # Errors
/// Propagates SQLite failures.
pub fn counts(conn: &Connection, key: &ExchangeKey) -> Result<Vec<(String, i64)>> {
    let holds: Vec<(String, i64, i64)> = orm::query_all(
        conn,
        ReplicaPullHold::select()
            .columns_typed(&[&col::reason, &col::from_sequence, &col::to_sequence])
            .filter(col::api_url.eq(key.api_url.as_str()))
            .filter(col::workspace_id.eq(key.workspace_id.as_str()))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut out: Vec<(String, i64)> = Vec::new();
    for (reason, from, to) in holds {
        let positions = to.saturating_sub(from).saturating_add(1);
        match out.iter_mut().find(|(r, _)| *r == reason) {
            Some((_, n)) => *n = n.saturating_add(positions),
            None => out.push((reason, positions)),
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
#[path = "tests/replica_pull_hold.rs"]
mod tests;
