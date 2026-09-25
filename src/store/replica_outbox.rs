//! `replica_operation` row CRUD — the durable outbox of mutations made here
//! that a peer has not yet accepted.
//!
//! Enqueued in the same transaction as the mutation's mirror write: a save
//! that committed can never forget that it owes an upload.

use rusqlite::Connection;
use toolu_orm::core::expr::Scalar;
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
    /// Last transport or rejection detail.
    pub last_error: Option<String>,
    /// Why the row is not sent now; `None` when it is eligible.
    pub hold_reason: Option<String>,
    /// Detail for the hold.
    pub hold_detail: Option<String>,
    /// Platform API base the row is stamped with.
    pub api_url: Option<String>,
    /// Workspace the row is stamped with.
    pub workspace_id: Option<String>,
    /// Canonical repository resolved at first send.
    pub wire_repository: Option<String>,
    /// RFC3339 time it was enqueued.
    pub created_at: String,
    /// `pending`, `accepted` or `rejected`.
    pub state: String,
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

/// The oldest `limit` operations still pending, held or not, in the order
/// they were made.
///
/// # Errors
/// Propagates SQLite failures and an `op` literal the schema's `CHECK` should
/// have refused.
pub fn pending(conn: &Connection, limit: usize) -> Result<Vec<PendingOperation>> {
    read(conn, Scope::All, limit)
}

/// Which outbox rows a read covers.
#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    /// Every pending row, held or not.
    All,
    /// Pending rows with no hold — what the next push batch sends.
    Eligible,
    /// Pending rows for one `(kind, key)`.
    Entity(&'a str, &'a str),
    /// One operation by id, WHATEVER its state — how a client recognizes its
    /// own operation when the upstream feed hands it back.
    Operation(&'a str),
}

/// The outbox rows `scope` covers, in the order they were made (`rowid`
/// breaks a `created_at` tie), at most `limit`.
///
/// # Errors
/// Propagates SQLite failures and an `op` literal the schema's `CHECK` should
/// have refused.
pub fn read(conn: &Connection, scope: Scope<'_>, limit: usize) -> Result<Vec<PendingOperation>> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut query = ReplicaOperation::select().columns_typed(&[
        &col::operation_id,
        &col::entity_kind,
        &col::entity_key,
        &col::op,
        &col::payload_digest,
        &col::schema_version,
        &col::repository,
        &col::observed_sequence,
        &col::attempts,
        &col::last_error,
        &col::hold_reason,
        &col::hold_detail,
        &col::api_url,
        &col::workspace_id,
        &col::wire_repository,
        &col::created_at,
        &col::state,
    ]);
    query = match scope {
        Scope::All => query.filter(col::state.eq("pending")),
        Scope::Eligible => query
            .filter(col::state.eq("pending"))
            .filter(col::hold_reason.is_null()),
        Scope::Entity(kind, key) => query
            .filter(col::state.eq("pending"))
            .filter(col::entity_kind.eq(kind))
            .filter(col::entity_key.eq(key)),
        Scope::Operation(operation_id) => query.filter(col::operation_id.eq(operation_id)),
    };
    orm::query_all(
        conn,
        query
            .order_by(col::created_at.asc())
            .order_by(Scalar::raw("rowid", Vec::new()).asc())
            .limit(limit)
            .to_sql(),
        decode,
    )
}

/// One outbox row, in [`read`]'s column order.
fn decode(r: &rusqlite::Row<'_>) -> rusqlite::Result<PendingOperation> {
    let op: String = r.get(3)?;
    Ok(PendingOperation {
        operation_id: r.get(0)?,
        entity_kind: r.get(1)?,
        entity_key: r.get(2)?,
        op: ReplicaOp::parse(&op).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?,
        payload_digest: r.get(4)?,
        schema_version: r.get(5)?,
        repository: r.get(6)?,
        observed_sequence: r.get(7)?,
        attempts: r.get(8)?,
        last_error: r.get(9)?,
        hold_reason: r.get(10)?,
        hold_detail: r.get(11)?,
        api_url: r.get(12)?,
        workspace_id: r.get(13)?,
        wire_repository: r.get(14)?,
        created_at: r.get(15)?,
        state: r.get(16)?,
    })
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

/// How many operations are in `state` (`pending` is what a push still owes).
///
/// # Errors
/// Propagates SQLite failures.
pub fn count(conn: &Connection, state: &str) -> Result<i64> {
    orm::query_one(
        conn,
        ReplicaOperation::select()
            .filter(col::state.eq(state))
            .to_count_sql(),
        |r| r.get(0),
    )
}

/// Remove an operation from the outbox — for a journalled write that owes no
/// upload (journal seeding), in the transaction that enqueued it.
///
/// # Errors
/// Propagates SQLite failures.
pub fn discard(tx: &Connection, operation_id: &str) -> Result<usize> {
    orm::execute(
        tx,
        ReplicaOperation::delete()
            .filter(col::operation_id.eq(operation_id))
            .to_sql(),
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
        /// Upstream epoch `sequence` belongs to.
        epoch: Option<&'a str>,
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
            epoch,
        } => update
            .set(&col::state, "accepted")
            .set(&col::upstream_sequence, sequence)
            .set(&col::upstream_epoch, epoch)
            .set(&col::disposition, disposition)
            .set(&col::hold_reason, None::<&str>)
            .set(&col::hold_detail, None::<&str>),
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
