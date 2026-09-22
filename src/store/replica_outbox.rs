//! `replica_operation` row CRUD — the durable outbox of mutations made here
//! that a peer has not yet accepted.
//!
//! Enqueued in the same transaction as the mutation's mirror write: a save
//! that committed can never forget that it owes an upload.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::replica_journal::{NewOperation, ReplicaOp};
use super::schema_replica::{ReplicaOperation, replica_operation as col};
use crate::prelude::*;

/// One mutation still owed to the upstream workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOperation {
    /// Client-unique operation id.
    pub operation_id: String,
    /// Entity kind.
    pub entity_kind: String,
    /// Entity key within its kind.
    pub entity_key: String,
    /// What the mutation does.
    pub op: ReplicaOp,
    /// Payload digest; `None` for a tombstone.
    pub payload_digest: Option<String>,
    /// Payload schema version.
    pub schema_version: i64,
    /// Canonical repository, when the entity has one.
    pub repository: Option<String>,
    /// Upstream sequence the mutation was made against.
    pub observed_sequence: Option<i64>,
    /// Upload attempts so far.
    pub attempts: i64,
}

/// Enqueue a mutation. The caller owns the transaction, which must be the one
/// that wrote the mirror row.
///
/// # Errors
/// Propagates SQLite failures, including a reused `operation_id`.
pub fn enqueue(
    tx: &Connection,
    new: &NewOperation<'_>,
    observed_sequence: Option<i64>,
) -> Result<()> {
    orm::execute(
        tx,
        ReplicaOperation::insert()
            .set(&col::operation_id, new.operation_id)
            .set(&col::entity_kind, new.entity_kind)
            .set(&col::entity_key, new.entity_key)
            .set(&col::op, new.op.as_str())
            .set(&col::payload_digest, new.payload.map(|p| p.digest))
            .set(&col::schema_version, new.schema_version)
            .set(&col::repository, new.repository)
            .set(&col::observed_sequence, observed_sequence)
            .set(&col::state, "pending")
            .set(&col::created_at, new.at)
            .set(&col::updated_at, new.at)
            .to_sql(),
    )?;
    Ok(())
}

/// The oldest `limit` operations still pending, oldest first.
///
/// # Errors
/// Propagates SQLite failures and an `op` literal the schema's `CHECK` should
/// have refused.
pub fn pending(conn: &Connection, limit: usize) -> Result<Vec<PendingOperation>> {
    let limit = i64::try_from(limit)
        .map_err(|_| Error::Other(format!("outbox limit not representable: {limit}")))?;
    let rows = orm::query_all(
        conn,
        ReplicaOperation::select()
            .columns_typed(&[
                &col::operation_id,
                &col::entity_kind,
                &col::entity_key,
                &col::op,
                &col::payload_digest,
                &col::schema_version,
                &col::repository,
                &col::observed_sequence,
                &col::attempts,
            ])
            .filter(col::state.eq("pending"))
            .order_by(col::created_at.asc())
            .limit(limit)
            .to_sql(),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, i64>(8)?,
            ))
        },
    )?;
    rows.into_iter()
        .map(|row| {
            Ok(PendingOperation {
                operation_id: row.0,
                entity_kind: row.1,
                entity_key: row.2,
                op: ReplicaOp::parse(&row.3)?,
                payload_digest: row.4,
                schema_version: row.5,
                repository: row.6,
                observed_sequence: row.7,
                attempts: row.8,
            })
        })
        .collect()
}

/// Whether this machine still owes an upload for one entity.
///
/// The import path asks before overwriting local state: a memory edited here
/// and not yet pushed must not be silently replaced by a peer's older view of
/// it, because the payload the outbox holds is the only record of that edit.
///
/// # Errors
/// Propagates SQLite failures.
pub fn has_pending_for(conn: &Connection, entity_kind: &str, entity_key: &str) -> Result<bool> {
    let count: i64 = orm::query_one(
        conn,
        ReplicaOperation::select()
            .filter(col::state.eq("pending"))
            .filter(col::entity_kind.eq(entity_kind))
            .filter(col::entity_key.eq(entity_key))
            .to_count_sql(),
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

/// How many operations are still pending — what a push still owes.
///
/// # Errors
/// Propagates SQLite failures.
pub fn pending_count(conn: &Connection) -> Result<i64> {
    orm::query_one(
        conn,
        ReplicaOperation::select()
            .filter(col::state.eq("pending"))
            .to_count_sql(),
        |r| r.get(0),
    )
}

/// What happened to an operation on its way upstream.
#[derive(Debug, Clone, Copy)]
pub enum Outcome<'a> {
    /// The upstream accepted it at `sequence`.
    Accepted {
        /// Position the upstream assigned.
        sequence: Option<i64>,
        /// Disposition it answered with.
        disposition: &'a str,
    },
    /// The upstream refused it. The row stays as evidence.
    Rejected {
        /// Disposition it answered with.
        disposition: &'a str,
    },
    /// The upload itself failed; the operation is still owed.
    Failed {
        /// Transport detail, for operator diagnosis.
        error: &'a str,
    },
}

/// Record what happened to one operation.
///
/// One writer for all three outcomes: an accepted, a refused and a failed
/// upload differ only in which columns they set, and splitting them into
/// separate functions made three copies of the same update.
///
/// # Errors
/// Propagates SQLite failures.
pub fn record(
    conn: &Connection,
    operation_id: &str,
    outcome: Outcome<'_>,
    at: &str,
) -> Result<usize> {
    let update = ReplicaOperation::update().set(&col::updated_at, at);
    let update = match outcome {
        Outcome::Accepted {
            sequence,
            disposition,
        } => update
            .set(&col::state, "accepted")
            .set(&col::upstream_sequence, sequence)
            .set(&col::disposition, disposition),
        Outcome::Rejected { disposition } => update
            .set(&col::state, "rejected")
            .set(&col::disposition, disposition),
        Outcome::Failed { error } => update
            .set_expr(&col::attempts, "attempts + 1")
            .set(&col::last_error, error),
    };
    orm::execute(
        conn,
        update.filter(col::operation_id.eq(operation_id)).to_sql(),
    )
}

#[cfg(test)]
#[path = "tests/replica_outbox.rs"]
mod tests;
