//! `POST /sync/replica/import` — decide each operation, then materialize,
//! journal and receipt it in one transaction.
//!
//! The decision order is the contract: a replay is answered from its receipt
//! before anything else happens, an erased payload is refused before it can be
//! rewritten, and tombstone ordering is checked before any state moves.

use crate::domains::sync::replica::contract::{
    Disposition, ImportRequest, ImportResponse, MAX_OPERATIONS, Operation, OperationResult,
    PROTOCOL,
};
use crate::domains::sync::replica::{materialize, validate};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_journal::{ReplicaOp, stream_epoch};
use crate::store::replica_receipt::{self, Receipt};
use crate::store::{memory_row, replica_read};
use crate::utilities::canonical_json;
use crate::utilities::context::Ctx;

/// Apply an envelope, one transaction per operation.
///
/// # Errors
/// Returns [`Error::BadRequest`] for an envelope this engine will not read at
/// all (wrong protocol, over the cap, or claiming a workspace), and propagates
/// SQLite failures. A single operation's refusal is a disposition, not an
/// error: the rest of the envelope still applies.
pub fn run(ctx: &mut Ctx<'_>, request: ImportRequest) -> Result<ImportResponse> {
    check_envelope(&request)?;
    let epoch = {
        let conn = ctx.conn()?;
        stream_epoch(conn)?
    };
    validate::check_cursor(request.cursor.as_ref(), &epoch)?;

    let mut results = Vec::with_capacity(request.operations.len());
    for operation in &request.operations {
        results.push(apply_one(ctx, &epoch, operation)?);
    }
    let head_sequence = replica_read::head(ctx.conn()?)?;
    Ok(ImportResponse {
        protocol: PROTOCOL.to_string(),
        stream_epoch: epoch,
        head_sequence,
        results,
    })
}

/// Refuse an envelope this engine will not read at all.
fn check_envelope(request: &ImportRequest) -> Result<()> {
    if request.protocol != PROTOCOL {
        return Err(Error::BadRequest(format!(
            "unsupported protocol {}, expected {PROTOCOL}",
            request.protocol
        )));
    }
    if request.workspace_id.is_some() {
        return Err(Error::BadRequest(
            "workspace comes from the authenticated credential; remove workspace_id".to_string(),
        ));
    }
    if request.operations.len() > MAX_OPERATIONS {
        return Err(Error::BadRequest(format!(
            "envelope accepts at most {MAX_OPERATIONS} operations, got {}",
            request.operations.len()
        )));
    }
    Ok(())
}

/// Decide and apply one operation.
pub(crate) fn apply_one(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
) -> Result<OperationResult> {
    if let Some(answered) = replayed(ctx.conn()?, operation)? {
        return Ok(answered);
    }
    let decision = validate::decide(ctx, operation)?;
    if decision != Disposition::Accepted {
        // An event this engine already holds is answered with the position
        // it was first accepted at, as a replay under its own id would be.
        let sequence = if decision == Disposition::Duplicate {
            replica_read::revision(ctx.conn()?, &operation.entity_kind, &operation.entity_key)?
                .map(|revision| revision.sequence)
        } else {
            None
        };
        return record_refusal(
            ctx,
            epoch,
            operation,
            decision,
            validate::reason(decision),
            sequence,
        );
    }
    materialize::apply(ctx, epoch, operation)
}

/// A replay is answered from its receipt; the same id with different bytes is
/// a conflict, not a second decision.
///
/// Identity is the digest of the bytes that actually ARRIVED, never the digest
/// the operation claims. Two consequences, both deliberate: a client that
/// resends altered bytes under the original digest is not answered "already
/// applied", and a replay of a refused operation reads back the SAME refusal
/// rather than turning into a conflict because the refusal was about the claim
/// disagreeing with the bytes in the first place.
fn replayed(conn: &Connection, operation: &Operation) -> Result<Option<OperationResult>> {
    let Some(receipt) = replica_receipt::lookup(conn, &operation.operation_id)? else {
        return Ok(None);
    };
    if receipt.payload_digest != arrived_digest(operation) {
        return Ok(Some(OperationResult {
            operation_id: operation.operation_id.clone(),
            disposition: Disposition::RejectedConflict,
            sequence: None,
            payload_digest: operation.payload_digest.clone(),
            reason: Some("operation id already answered for different payload bytes".to_string()),
        }));
    }
    let disposition = if receipt.disposition == Disposition::Accepted.as_str() {
        Disposition::Duplicate
    } else {
        // A refusal replays as the same refusal: the peer must see the
        // original answer, not a fresh evaluation against moved state.
        validate::parse_disposition(&receipt.disposition)
    };
    Ok(Some(OperationResult {
        operation_id: operation.operation_id.clone(),
        disposition,
        sequence: receipt.sequence,
        payload_digest: receipt.payload_digest,
        reason: receipt.reason,
    }))
}

/// Persist a refusal so a replay reads the same answer, and return it.
///
/// `sequence` is `Some` only for a `duplicate` event: the position the event
/// already holds, which the receipt keeps so a later replay reads it too.
fn record_refusal(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
    disposition: Disposition,
    reason: Option<String>,
    sequence: Option<i64>,
) -> Result<OperationResult> {
    let at = memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    let receipt = Receipt {
        operation_id: operation.operation_id.clone(),
        epoch: epoch.to_string(),
        sequence,
        // The digest of what arrived, not of what was claimed: this is the
        // identity a replay is matched against, and storing the claim would
        // make the retry of a refused operation look like new bytes.
        payload_digest: arrived_digest(operation),
        disposition: disposition.as_str().to_string(),
        reason: reason.clone(),
    };
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    replica_receipt::record(&tx, &receipt, &at)?;
    tx.commit()?;
    Ok(OperationResult {
        operation_id: operation.operation_id.clone(),
        disposition,
        sequence,
        payload_digest: operation.payload_digest.clone(),
        reason,
    })
}

/// The digest of the payload bytes that actually arrived, when the operation
/// carries a payload. `None` for a tombstone, and for a payload that cannot be
/// canonicalized at all — which never matches a stored receipt, so it falls
/// through to the conflict branch rather than being mistaken for a replay.
fn arrived_digest(operation: &Operation) -> Option<String> {
    let payload = operation.payload.as_ref()?;
    canonical_json::bytes_and_digest(payload)
        .ok()
        .map(|(_, digest)| digest)
}

/// Whether an operation carries a payload it must have.
pub(crate) fn needs_payload(op: ReplicaOp) -> bool {
    matches!(op, ReplicaOp::Upsert | ReplicaOp::Restore)
}

#[cfg(test)]
#[path = "tests/accept.rs"]
mod tests;
