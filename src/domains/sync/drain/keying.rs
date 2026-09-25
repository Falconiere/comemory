//! Whose data a pending operation is: the session key it belongs to.
//!
//! An operation is stamped with a key the first time a session sends it. A
//! logout, or a login that changes the key, stamps every still-unstamped row
//! with the key being left; a credential replaced by hand is caught at the next
//! session, which stamps the rows made before the last session under the old
//! key ended. A stamped row is only ever sent to its key.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding;
use crate::store::replica_outbox_hold::{self, Change, Target};
use crate::store::sync_exchange::{self, ExchangeKey};

/// Stamp every unstamped pending row with `key` — the key a logout or a
/// switching login is leaving.
///
/// # Errors
/// Propagates SQLite failures.
pub fn stamp_outgoing(conn: &Connection, key: &ExchangeKey, at: &str) -> Result<usize> {
    replica_outbox_hold::update(conn, Target::Unstamped(None), Change::Stamp(key), at)
}

/// When the key used last is not `current`, stamp the unstamped rows made
/// before that key's last session ended with it. Returns how many.
///
/// # Errors
/// Propagates SQLite failures.
pub fn apply_last_used_rule(conn: &Connection, current: &ExchangeKey, at: &str) -> Result<usize> {
    let last = sync_exchange::all(conn)?
        .into_iter()
        .filter_map(|row| row.last_session_at.clone().map(|ended| (ended, row.key)))
        .max();
    match last {
        Some((ended, key)) if &key != current => replica_outbox_hold::update(
            conn,
            Target::Unstamped(Some(&ended)),
            Change::Stamp(&key),
            at,
        ),
        _ => Ok(0),
    }
}

/// The key an entity belongs to when it is bound to some key other than
/// `current` and never to `current` — the most recently bound one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn foreign_owner(
    conn: &Connection,
    current: &ExchangeKey,
    entity_kind: &str,
    entity_key: &str,
) -> Result<Option<ExchangeKey>> {
    let keys = replica_binding::keys_for(conn, entity_kind, entity_key)?;
    if keys.iter().any(|k| k == current) {
        return Ok(None);
    }
    Ok(keys.into_iter().next())
}

#[cfg(test)]
#[path = "tests/keying.rs"]
mod tests;
