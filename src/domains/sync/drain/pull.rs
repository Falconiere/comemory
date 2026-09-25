//! One pull page: read the upstream's feed above the cursor and resolve each
//! entry by the rules in [`super::pull_rules`], moving the cursor only past
//! positions actually resolved.
//!
//! The cursor names durable, contiguous handling: an entry is applied, held
//! with its reason, settled as this client's own, or skipped as superseded
//! before the cursor moves past it. A stall leaves the cursor before the entry
//! that stalled. After the page the cursor takes the page's `next_sequence` —
//! the server's raw continuation — so a page a server-side filter emptied
//! still advances; on a managed origin the positions it skipped are recorded
//! as a `server_withheld` hold under the policy revision the request carried.

use crate::config::{Config, Paths};
use crate::domains::code::replica_payload::CODE_ENTITY_KIND;
use crate::domains::sync::drain::code_capture;
use crate::domains::sync::drain::pull_rules::{self, Rule};
use crate::domains::sync::drain::transport::{Failure, Transport};
use crate::domains::sync::replica::contract_views::{ChangeEntry, ChangesResponse};
use crate::domains::sync::replica::pulled::{self, Landing, Pulled};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_cursor::{self, Anchor, Cursor};
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_outbox::{self, Outcome};
use crate::store::replica_pull_hold::{self, PullHold};
use crate::store::sync_exchange::ExchangeKey;
use crate::utilities::context::Ctx;

/// Entries asked for per page.
pub const PAGE: usize = 500;

/// What a pull page did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paged {
    /// Entries applied here.
    pub applied: u32,
    /// Entries held with a reason.
    pub held: u32,
    /// This client's operations settled by the pull.
    pub settled: u32,
    /// Entries skipped (superseded, payload gone).
    pub skipped: u32,
    /// Whether the cursor moved.
    pub advanced: bool,
    /// The position the pull stalled before, and why.
    pub stalled: Option<(i64, String)>,
    /// The head the page reported.
    pub head: i64,
    /// Whether the upstream answered as a replaced stream.
    pub rebootstrap: bool,
    /// The failure that stopped the page, if one did.
    pub failure: Option<Failure>,
}

/// Everything a pull needs from the pass.
pub struct Pull<'a> {
    /// The session key.
    pub key: &'a ExchangeKey,
    /// Replica calls.
    pub transport: &'a Transport,
    /// The pass's policy.
    pub policy: &'a RepositoryPolicy,
    /// The policy revision, when the origin is managed (gaps are then policy).
    pub managed_revision: Option<i64>,
}

/// The pass-level context every entry's resolution reads.
pub struct Step<'a> {
    /// The data directory.
    pub paths: &'a Paths,
    /// The configuration.
    pub cfg: &'a Config,
    /// What the pull needs from the session.
    pub pull: &'a Pull<'a>,
    /// Timestamp for every row this page writes.
    pub at: &'a str,
}

/// Read and resolve one page above `cursor`.
///
/// # Errors
/// Propagates SQLite failures; network failures are in [`Paged::failure`].
pub fn page(conn: &mut Connection, step: &Step<'_>, cursor: &mut Cursor) -> Result<Paged> {
    let mut paged = Paged::default();
    let response = match fetch(step.pull, cursor) {
        Ok(response) => response,
        Err(failure) if failure.replaced_stream() => {
            paged.rebootstrap = true;
            return Ok(paged);
        }
        Err(failure) => {
            paged.failure = Some(failure);
            return Ok(paged);
        }
    };
    paged.head = response.head_sequence;
    if cursor.stream_epoch.is_empty() {
        cursor.stream_epoch.clone_from(&response.stream_epoch);
    } else if cursor.stream_epoch != response.stream_epoch {
        paged.rebootstrap = true;
        return Ok(paged);
    }
    // The page is judged under the snapshot as it stands when the answer
    // arrives, so a repository revoked while the request was in flight is
    // held, not applied.
    let policy = RepositoryPolicy::from_snapshot(conn, step.pull.key)?;
    let pull = Pull {
        policy: &policy,
        ..*step.pull
    };
    walk(
        conn,
        &Step {
            pull: &pull,
            ..*step
        },
        cursor,
        &response,
        &mut paged,
    )?;
    if paged.applied > 0 {
        let _stale = crate::domains::graph::derived::refresh_derived_best_effort(conn);
    }
    Ok(paged)
}

/// `GET changes` above the cursor, under its epoch once it has one.
fn fetch(pull: &Pull<'_>, cursor: &Cursor) -> std::result::Result<ChangesResponse, Failure> {
    let mut query = vec![
        ("since", cursor.applied_sequence.to_string()),
        ("limit", PAGE.to_string()),
    ];
    if !cursor.stream_epoch.is_empty() {
        query.push(("epoch", cursor.stream_epoch.clone()));
    }
    pull.transport
        .retrying(|t| t.get::<ChangesResponse>("/v1/sync/replica/changes", &query))
}

/// Resolve the page's entries in order, then take the raw continuation.
fn walk(
    conn: &mut Connection,
    step: &Step<'_>,
    cursor: &mut Cursor,
    response: &ChangesResponse,
    paged: &mut Paged,
) -> Result<()> {
    for entry in &response.entries {
        record_gap(conn, step.pull, cursor, entry.sequence - 1, step.at)?;
        let epoch = cursor.stream_epoch.clone();
        if let Some(reason) = resolve(conn, step, &epoch, entry, paged)? {
            paged.stalled = Some((entry.sequence, reason));
            return Ok(());
        }
        cursor.applied_sequence = entry.sequence;
        cursor.anchor = Some(Anchor {
            sequence: entry.sequence,
            operation_id: entry.operation_id.clone(),
        });
        replica_cursor::save(conn, cursor, step.at)?;
        paged.advanced = true;
    }
    if let Some(next) = response
        .next_sequence
        .filter(|n| *n > cursor.applied_sequence)
    {
        record_gap(conn, step.pull, cursor, next, step.at)?;
        cursor.applied_sequence = next;
        cursor.anchor = None;
        replica_cursor::save(conn, cursor, step.at)?;
        paged.advanced = true;
    }
    Ok(())
}

/// Record positions a managed server skipped between the cursor and `through`.
fn record_gap(
    conn: &Connection,
    pull: &Pull<'_>,
    cursor: &Cursor,
    through: i64,
    at: &str,
) -> Result<()> {
    let (Some(revision), from) = (pull.managed_revision, cursor.applied_sequence + 1) else {
        return Ok(());
    };
    if through < from {
        return Ok(());
    }
    let hold = PullHold {
        from_sequence: from,
        to_sequence: through,
        reason: "server_withheld".to_string(),
        entity_kind: None,
        entity_key: None,
        repository: None,
        policy_revision: Some(revision),
    };
    replica_pull_hold::record(conn, pull.key, &cursor.stream_epoch, &hold, at)
}

/// Resolve one entry; `Some(reason)` when the pull must stall before it.
fn resolve(
    conn: &mut Connection,
    step: &Step<'_>,
    epoch: &str,
    entry: &ChangeEntry,
    paged: &mut Paged,
) -> Result<Option<String>> {
    let (pull, at) = (step.pull, step.at);
    match pull_rules::classify(conn, pull.key, pull.policy, epoch, entry)? {
        Rule::Own => {
            settle(
                conn,
                std::slice::from_ref(&entry.operation_id),
                "accepted",
                epoch,
                entry,
                at,
            )?;
            paged.settled += 1;
        }
        Rule::AlreadyUpstream(ids) => {
            settle(conn, &ids, "already_upstream", epoch, entry, at)?;
            paged.settled += u32::try_from(ids.len()).unwrap_or(u32::MAX);
        }
        Rule::Unreadable(_) => return Ok(Some("incompatible_version".to_string())),
        Rule::Superseded | Rule::PayloadGone => {
            paged.skipped += 1;
            return Ok(None);
        }
        Rule::Unapproved(repository) => {
            hold(conn, pull, epoch, entry, "policy", Some(repository), at)?;
            paged.held += 1;
            return Ok(None);
        }
        Rule::PendingLocal => {
            hold(conn, pull, epoch, entry, "pending_local", None, at)?;
            paged.held += 1;
            return Ok(None);
        }
        Rule::Apply => {
            if let Some(reason) = apply(conn, step, entry) {
                return Ok(Some(reason));
            }
            paged.applied += 1;
        }
    }
    bind(conn, pull.key, epoch, entry, at)?;
    Ok(None)
}

/// Rules 8 and 9: apply the entry in upstream order; `Some(reason)` when it
/// was refused or failed, and the pull must stall before it.
fn apply(conn: &mut Connection, step: &Step<'_>, entry: &ChangeEntry) -> Option<String> {
    let mut ctx = Ctx::borrowed(step.paths, step.cfg, conn);
    match pulled::apply(&mut ctx, entry, Landing::Received) {
        Ok(Pulled::Applied | Pulled::Duplicate) => None,
        Ok(Pulled::Refused(disposition)) => Some(disposition.as_str().to_string()),
        Err(e) => Some(e.to_string()),
    }
}

/// Settle this client's operations the upstream already holds.
fn settle(
    conn: &Connection,
    ids: &[String],
    disposition: &str,
    epoch: &str,
    entry: &ChangeEntry,
    at: &str,
) -> Result<()> {
    for id in ids {
        let accepted = Outcome::Accepted {
            sequence: Some(entry.sequence),
            disposition,
            epoch: Some(epoch),
        };
        replica_outbox::record(conn, id, accepted, at)?;
    }
    if entry.entity_kind == CODE_ENTITY_KIND
        && let Some(digest) = entry.payload_digest.as_deref()
    {
        code_capture::activate(conn, &entry.entity_key, digest, at)?;
    }
    Ok(())
}

/// Hold one entry's position with its reason.
fn hold(
    conn: &Connection,
    pull: &Pull<'_>,
    epoch: &str,
    entry: &ChangeEntry,
    reason: &str,
    repository: Option<String>,
    at: &str,
) -> Result<()> {
    let hold = PullHold {
        from_sequence: entry.sequence,
        to_sequence: entry.sequence,
        reason: reason.to_string(),
        entity_kind: Some(entry.entity_kind.clone()),
        entity_key: Some(entry.entity_key.clone()),
        repository,
        policy_revision: pull.managed_revision,
    };
    replica_pull_hold::record(conn, pull.key, epoch, &hold, at)
}

/// Record what the upstream now holds for the entry's entity.
fn bind(
    conn: &Connection,
    key: &ExchangeKey,
    epoch: &str,
    entry: &ChangeEntry,
    at: &str,
) -> Result<()> {
    let binding = Binding {
        entity_kind: entry.entity_kind.clone(),
        entity_key: entry.entity_key.clone(),
        synced_digest: entry.payload_digest.clone(),
        synced_deleted: entry.op == ReplicaOp::Tombstone,
        synced_sequence: Some(entry.sequence),
        synced_epoch: Some(epoch.to_string()),
    };
    replica_binding::upsert(conn, key, &binding, at)
}

#[cfg(test)]
#[path = "tests/pull.rs"]
mod tests;
