//! Coming back for held pull positions once their cause is gone.
//!
//! A `policy` hold is due when its repository now resolves; a
//! `server_withheld` range when the policy revision it was read under has
//! changed; a `pending_local` hold when its entity owes nothing any more. The
//! cursor rewinds to one before the earliest due hold and the due holds are
//! dropped; the replay is safe because a superseded entry is skipped before
//! any apply (rule 3) and an applied one answers `duplicate`.

use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_cursor::{self, Cursor};
use crate::store::replica_outbox::{self, Scope};
use crate::store::replica_pull_hold::{self, PullHold, Which};
use crate::store::sync_exchange::ExchangeKey;

/// Rewind `cursor` for every due hold; returns how many came due.
///
/// # Errors
/// Propagates SQLite failures.
pub fn reconsider(
    conn: &Connection,
    key: &ExchangeKey,
    policy: &RepositoryPolicy,
    cursor: &mut Cursor,
    at: &str,
) -> Result<usize> {
    if cursor.stream_epoch.is_empty() {
        return Ok(0);
    }
    let mut due: Vec<PullHold> = Vec::new();
    for hold in replica_pull_hold::list(conn, key, &cursor.stream_epoch)? {
        if is_due(conn, policy, &hold)? {
            due.push(hold);
        }
    }
    let Some(earliest) = due.iter().map(|h| h.from_sequence).min() else {
        return Ok(0);
    };
    for hold in &due {
        let which = Which::At {
            epoch: &cursor.stream_epoch,
            from_sequence: hold.from_sequence,
        };
        replica_pull_hold::drop_holds(conn, key, which)?;
    }
    if earliest <= cursor.applied_sequence {
        cursor.applied_sequence = earliest - 1;
        // The entry at the new position was resolved long ago and is not
        // re-read here; an unknown anchor never reads as a rewritten stream.
        cursor.anchor = None;
        replica_cursor::save(conn, cursor, at)?;
    }
    Ok(due.len())
}

/// Whether one hold's cause is gone.
fn is_due(conn: &Connection, policy: &RepositoryPolicy, hold: &PullHold) -> Result<bool> {
    Ok(match hold.reason.as_str() {
        "policy" => hold
            .repository
            .as_deref()
            .is_some_and(|repo| policy.memory_repository(repo).is_some()),
        "server_withheld" => hold.policy_revision != Some(policy.revision()),
        "pending_local" => match (&hold.entity_kind, &hold.entity_key) {
            (Some(kind), Some(entity)) => {
                replica_outbox::read(conn, Scope::Entity(kind, entity), 1)?.is_empty()
            }
            _ => true,
        },
        _ => false,
    })
}

#[cfg(test)]
#[path = "tests/holds.rs"]
mod tests;
