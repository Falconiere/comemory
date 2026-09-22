//! The journal rows every memory mutation owes — the legacy `sync_log` entry
//! and the `replica-v1` feed position, payload and outbox row — written
//! inside the caller's mirror transaction.
//!
//! One helper for all four seams (`save`, `update`, `restore`, `delete`) so no
//! surface journals a different shape than its siblings, and both feeds are
//! written together so they cannot describe different histories. A failure
//! here aborts the mutation's transaction: a memory that exists locally but
//! owes no upload is the divergence this journal exists to prevent.

use time::OffsetDateTime;

use crate::domains::memories::Frontmatter;
use crate::domains::memories::replica_payload::{
    MEMORY_ENTITY_KIND, MEMORY_PAYLOAD_VERSION, MemoryPayloadV1,
};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_journal::{
    self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin, stream_epoch,
};
use crate::store::{replica_outbox, sync_log};
use crate::utilities::dated_id::dated_id;

/// Where one journalled mutation landed in each feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Positions {
    /// `sync_log.seq` — what the legacy wire reports.
    pub legacy_seq: i64,
    /// `replica_feed.sequence` — what `replica-v1` reports.
    pub sequence: i64,
}

/// Journal an upsert or restore of a live memory.
///
/// `at` is the mutation's provenance time, the same value the legacy
/// `sync_log` row carries, so the two feeds describe one event.
///
/// `operation_id` is `Some` when the caller already minted one — a local
/// write records it in its [`crate::store::memory_intent`] row before the
/// markdown moves, so the write that finishes (now, or at the next
/// reconciliation) journals under exactly the id the intent named. Every
/// other caller passes `None` and one is minted here.
///
/// # Errors
/// Propagates payload serialization and SQLite failures.
pub(crate) fn record_write(
    tx: &Connection,
    op: ReplicaOp,
    fm: &Frontmatter,
    body: &str,
    at: &str,
    origin: ReplicaOrigin,
    operation_id: Option<&str>,
) -> Result<Positions> {
    let payload = MemoryPayloadV1::new(fm, body)?;
    let (bytes, digest) = payload.canonical()?;
    let legacy_seq = sync_log::append(tx, op, &fm.id, &fm.content_hash, at, origin)?;
    journal(
        legacy_seq,
        tx,
        &NewOperation {
            operation_id: operation_id
                .map_or_else(|| mint_operation_id(&fm.id, op), str::to_string)
                .as_str(),
            entity_kind: MEMORY_ENTITY_KIND,
            entity_key: &fm.id,
            op,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: &bytes,
            }),
            schema_version: MEMORY_PAYLOAD_VERSION,
            repository: repository_of(fm),
            origin,
            at,
        },
    )
}

/// Journal a tombstone. A tombstone carries no payload — the bytes it would
/// name are exactly what the delete removed.
///
/// # Errors
/// Propagates SQLite failures.
pub(crate) fn record_tombstone(
    tx: &Connection,
    memory_id: &str,
    content_hash: &str,
    repository: Option<&str>,
    at: &str,
    origin: ReplicaOrigin,
    operation_id: Option<&str>,
) -> Result<Positions> {
    let legacy_seq = sync_log::append(
        tx,
        ReplicaOp::Tombstone,
        memory_id,
        content_hash,
        at,
        origin,
    )?;
    journal(
        legacy_seq,
        tx,
        &NewOperation {
            operation_id: operation_id
                .map_or_else(
                    || mint_operation_id(memory_id, ReplicaOp::Tombstone),
                    str::to_string,
                )
                .as_str(),
            entity_kind: MEMORY_ENTITY_KIND,
            entity_key: memory_id,
            op: ReplicaOp::Tombstone,
            payload: None,
            schema_version: MEMORY_PAYLOAD_VERSION,
            repository,
            origin,
            at,
        },
    )
}

/// Append the feed row and, for a local mutation, enqueue the upload it owes.
///
/// An imported mutation is journalled but never enqueued: pushing it back is
/// how a replication loop starts.
fn journal(legacy_seq: i64, tx: &Connection, new: &NewOperation<'_>) -> Result<Positions> {
    let epoch = stream_epoch(tx)?;
    let sequence = replica_journal::append(tx, &epoch, new)?;
    if new.origin == ReplicaOrigin::Local {
        // `observed_sequence` stays NULL here: the position this mutation was
        // made against lives in the upstream's sequence space, which the push
        // fills in when it knows the workspace and its cursor.
        replica_outbox::enqueue(tx, new, None)?;
    }
    Ok(Positions {
        legacy_seq,
        sequence,
    })
}

/// A repository label is only replicated when the memory has one.
fn repository_of(fm: &Frontmatter) -> Option<&str> {
    (!fm.repo.is_empty()).then_some(fm.repo.as_str())
}

/// Mint `op-<yyyymmdd>-<8hex>` for one mutation.
///
/// The seed carries the entity and the operation so two different mutations
/// cannot collide on a slow clock, and `dated_id` mixes in nanoseconds so the
/// same entity mutated twice yields two ids.
pub(crate) fn mint_operation_id(entity_key: &str, op: ReplicaOp) -> String {
    dated_id(
        "op",
        &format!("{MEMORY_ENTITY_KIND}:{entity_key}:{}", op.as_str()),
        OffsetDateTime::now_utc(),
    )
}

#[cfg(test)]
#[path = "tests/journal.rs"]
mod tests;
