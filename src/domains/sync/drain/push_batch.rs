//! Turning an eligible outbox row into the operation a push sends, and
//! sending one too large for a request through staged parts.
//!
//! The wire fields a first send resolves — the canonical repository, a
//! restore's tombstone position — are persisted on the row before it goes
//! out, so every retry carries exactly what the first attempt carried.

use crate::domains::sync::drain::push_hold;
use crate::domains::sync::drain::transport::{Answer, Failure, Transport};
use crate::domains::sync::exchange::changes::vector_for_memory;
use crate::domains::sync::replica::contract::{CursorRef, Operation, OperationResult, PROTOCOL};
use crate::domains::sync::replica::contract_views::{ActivateRequest, StageRequest, StageResponse};
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_journal::ReplicaOp;
use crate::store::replica_outbox::PendingOperation;
use crate::store::replica_outbox_hold::{self, Change, Target};
use crate::store::replica_read;
use crate::store::sync_exchange::ExchangeKey;

/// One row ready to send.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The outbox row.
    pub row: PendingOperation,
    /// The operation it becomes on the wire.
    pub operation: Operation,
    /// Serialized size of the operation.
    pub size: usize,
}

/// Prepare `row` for sending; `None` when this engine no longer holds the
/// payload bytes the row names (it can never be sent).
///
/// # Errors
/// Propagates SQLite and JSON failures.
pub fn prepare(
    conn: &Connection,
    key: &ExchangeKey,
    policy: &RepositoryPolicy,
    row: PendingOperation,
    at: &str,
) -> Result<Option<Prepared>> {
    let (repository, observed) = wire_fields(conn, (key, policy), &row, at)?;
    let payload = match row.payload_digest.as_deref() {
        Some(digest) => match replica_read::payload_bytes(conn, digest)? {
            Some(bytes) => Some(serde_json::from_str::<serde_json::Value>(&bytes)?),
            None => return Ok(None),
        },
        None => None,
    };
    let operation = Operation {
        operation_id: row.operation_id.clone(),
        entity_kind: row.entity_kind.clone(),
        entity_key: row.entity_key.clone(),
        op: row.op,
        schema_version: row.schema_version,
        payload_digest: row.payload_digest.clone(),
        payload,
        observed_sequence: observed,
        repository,
        vector: vector_of(conn, &row)?,
    };
    let size = serde_json::to_vec(&operation)?.len();
    Ok(Some(Prepared {
        row,
        operation,
        size,
    }))
}

/// Stamp an unstamped row with the key, and fix what the wire carries for it
/// — the canonical repository and, for a restore, the deletion's position —
/// on the row, so a resend carries the same bytes.
fn wire_fields(
    conn: &Connection,
    (key, policy): (&ExchangeKey, &RepositoryPolicy),
    row: &PendingOperation,
    at: &str,
) -> Result<(Option<String>, Option<i64>)> {
    let target = Target::Operation(&row.operation_id);
    if row.api_url.is_none() {
        replica_outbox_hold::update(conn, target, Change::Stamp(key), at)?;
    }
    let repository = match &row.wire_repository {
        Some(repository) => Some(repository.clone()),
        None => row
            .repository
            .as_deref()
            .and_then(|label| policy.memory_repository(label))
            .map(str::to_string),
    };
    let observed = match (row.op, row.observed_sequence) {
        (ReplicaOp::Restore, None) => push_hold::deletion_position(conn, key, row)?,
        (_, observed) => observed,
    };
    if row.wire_repository != repository || row.observed_sequence != observed {
        let change = Change::Wire {
            repository: repository.as_deref(),
            observed_sequence: observed,
        };
        replica_outbox_hold::update(conn, target, change, at)?;
    }
    Ok((repository, observed))
}

/// The local vector, when it belongs to exactly the revision being pushed.
fn vector_of(
    conn: &Connection,
    row: &PendingOperation,
) -> Result<Option<crate::domains::sync::exchange::SyncVector>> {
    if row.entity_kind != "memory" || row.op == ReplicaOp::Tombstone {
        return Ok(None);
    }
    let current = replica_read::revision(conn, "memory", &row.entity_key)?;
    if current.and_then(|r| r.payload_digest) != row.payload_digest {
        return Ok(None);
    }
    vector_for_memory(conn, &row.entity_key)
}

/// Send one operation too large for a request: its payload in parts of at
/// most `part_bytes`, then the activation that publishes it.
pub fn stage_and_activate(
    transport: &Transport,
    prepared: &Prepared,
    cursor: Option<CursorRef>,
    part_bytes: usize,
) -> Answer<OperationResult> {
    let payload = match prepared.operation.payload.as_ref() {
        Some(payload) => serde_json::to_string(payload)
            .map_err(|e| Failure::Protocol(format!("staged payload: {e}")))?,
        None => String::new(),
    };
    let parts = parts(&payload, part_bytes);
    let count = i64::try_from(parts.len()).unwrap_or(i64::MAX);
    for (index, part) in parts.into_iter().enumerate() {
        let request = StageRequest {
            protocol: PROTOCOL.to_string(),
            staging_id: prepared.operation.operation_id.clone(),
            part_index: i64::try_from(index).unwrap_or(i64::MAX),
            part_count: count,
            bytes: part.to_string(),
        };
        transport.retrying(|t| t.post::<StageResponse, _>("/v1/sync/replica/stage", &request))?;
    }
    let activate = ActivateRequest {
        protocol: PROTOCOL.to_string(),
        staging_id: prepared.operation.operation_id.clone(),
        operation: Operation {
            payload: None,
            ..prepared.operation.clone()
        },
        cursor,
    };
    transport.retrying(|t| t.post("/v1/sync/replica/activate", &activate))
}

/// `text` cut into parts of at most `max_bytes` bytes each (four at least,
/// the widest character), never inside a character.
pub(crate) fn parts(text: &str, max_bytes: usize) -> Vec<&str> {
    let max_bytes = max_bytes.max(4);
    let mut parts = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = rest.len().min(max_bytes);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (part, tail) = rest.split_at(end);
        parts.push(part);
        rest = tail;
    }
    parts
}

#[cfg(test)]
#[path = "tests/push_batch.rs"]
mod tests;
