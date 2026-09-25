//! What one pulled entry deserves — spec § Pull, rules 1–7 — decided before
//! anything is written. Rules 8 and 9 (apply, or stall on a refusal) are the
//! pull's own step.
//!
//! The order is the contract. This client's own operation is recognized
//! first; an unreadable kind stalls before anything else is considered; an
//! entry older than what this key last applied or had accepted for its entity
//! is skipped before any apply, which is what makes every rewind safe.

use crate::domains::sync::replica::contract_views::{ChangeEntry, PayloadState};
use crate::domains::sync::replica::manifest;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding;
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_outbox::{self, PendingOperation, Scope};
use crate::store::sync_exchange::ExchangeKey;

/// What to do with one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// Rule 1: this client's own operation — settle it, apply nothing.
    Own,
    /// Rule 2: a kind or schema version this build cannot read — stall.
    Unreadable(String),
    /// Rule 3: older than the entity's synced position — skip.
    Superseded,
    /// Rule 4: its repository is not approved — hold `policy`.
    Unapproved(String),
    /// Rule 5: a pending operation already carries these exact bytes — settle
    /// that operation (the oldest such) and every older one, apply nothing.
    AlreadyUpstream(Vec<String>),
    /// Rule 6: this machine owes a different change to the memory — hold
    /// `pending_local`.
    PendingLocal,
    /// Rule 7: an upsert whose payload is gone upstream — skip.
    PayloadGone,
    /// Rule 8: apply it.
    Apply,
}

/// Classify `entry` for `key` under `policy`, in stream `epoch`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn classify(
    conn: &Connection,
    key: &ExchangeKey,
    policy: &RepositoryPolicy,
    epoch: &str,
    entry: &ChangeEntry,
) -> Result<Rule> {
    if !replica_outbox::read(conn, Scope::Operation(&entry.operation_id), 1)?.is_empty() {
        return Ok(Rule::Own);
    }
    let kind = format!("{}@{}", entry.entity_kind, entry.schema_version);
    if !manifest::advertised().contains(&kind) {
        return Ok(Rule::Unreadable(kind));
    }
    let synced = replica_binding::get(conn, key, &entry.entity_kind, &entry.entity_key)?;
    if synced.is_some_and(|b| {
        b.synced_epoch.as_deref() == Some(epoch)
            && b.synced_sequence.is_some_and(|s| entry.sequence <= s)
    }) {
        return Ok(Rule::Superseded);
    }
    let repository = entry.repository.clone().unwrap_or_default();
    if policy.memory_repository(&repository).is_none() {
        return Ok(Rule::Unapproved(repository));
    }
    if entry.entity_kind == "memory" {
        let pending = replica_outbox::read(
            conn,
            Scope::Entity(&entry.entity_kind, &entry.entity_key),
            usize::MAX,
        )?;
        if let Some(settled) = already_upstream(&pending, entry) {
            return Ok(Rule::AlreadyUpstream(settled));
        }
        if !pending.is_empty() {
            return Ok(Rule::PendingLocal);
        }
    }
    if entry.op != ReplicaOp::Tombstone && entry.payload_state != PayloadState::Present {
        return Ok(Rule::PayloadGone);
    }
    Ok(Rule::Apply)
}

/// The oldest pending operation carrying exactly this entry's bytes, and
/// every pending one older than it — oldest first.
fn already_upstream(pending: &[PendingOperation], entry: &ChangeEntry) -> Option<Vec<String>> {
    let matches = |op: &PendingOperation| {
        if entry.op == ReplicaOp::Tombstone {
            op.op == ReplicaOp::Tombstone
        } else {
            op.op != ReplicaOp::Tombstone && op.payload_digest == entry.payload_digest
        }
    };
    let at = pending.iter().position(matches)?;
    Some(
        pending[..=at]
            .iter()
            .map(|op| op.operation_id.clone())
            .collect(),
    )
}

#[cfg(test)]
#[path = "tests/pull_rules.rs"]
mod tests;
