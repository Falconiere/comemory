//! `POST /sync/replica/stage` and `/activate` — revisions too large for one
//! envelope.
//!
//! Parts land in a table nothing else reads, so an upload that never
//! activates publishes nothing. Activation assembles the parts, checks the
//! assembled bytes against the digest the operation declared, and then goes
//! through the ordinary acceptance path — staging changes how the payload
//! arrives, never what acceptance means.

use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::contract::{
    MAX_ENVELOPE_BYTES, Operation, OperationResult, PROTOCOL,
};
use crate::domains::sync::replica::contract_views::{ActivateRequest, StageRequest, StageResponse};
use crate::domains::sync::replica::validate;
use crate::prelude::*;
use crate::store::memory_row;
use crate::store::replica_journal::stream_epoch;
use crate::store::replica_staging;
use crate::utilities::canonical_json;
use crate::utilities::context::Ctx;

/// Store one part of a staged revision.
///
/// # Errors
/// Returns [`Error::BadRequest`] for a wrong protocol, an out-of-range part
/// index, or a part above the envelope cap; propagates SQLite failures.
pub fn stage(ctx: &mut Ctx<'_>, request: StageRequest) -> Result<StageResponse> {
    if request.protocol != PROTOCOL {
        return Err(Error::BadRequest(format!(
            "unsupported protocol {}, expected {PROTOCOL}",
            request.protocol
        )));
    }
    if request.part_count < 1 || request.part_index < 0 || request.part_index >= request.part_count
    {
        return Err(Error::BadRequest(format!(
            "part {} is outside a {}-part upload",
            request.part_index, request.part_count
        )));
    }
    if request.bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(Error::BadRequest(format!(
            "part is {} bytes, over the {MAX_ENVELOPE_BYTES} cap",
            request.bytes.len()
        )));
    }
    let at = memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    let conn = ctx.conn()?;
    let state = replica_staging::put_part(
        conn,
        &request.staging_id,
        request.part_index,
        request.part_count,
        &request.bytes,
        &at,
    )?;
    Ok(StageResponse {
        protocol: PROTOCOL.to_string(),
        staging_id: request.staging_id,
        received: state.received,
        declared: state.declared,
        complete: state.complete(),
    })
}

/// Assemble a staged upload and accept it as one operation.
///
/// # Errors
/// Returns [`Error::BadRequest`] for a wrong protocol or an operation that
/// already carries a payload, [`Error::EpochMismatch`] for a foreign cursor,
/// and [`Error::Conflict`] (`staging_incomplete`) while a declared part is
/// still missing — a truncated revision is never published. An incomplete
/// upload keeps its parts for the retry; one that acceptance has answered,
/// accepted or refused alike, is discarded.
pub fn activate(ctx: &mut Ctx<'_>, request: ActivateRequest) -> Result<OperationResult> {
    if request.protocol != PROTOCOL {
        return Err(Error::BadRequest(format!(
            "unsupported protocol {}, expected {PROTOCOL}",
            request.protocol
        )));
    }
    if request.operation.payload.is_some() {
        return Err(Error::BadRequest(
            "an activated operation carries its payload in the staged parts".to_string(),
        ));
    }
    let epoch = {
        let conn = ctx.conn()?;
        stream_epoch(conn)?
    };
    validate::check_cursor(request.cursor.as_ref(), &epoch)?;

    let assembled = {
        let conn = ctx.conn()?;
        replica_staging::assemble(conn, &request.staging_id)?
    };
    let Some(bytes) = assembled else {
        return Err(Error::Conflict(format!(
            "staging_incomplete: upload {} is missing a part",
            request.staging_id
        )));
    };
    let operation = with_payload(request.operation, &bytes)?;
    let result = accept::apply_one(ctx, &epoch, &operation)?;
    // Answered is finished, whatever the answer was. A refusal records a
    // receipt, and a receipt is keyed on the bytes that arrived, so the same
    // operation id can never accept afterwards: resending it with the part
    // corrected is a conflict, not a second chance. Keeping the parts would
    // leave rows nothing can ever reclaim — there is no sweep — and a code
    // generation is the first payload large enough to need staging routinely.
    let conn = ctx.conn()?;
    replica_staging::discard(conn, &request.staging_id)?;
    Ok(result)
}

/// Attach the assembled bytes to the operation, as the import path would
/// have received them.
///
/// The bytes are parsed, not trusted: a payload that is not JSON is refused
/// here rather than reaching acceptance as an opaque blob. The digest is taken
/// over the CANONICAL form of the parsed value, not over the bytes as they
/// arrived — acceptance recomputes it the same way, so a sender whose parts
/// were not already canonical would otherwise be refused as invalid.
fn with_payload(operation: Operation, bytes: &str) -> Result<Operation> {
    let value: serde_json::Value = serde_json::from_str(bytes)
        .map_err(|e| Error::BadRequest(format!("assembled payload is not json: {e}")))?;
    let (_, digest) = canonical_json::bytes_and_digest(&value)?;
    Ok(Operation {
        payload: Some(value),
        payload_digest: operation.payload_digest.or(Some(digest)),
        ..operation
    })
}

#[cfg(test)]
#[path = "tests/staging.rs"]
mod tests;
