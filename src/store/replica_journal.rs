//! `replica-v1` journal writes — the payload row, the feed append and the
//! revision update that one accepted mutation owes, in the caller's
//! transaction.
//!
//! Whether an operation may be accepted at all (stale, conflicting, erased) is
//! the replication contract's decision and stays in `domains::sync::replica`.
//! This module writes what that decision produced, as one unit, so a feed
//! position can never exist without the payload it names.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::schema_replica::{
    ReplicaFeed, ReplicaPayload, ReplicaRevision, replica_feed as feed_col,
    replica_payload as payload_col, replica_revision as revision_col,
};
use super::{orm, schema_replica};
use crate::prelude::*;

/// What one journalled mutation does to an entity, and where it came from.
///
/// The legacy `sync_log` vocabulary, reused rather than restated: both feeds
/// describe the same events, and a second copy of `upsert | tombstone |
/// restore` would be a place for them to drift.
pub use super::sync_log::{SyncOp as ReplicaOp, SyncOrigin as ReplicaOrigin};

/// The immutable bytes a mutation carries, already canonicalized and hashed by
/// the domain that owns the entity.
#[derive(Debug, Clone, Copy)]
pub struct PayloadRef<'a> {
    /// 64-hex SHA-256 of `bytes`.
    pub digest: &'a str,
    /// Canonical JSON bytes.
    pub bytes: &'a str,
}

/// One mutation to journal.
#[derive(Debug, Clone, Copy)]
pub struct NewOperation<'a> {
    /// Client-unique operation id.
    pub operation_id: &'a str,
    /// Entity kind (`memory` today).
    pub entity_kind: &'a str,
    /// Entity key within its kind.
    pub entity_key: &'a str,
    /// What the mutation does.
    pub op: ReplicaOp,
    /// Payload bytes; `None` for a tombstone.
    pub payload: Option<PayloadRef<'a>>,
    /// Payload schema version for this kind.
    pub schema_version: i64,
    /// Canonical repository, when the entity has one.
    pub repository: Option<&'a str>,
    /// Local or imported.
    pub origin: ReplicaOrigin,
    /// RFC3339 provenance time.
    pub at: &'a str,
}

/// Store the payload, append the feed row and update the entity's revision.
/// Returns the assigned feed sequence. The caller owns the transaction.
///
/// # Errors
/// Propagates SQLite failures. A duplicate `operation_id` violates
/// `uq_replica_feed_operation` and is returned as an error rather than
/// silently ignored: a replay must be answered from its receipt, before this
/// is called.
pub fn append(tx: &Connection, epoch: &str, new: &NewOperation<'_>) -> Result<i64> {
    if let Some(payload) = new.payload {
        store_payload(tx, new, payload)?;
    }
    let sequence = append_feed(tx, epoch, new)?;
    update_revision(tx, new, sequence)?;
    Ok(sequence)
}

/// Insert the payload row, keeping any existing row for the same digest.
///
/// Identical bytes are one payload: content addressing is what lets a replay
/// and a re-save share history instead of duplicating it. `OR IGNORE` also
/// protects a redacted row — retention blanked its bytes on purpose, and a
/// later write must not restore them.
fn store_payload(tx: &Connection, new: &NewOperation<'_>, payload: PayloadRef<'_>) -> Result<()> {
    let byte_len = i64::try_from(payload.bytes.len())
        .map_err(|_| Error::Other("payload larger than i64 bytes".to_string()))?;
    orm::execute(
        tx,
        ReplicaPayload::insert()
            .or_ignore()
            .set(&payload_col::digest, payload.digest)
            .set(&payload_col::entity_kind, new.entity_kind)
            .set(&payload_col::schema_version, new.schema_version)
            .set(&payload_col::bytes, payload.bytes)
            .set(&payload_col::byte_len, byte_len)
            .set(&payload_col::created_at, new.at)
            .to_sql(),
    )?;
    Ok(())
}

/// Append the acceptance row and return the sequence SQLite assigned.
fn append_feed(tx: &Connection, epoch: &str, new: &NewOperation<'_>) -> Result<i64> {
    let insert = ReplicaFeed::insert()
        .set(&feed_col::epoch, epoch)
        .set(&feed_col::entity_kind, new.entity_kind)
        .set(&feed_col::entity_key, new.entity_key)
        .set(&feed_col::op, new.op.as_str())
        .set(&feed_col::payload_digest, new.payload.map(|p| p.digest))
        .set(&feed_col::schema_version, new.schema_version)
        .set(&feed_col::operation_id, new.operation_id)
        .set(&feed_col::origin, new.origin.as_str())
        .set(&feed_col::repository, new.repository)
        .set(&feed_col::at, new.at);
    orm::execute(tx, insert.to_sql())?;
    Ok(tx.last_insert_rowid())
}

/// Point the entity's revision at this acceptance.
///
/// A tombstone records its own position so a restore must name the deletion it
/// observed; an upsert or restore clears the deletion state.
fn update_revision(tx: &Connection, new: &NewOperation<'_>, sequence: i64) -> Result<()> {
    let deleted = i64::from(new.op == ReplicaOp::Tombstone);
    let deleted_sequence = (new.op == ReplicaOp::Tombstone).then_some(sequence);
    let digest = new.payload.map(|p| p.digest);
    let updated = orm::execute(
        tx,
        ReplicaRevision::update()
            .set(&revision_col::sequence, sequence)
            .set(&revision_col::payload_digest, digest)
            .set(&revision_col::deleted, deleted)
            .set(&revision_col::deleted_sequence, deleted_sequence)
            .set(&revision_col::updated_at, new.at)
            .filter(revision_col::entity_kind.eq(new.entity_kind))
            .filter(revision_col::entity_key.eq(new.entity_key))
            .to_sql(),
    )?;
    if updated > 0 {
        return Ok(());
    }
    orm::execute(
        tx,
        ReplicaRevision::insert()
            .set(&revision_col::entity_kind, new.entity_kind)
            .set(&revision_col::entity_key, new.entity_key)
            .set(&revision_col::sequence, sequence)
            .set(&revision_col::payload_digest, digest)
            .set(&revision_col::deleted, deleted)
            .set(&revision_col::deleted_sequence, deleted_sequence)
            .set(&revision_col::updated_at, new.at)
            .to_sql(),
    )?;
    Ok(())
}

/// Blank a payload's bytes while keeping the row.
///
/// Permanent erasure must remove the text without removing the barrier: the
/// digest stays, so an old upsert carrying those bytes is refused instead of
/// resurrecting what was erased.
///
/// # Errors
/// Propagates SQLite failures.
pub fn redact_payload(conn: &Connection, digest: &str, at: &str) -> Result<usize> {
    orm::execute(
        conn,
        ReplicaPayload::update()
            .set(&payload_col::bytes, None::<&str>)
            .set(&payload_col::redacted_at, at)
            .filter(payload_col::digest.eq(digest))
            .filter(payload_col::redacted_at.is_null())
            .to_sql(),
    )
}

/// This database's stream epoch.
///
/// # Errors
/// Returns [`Error::Other`] when the row the v22 post-pass mints is missing —
/// a stream with no epoch cannot answer a cursor, and guessing one would make
/// every peer's cursor look valid.
pub fn stream_epoch(conn: &Connection) -> Result<String> {
    let epoch: Option<String> = orm::query_optional(
        conn,
        schema_replica::ReplicaStream::select()
            .columns_typed(&[&schema_replica::replica_stream::epoch])
            .to_sql(),
        |r| r.get(0),
    )?;
    epoch.ok_or_else(|| Error::Other("replica_stream has no epoch row".to_string()))
}

#[cfg(test)]
#[path = "tests/replica_journal.rs"]
mod tests;
