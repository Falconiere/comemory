//! `replica_receipt` row CRUD — what this engine answered for an operation,
//! written in the accept transaction and read back on a replay.
//!
//! A receipt is the reason a lost acknowledgement costs a round trip instead
//! of a second effect: the replay reads the original decision, including the
//! position it was given, rather than being accepted again at a newer one.

use rusqlite::Connection;
use toolu_orm::core::query_column::CommonOps;

use super::orm;
use super::schema_replica::{ReplicaReceipt, replica_receipt as col};
use crate::prelude::*;

/// The raw column tuple [`lookup`] decodes: epoch, sequence, disposition,
/// payload digest, reason.
type StoredReceipt = (String, Option<i64>, String, Option<String>, Option<String>);

/// A stored decision.
#[derive(Debug, Clone)]
pub struct Receipt {
    /// The operation this receipt answers.
    pub operation_id: String,
    /// Stream epoch the decision was made under.
    pub epoch: String,
    /// Assigned position; `None` when nothing was written.
    pub sequence: Option<i64>,
    /// Wire disposition literal.
    pub disposition: String,
    /// Payload digest the decision was made against.
    pub payload_digest: Option<String>,
    /// Detail for a refusal.
    pub reason: Option<String>,
}

/// Write a decision. The caller owns the transaction, which must be the one
/// that wrote the materialized state and the feed row.
///
/// # Errors
/// Propagates SQLite failures, including a duplicate `operation_id` — a
/// second decision for one operation is a contradiction, not an update.
pub fn record(tx: &Connection, receipt: &Receipt, at: &str) -> Result<()> {
    orm::execute(
        tx,
        ReplicaReceipt::insert()
            .set(&col::operation_id, receipt.operation_id.as_str())
            .set(&col::epoch, receipt.epoch.as_str())
            .set(&col::sequence, receipt.sequence)
            .set(&col::disposition, receipt.disposition.as_str())
            .set(&col::payload_digest, receipt.payload_digest.as_deref())
            .set(&col::reason, receipt.reason.as_deref())
            .set(&col::accepted_at, at)
            .to_sql(),
    )?;
    Ok(())
}

/// The stored decision for `operation_id`, if this engine already answered it.
///
/// # Errors
/// Propagates SQLite failures.
pub fn lookup(conn: &Connection, operation_id: &str) -> Result<Option<Receipt>> {
    let row: Option<StoredReceipt> = orm::query_optional(
        conn,
        ReplicaReceipt::select()
            .columns_typed(&[
                &col::epoch,
                &col::sequence,
                &col::disposition,
                &col::payload_digest,
                &col::reason,
            ])
            .filter(col::operation_id.eq(operation_id))
            .to_sql(),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    Ok(row.map(
        |(epoch, sequence, disposition, payload_digest, reason)| Receipt {
            operation_id: operation_id.to_string(),
            epoch,
            sequence,
            disposition,
            payload_digest,
            reason,
        },
    ))
}

#[cfg(test)]
#[path = "tests/replica_receipt.rs"]
mod tests;
