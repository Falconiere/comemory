//! Why a pending operation is not sent this pass — decided before anything
//! is sent, written to the row, never to the upstream.
//!
//! Every pass re-decides every pending row, so a hold whose cause is gone
//! (an approval restored, an override recorded, the upgrade horizon reached)
//! clears on its own, and a held row never sits at the head of the queue.
//! `order` keeps one entity's changes in the order made: a row behind a held
//! row for the same entity waits with it; other entities are unaffected.

use std::collections::{BTreeMap, BTreeSet};

use crate::domains::sync::drain::keying;
use crate::domains::sync::redact;
use crate::domains::sync::replica::contract_views::ManifestResponse;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::domains::sync::skip_repos::SkipMatcher;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding;
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_outbox::{self, PendingOperation, Scope};
use crate::store::replica_outbox_hold::{self, Change, Hold, Target};
use crate::store::replica_read;
use crate::store::sync_exchange::ExchangeKey;

/// What the classification needs to know about the pass.
pub struct Context<'a> {
    /// The session key.
    pub key: &'a ExchangeKey,
    /// The policy the pass runs under.
    pub policy: &'a RepositoryPolicy,
    /// `[sync] skip_repos`.
    pub skip: &'a SkipMatcher,
    /// `<kind>@<version>` the upstream advertises.
    pub capabilities: &'a BTreeSet<String>,
    /// When the key selected `replica-v1`, while the pull has not yet reached
    /// the upgrade horizon.
    pub upgrading_since: Option<&'a str>,
}

/// The `<kind>@<version>` the upstream can read. An engine that advertises only the
/// protocol is the #250 engine, which read memories alone.
pub fn capabilities(manifest: &ManifestResponse) -> BTreeSet<String> {
    let mut kinds: BTreeSet<String> = manifest
        .capabilities
        .iter()
        .filter(|c| c.contains('@'))
        .cloned()
        .collect();
    if kinds.is_empty() {
        kinds.insert("memory@1".to_string());
    }
    kinds
}

/// Re-decide the hold of every pending row; returns how many are held.
///
/// # Errors
/// Propagates SQLite failures and a stored hold literal no writer produces.
pub fn classify(conn: &Connection, ctx: &Context<'_>, at: &str) -> Result<usize> {
    let mut held_entities: BTreeSet<(String, String)> = BTreeSet::new();
    let mut tombstones_owed: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut held_count = 0;
    for row in replica_outbox::read(conn, Scope::All, usize::MAX)? {
        let entity = (row.entity_kind.clone(), row.entity_key.clone());
        let hold = if held_entities.contains(&entity) {
            Some((
                Hold::Order,
                "an earlier change to this entity is held".to_string(),
            ))
        } else {
            decide(
                conn,
                ctx,
                &row,
                tombstones_owed.get(&entity).copied().unwrap_or(false),
                at,
            )?
        };
        if row.op == ReplicaOp::Tombstone {
            tombstones_owed.insert(entity.clone(), true);
        }
        if hold.is_some() {
            held_count += 1;
            held_entities.insert(entity);
        }
        let current = row.hold_reason.as_deref().map(Hold::parse).transpose()?;
        let wanted = hold.as_ref().map(|(h, _)| *h);
        if current != wanted || row.hold_detail.as_deref() != hold.as_ref().map(|(_, d)| d.as_str())
        {
            let change = Change::Hold(hold.as_ref().map(|(h, d)| (*h, d.as_str())));
            replica_outbox_hold::update(conn, Target::Operation(&row.operation_id), change, at)?;
        }
    }
    Ok(held_count)
}

/// The hold one row deserves on its own, if any.
fn decide(
    conn: &Connection,
    ctx: &Context<'_>,
    row: &PendingOperation,
    tombstone_owed: bool,
    at: &str,
) -> Result<Option<(Hold, String)>> {
    if let Some(owner) = owner(conn, ctx.key, row, at)? {
        return Ok(Some((Hold::Workspace, owner.workspace_id)));
    }
    let capability = format!("{}@{}", row.entity_kind, row.schema_version);
    if !ctx.capabilities.contains(&capability) {
        return Ok(Some((Hold::Incompatible, capability)));
    }
    let label = row.repository.clone().unwrap_or_default();
    if ctx.skip.is_skipped(&label) {
        return Ok(Some((Hold::SkipRepos, label)));
    }
    if ctx.policy.memory_repository(&label).is_none() {
        return Ok(Some((Hold::Policy, label)));
    }
    if let Some(rule) = secret(conn, row)? {
        return Ok(Some((Hold::Secret, rule)));
    }
    if row.entity_kind == "memory"
        && ctx
            .upgrading_since
            .is_some_and(|since| row.created_at.as_str() <= since)
    {
        return Ok(Some((
            Hold::Upgrade,
            "waiting for the pull to reach the upgrade horizon".into(),
        )));
    }
    if row.op == ReplicaOp::Restore
        && row.observed_sequence.is_none()
        && (tombstone_owed || deletion_position(conn, ctx.key, row)?.is_none())
    {
        return Ok(Some((
            Hold::Order,
            "the deletion this restore names is not known yet".into(),
        )));
    }
    Ok(None)
}

/// The other key this row belongs to, if it belongs to one: its stamp, or —
/// unstamped — the key its entity is bound to. An unstamped row of a foreign
/// entity is stamped with that key here, so it follows its data.
fn owner(
    conn: &Connection,
    key: &ExchangeKey,
    row: &PendingOperation,
    at: &str,
) -> Result<Option<ExchangeKey>> {
    if let (Some(api_url), Some(workspace_id)) = (&row.api_url, &row.workspace_id) {
        let stamped = ExchangeKey {
            api_url: api_url.clone(),
            workspace_id: workspace_id.clone(),
        };
        return Ok((&stamped != key).then_some(stamped));
    }
    let Some(foreign) = keying::foreign_owner(conn, key, &row.entity_kind, &row.entity_key)? else {
        return Ok(None);
    };
    let target = Target::Operation(&row.operation_id);
    replica_outbox_hold::update(conn, target, Change::Stamp(&foreign), at)?;
    Ok(Some(foreign))
}

/// The secret rule a memory's pushed body matches without an override.
fn secret(conn: &Connection, row: &PendingOperation) -> Result<Option<String>> {
    if row.entity_kind != "memory" || row.op == ReplicaOp::Tombstone {
        return Ok(None);
    }
    let Some(bytes) = row
        .payload_digest
        .as_deref()
        .map(|d| replica_read::payload_bytes(conn, d))
        .transpose()?
        .flatten()
    else {
        return Ok(None);
    };
    let payload: serde_json::Value = serde_json::from_str(&bytes)?;
    let body = payload["body"].as_str().unwrap_or_default();
    redact::scan_with_override(conn, &row.entity_key, body)
}

/// The upstream position of the deletion a restore of this entity names —
/// the tombstone this key last applied or had accepted.
///
/// # Errors
/// Propagates SQLite failures.
pub fn deletion_position(
    conn: &Connection,
    key: &ExchangeKey,
    row: &PendingOperation,
) -> Result<Option<i64>> {
    Ok(
        replica_binding::get(conn, key, &row.entity_kind, &row.entity_key)?
            .filter(|b| b.synced_deleted)
            .and_then(|b| b.synced_sequence),
    )
}

#[cfg(test)]
#[path = "tests/push_hold.rs"]
mod tests;
