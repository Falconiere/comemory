//! Phase 2 of a compacting replay: apply each entity's LAST entry, in position
//! order, so an older revision can never land over a newer one.
//!
//! When this engine already holds the entry's operation — its own operation,
//! or one a receipt already answered — but its local revision differs from
//! the entry's, local state is REWRITTEN to the entry under a fresh id: the
//! replay is repairing what a restored or replaced upstream now says. An
//! entity with a pending local operation is left to that operation (held
//! `pending_local`). Each scratch row is deleted once its entity is resolved.

use crate::domains::sync::drain::pull::Step;
use crate::domains::sync::drain::pull_rules::{self, Rule};
use crate::domains::sync::replica::contract_views::ChangeEntry;
use crate::domains::sync::replica::pulled::{self, Landing, Pulled};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_pull_hold::{self, PullHold};
use crate::store::replica_read;
use crate::store::replica_replay;
use crate::utilities::context::Ctx;

/// How an apply batch ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Scratch rows remain.
    More(u32),
    /// Every entity is resolved.
    Done(u32),
    /// An entity could not be applied; its row stays for the next pass.
    Stalled(i64, String),
}

/// Apply up to one page of scratch rows.
///
/// # Errors
/// Propagates SQLite and JSON failures.
pub fn batch(conn: &mut Connection, step: &Step<'_>, epoch: &str) -> Result<Applied> {
    let key = step.pull.key;
    let mut applied = 0;
    for row in replica_replay::next(conn, key, super::pull::PAGE)? {
        let entry: ChangeEntry = serde_json::from_str(&row.entry_json)?;
        if let Some(reason) = resolve(conn, step, epoch, &entry)? {
            return Ok(Applied::Stalled(entry.sequence, reason));
        }
        let entity = (entry.entity_kind.as_str(), entry.entity_key.as_str());
        replica_replay::clear(conn, key, Some(entity))?;
        applied += 1;
    }
    if applied > 0 {
        let _stale = crate::domains::graph::derived::refresh_derived_best_effort(conn);
    }
    let left = replica_replay::next(conn, key, 1)?;
    Ok(if left.is_empty() {
        Applied::Done(applied)
    } else {
        Applied::More(applied)
    })
}

/// Resolve one entity's last entry; `Some(reason)` on a stall.
fn resolve(
    conn: &mut Connection,
    step: &Step<'_>,
    epoch: &str,
    entry: &ChangeEntry,
) -> Result<Option<String>> {
    match pull_rules::classify(conn, step.pull.key, step.pull.policy, epoch, entry)? {
        Rule::Unreadable(_) => return Ok(Some("incompatible_version".to_string())),
        Rule::Superseded | Rule::PayloadGone => return Ok(None),
        Rule::Unapproved(repository) => {
            return hold(conn, step, epoch, entry, "policy", Some(repository));
        }
        Rule::PendingLocal => return hold(conn, step, epoch, entry, "pending_local", None),
        Rule::Own | Rule::AlreadyUpstream(_) | Rule::Apply => {}
    }
    if !holds_same(conn, entry)? {
        let mut ctx = Ctx::borrowed(step.paths, step.cfg, conn);
        let answer = match pulled::apply(&mut ctx, entry, Landing::Received) {
            // The id is already known here: rewrite to what the stream says.
            Ok(Pulled::Duplicate) => pulled::apply(&mut ctx, entry, Landing::Rewrite),
            Err(_) if owns(ctx.conn()?, entry)? => pulled::apply(&mut ctx, entry, Landing::Rewrite),
            other => other,
        };
        match answer {
            Ok(Pulled::Applied | Pulled::Duplicate) => {}
            Ok(Pulled::Refused(d)) => return Ok(Some(d.as_str().to_string())),
            Err(e) => return Ok(Some(e.to_string())),
        }
    }
    let binding = Binding {
        entity_kind: entry.entity_kind.clone(),
        entity_key: entry.entity_key.clone(),
        synced_digest: entry.payload_digest.clone(),
        synced_deleted: entry.op == ReplicaOp::Tombstone,
        synced_sequence: Some(entry.sequence),
        synced_epoch: Some(epoch.to_string()),
    };
    replica_binding::upsert(conn, step.pull.key, &binding, step.at)?;
    Ok(None)
}

/// Whether local state already is what the entry says.
fn holds_same(conn: &Connection, entry: &ChangeEntry) -> Result<bool> {
    let local = replica_read::revision(conn, &entry.entity_kind, &entry.entity_key)?;
    Ok(match (local, entry.op) {
        (Some(local), ReplicaOp::Tombstone) => local.deleted,
        (Some(local), _) => !local.deleted && local.payload_digest == entry.payload_digest,
        (None, _) => false,
    })
}

/// Whether the entry's operation is one this engine journalled itself.
fn owns(conn: &Connection, entry: &ChangeEntry) -> Result<bool> {
    Ok(replica_read::position_of(conn, &entry.operation_id)?.is_some())
}

/// Hold one entity's last entry with its reason.
fn hold(
    conn: &Connection,
    step: &Step<'_>,
    epoch: &str,
    entry: &ChangeEntry,
    reason: &str,
    repository: Option<String>,
) -> Result<Option<String>> {
    let hold = PullHold {
        from_sequence: entry.sequence,
        to_sequence: entry.sequence,
        reason: reason.to_string(),
        entity_kind: Some(entry.entity_kind.clone()),
        entity_key: Some(entry.entity_key.clone()),
        repository,
        policy_revision: step.pull.managed_revision,
    };
    replica_pull_hold::record(conn, step.pull.key, epoch, &hold, step.at)?;
    Ok(None)
}

#[cfg(test)]
#[path = "tests/replay_apply.rs"]
mod tests;
