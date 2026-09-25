//! One push batch: the oldest eligible operations, in the order this device
//! made them, under their original ids and bytes — and what the upstream's
//! answer does to each row.
//!
//! A batch holds at most 500 operations and at most `[sync] max_request_bytes`
//! serialized; one operation over that crosses through staged parts. A batch
//! the upstream refuses as too large is split in halves until it fits, and a
//! single operation it still refuses is recorded refused. A restore whose
//! tombstone is in the same batch ends the batch, so the tombstone's position
//! is known before the restore names it. An operation the upstream leaves
//! unanswered is retried within the pass at most once more; after its second
//! unanswered send it — and every later change to its entity — waits for the
//! next pass.

use std::collections::{BTreeMap, BTreeSet};

use crate::domains::code::replica_payload::CODE_ENTITY_KIND;
use crate::domains::sync::drain::code_capture;
use crate::domains::sync::drain::push_batch::{self, Prepared};
use crate::domains::sync::drain::transport::{Answer, Failure, Transport};
use crate::domains::sync::replica::contract::{
    CursorRef, Disposition, ImportRequest, ImportResponse, OperationResult, PROTOCOL,
};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding::{self, Binding};
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_outbox::{self, Outcome, PendingOperation, Scope};
use crate::store::replica_outbox_hold::{self, Change, Hold, Target};
use crate::store::sync_exchange::ExchangeKey;

/// Operations in one batch at most.
const MAX_OPERATIONS: usize = 500;

/// What a push batch did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pushed {
    /// Operations sent.
    pub sent: u32,
    /// Operations the upstream accepted (or answered `duplicate`).
    pub accepted: u32,
    /// Operations the upstream refused for good.
    pub rejected: u32,
    /// The failure that stopped the batch, if one did.
    pub failure: Option<Failure>,
    /// Operations the upstream answered the batch without answering.
    pub unanswered: Vec<String>,
}

/// Everything a push needs from the pass.
pub struct Push<'a> {
    /// The session key.
    pub key: &'a ExchangeKey,
    /// Replica calls.
    pub transport: &'a Transport,
    /// The pass's policy.
    pub policy: &'a RepositoryPolicy,
    /// The cursor to send, so a replaced stream answers `epoch_mismatch`.
    pub cursor: Option<CursorRef>,
    /// `[sync] max_request_bytes`.
    pub max_bytes: usize,
    /// The upstream stream epoch an acceptance's sequence belongs to.
    pub epoch: Option<&'a str>,
    /// Operations this pass sends no more: already sent twice without an
    /// answer. They, and every later change to the same entity, wait for the
    /// next pass.
    pub resting: &'a BTreeSet<String>,
}

/// Operations the upstream left unanswered this pass, and those that rest.
#[derive(Debug, Default)]
pub struct Unanswered {
    sends: BTreeMap<String, u8>,
    resting: BTreeSet<String>,
}

impl Unanswered {
    /// Count one batch's unanswered operations: one sent twice without an
    /// answer rests until the next pass.
    pub fn note(&mut self, ids: &[String]) {
        for id in ids {
            let sends = self.sends.entry(id.clone()).or_insert(0);
            *sends += 1;
            if *sends >= 2 {
                self.resting.insert(id.clone());
            }
        }
    }

    /// The operations this pass sends no more.
    #[must_use]
    pub const fn resting(&self) -> &BTreeSet<String> {
        &self.resting
    }
}

/// Send one batch of eligible operations.
///
/// # Errors
/// Propagates SQLite and JSON failures; network failures are in
/// [`Pushed::failure`].
pub fn batch(conn: &Connection, push: &Push<'_>, at: &str) -> Result<Pushed> {
    let prepared = collect(conn, push, at)?;
    let mut pushed = Pushed::default();
    if prepared.is_empty() {
        return Ok(pushed);
    }
    pushed.sent = u32::try_from(prepared.len()).unwrap_or(u32::MAX);
    let answer = if prepared.len() == 1 && prepared[0].size > push.max_bytes {
        let part = push.max_bytes / 4;
        push_batch::stage_and_activate(push.transport, &prepared[0], push.cursor.clone(), part)
            .map(|result| vec![result])
    } else {
        send_splitting(push, &prepared)
    };
    match answer {
        Ok(results) => record(conn, push, &prepared, &results, &mut pushed, at)?,
        Err(failure) => {
            let error = format!("{failure:?}");
            for p in &prepared {
                let failed = Outcome::Failed { error: &error };
                replica_outbox::record(conn, &p.row.operation_id, failed, at)?;
            }
            pushed.failure = Some(failure);
        }
    }
    Ok(pushed)
}

/// The next batch: eligible rows in the order made, up to the operation and
/// byte budgets, stopping before a restore whose tombstone is in the batch.
fn collect(conn: &Connection, push: &Push<'_>, at: &str) -> Result<Vec<Prepared>> {
    let mut batch: Vec<Prepared> = Vec::new();
    let mut waiting: BTreeSet<(String, String)> = BTreeSet::new();
    let limit = MAX_OPERATIONS + push.resting.len();
    for row in replica_outbox::read(conn, Scope::Eligible, limit)? {
        let entity = (row.entity_kind.clone(), row.entity_key.clone());
        if push.resting.contains(&row.operation_id) || waiting.contains(&entity) {
            waiting.insert(entity);
            continue;
        }
        if tombstone_ahead(&batch, &row) || bytes(&batch) > push.max_bytes {
            break;
        }
        // A first operation always goes; one over the budget alone crosses
        // through staged parts.
        if let Some(ready) = ready(conn, push, row, at)? {
            if !batch.is_empty() && bytes(&batch) + ready.size > push.max_bytes {
                break;
            }
            batch.push(ready);
        }
    }
    Ok(batch)
}

/// The serialized size of a batch.
fn bytes(batch: &[Prepared]) -> usize {
    batch.iter().map(|p| p.size).sum()
}

/// Whether `row` restores an entity whose tombstone is already in the batch.
fn tombstone_ahead(batch: &[Prepared], row: &PendingOperation) -> bool {
    row.op == ReplicaOp::Restore
        && batch
            .iter()
            .any(|p| p.row.entity_key == row.entity_key && p.row.op == ReplicaOp::Tombstone)
}

/// `row` prepared for sending; one whose payload bytes are gone can never be
/// sent and is refused here for good.
fn ready(
    conn: &Connection,
    push: &Push<'_>,
    row: PendingOperation,
    at: &str,
) -> Result<Option<Prepared>> {
    let operation_id = row.operation_id.clone();
    let prepared = push_batch::prepare(conn, push.key, push.policy, row, at)?;
    if prepared.is_none() {
        let gone = Outcome::Rejected {
            disposition: "payload_missing",
        };
        replica_outbox::record(conn, &operation_id, gone, at)?;
    }
    Ok(prepared)
}

/// Send `prepared` as one envelope, halving it while the upstream refuses the
/// request as too large; a single refused operation answers `rejected_invalid`.
fn send_splitting(push: &Push<'_>, prepared: &[Prepared]) -> Answer<Vec<OperationResult>> {
    let request = ImportRequest {
        protocol: PROTOCOL.to_string(),
        cursor: push.cursor.clone(),
        operations: prepared.iter().map(|p| p.operation.clone()).collect(),
        workspace_id: None,
    };
    match push
        .transport
        .retrying(|t| t.post::<ImportResponse, _>("/v1/sync/replica/import", &request))
    {
        Ok(response) => Ok(response.results),
        Err(Failure::Refused(why)) if prepared.len() > 1 => {
            let (left, right) = prepared.split_at(prepared.len() / 2);
            let mut results = send_splitting(push, left)?;
            results.extend(send_splitting(push, right)?);
            tracing::debug!(%why, "a refused batch was split");
            Ok(results)
        }
        Err(Failure::Refused(why)) => Ok(vec![OperationResult {
            operation_id: prepared[0].operation.operation_id.clone(),
            disposition: Disposition::RejectedInvalid,
            sequence: None,
            payload_digest: prepared[0].operation.payload_digest.clone(),
            reason: Some(why),
        }]),
        Err(failure) => Err(failure),
    }
}

/// Write each answer to its row; bind every accepted entity to the key.
fn record(
    conn: &Connection,
    push: &Push<'_>,
    prepared: &[Prepared],
    results: &[OperationResult],
    pushed: &mut Pushed,
    at: &str,
) -> Result<()> {
    let answered = |p: &&Prepared| {
        results
            .iter()
            .any(|r| r.operation_id == p.operation.operation_id)
    };
    for unanswered in prepared.iter().filter(|p| !answered(p)) {
        let failed = Outcome::Failed {
            error: "the upstream gave no answer for this operation",
        };
        replica_outbox::record(conn, &unanswered.operation.operation_id, failed, at)?;
        pushed
            .unanswered
            .push(unanswered.operation.operation_id.clone());
    }
    for (p, result) in prepared.iter().filter_map(|p| {
        let result = results
            .iter()
            .find(|r| r.operation_id == p.operation.operation_id)?;
        Some((p, result))
    }) {
        let id = p.operation.operation_id.as_str();
        match result.disposition {
            Disposition::Accepted | Disposition::Duplicate => {
                settle(conn, push, p, result, at)?;
                pushed.accepted += 1;
            }
            Disposition::RejectedNotAllowed => {
                let reason = result.reason.as_deref().unwrap_or("rejected_not_allowed");
                let change = Change::Hold(Some((Hold::Policy, reason)));
                replica_outbox_hold::update(conn, Target::Operation(id), change, at)?;
            }
            refused => {
                let rejected = Outcome::Rejected {
                    disposition: refused.as_str(),
                };
                replica_outbox::record(conn, id, rejected, at)?;
                pushed.rejected += 1;
            }
        }
    }
    Ok(())
}

/// An accepted operation: settle the row and bind its entity to the key.
fn settle(
    conn: &Connection,
    push: &Push<'_>,
    p: &Prepared,
    result: &OperationResult,
    at: &str,
) -> Result<()> {
    let accepted = Outcome::Accepted {
        sequence: result.sequence,
        disposition: result.disposition.as_str(),
        epoch: push.epoch,
    };
    replica_outbox::record(conn, &p.operation.operation_id, accepted, at)?;
    if p.operation.entity_kind == CODE_ENTITY_KIND
        && let Some(digest) = p.operation.payload_digest.as_deref()
    {
        code_capture::activate(conn, &p.operation.entity_key, digest, at)?;
    }
    let binding = Binding {
        entity_kind: p.operation.entity_kind.clone(),
        entity_key: p.operation.entity_key.clone(),
        synced_digest: p.operation.payload_digest.clone(),
        synced_deleted: p.operation.op == ReplicaOp::Tombstone,
        synced_sequence: result.sequence,
        synced_epoch: push.epoch.map(str::to_string),
    };
    replica_binding::upsert(conn, push.key, &binding, at)
}

#[cfg(test)]
#[path = "tests/push.rs"]
mod tests;
