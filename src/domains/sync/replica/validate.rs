//! The acceptance decision — everything checked before any state moves.
//!
//! Kept apart from the write path so the rules can be read (and tested) as
//! rules: what the engine understands, what the digest must cover, what an
//! erased payload means, and how a tombstone orders against an edit.

use crate::domains::memories::id::{is_valid_memory_id, memory_id};
use crate::domains::memories::replica_payload::{
    MEMORY_ENTITY_KIND, MEMORY_PAYLOAD_VERSION, MemoryPayloadV1,
};
use crate::domains::sync::replica::accept::needs_payload;
use crate::domains::sync::replica::contract::{CursorRef, Disposition, Operation};
use crate::prelude::*;
use crate::store::replica_journal::ReplicaOp;
use crate::store::{replica_outbox, replica_read};
use crate::utilities::canonical_json;
use crate::utilities::context::Ctx;
use crate::utilities::digest::sha256_hex;

/// Refuse a cursor taken under a different stream.
///
/// # Errors
/// Returns [`Error::EpochMismatch`] when the epochs disagree — a replaced or
/// restored stream must be visible, never answered with an empty page the
/// peer would read as agreement.
pub fn check_cursor(cursor: Option<&CursorRef>, epoch: &str) -> Result<()> {
    match cursor {
        Some(cursor) if cursor.stream_epoch != epoch => Err(Error::EpochMismatch(format!(
            "cursor is for stream {}, this stream is {epoch}",
            cursor.stream_epoch
        ))),
        _ => Ok(()),
    }
}

/// Refuse a continuation that points past the head this stream has issued.
///
/// A cursor at the head is caught up and reads as an empty page; a cursor
/// *above* it names a position this stream never assigned, which means the
/// peer is holding progress from somewhere else. Answering that with an empty
/// page would let it keep believing it was up to date.
///
/// # Errors
/// Returns [`Error::Conflict`] (`cursor_ahead`) when `since` exceeds `head`.
pub fn check_position(since: i64, head: i64) -> Result<()> {
    if since > head {
        return Err(Error::Conflict(format!(
            "cursor_ahead: cursor is at {since}, this stream's head is {head}"
        )));
    }
    Ok(())
}

/// Decide one operation against current state.
///
/// # Errors
/// Propagates SQLite failures. Every refusal is a [`Disposition`], not an
/// error: one bad operation must not discard the rest of the envelope.
pub fn decide(ctx: &mut Ctx<'_>, operation: &Operation) -> Result<Disposition> {
    if operation.entity_kind != MEMORY_ENTITY_KIND {
        return Ok(Disposition::RejectedUnsupported);
    }
    if operation.schema_version != MEMORY_PAYLOAD_VERSION {
        return Ok(Disposition::RejectedUnsupported);
    }
    if let Some(problem) = payload_shape(operation) {
        return Ok(problem);
    }
    let conn = ctx.conn()?;
    if let Some(digest) = operation.payload_digest.as_deref()
        && replica_read::is_erased(conn, digest)?
    {
        return Ok(Disposition::PayloadErased);
    }
    // A local mutation this machine has not yet pushed is the only record of
    // that edit: the outbox holds its payload, and materializing the peer's
    // version would overwrite the markdown the pending operation describes.
    // Refusing keeps the local edit intact and makes the peer's write wait
    // for the push that will order the two properly.
    if replica_outbox::has_pending_for(conn, &operation.entity_kind, &operation.entity_key)? {
        return Ok(Disposition::RejectedStale);
    }
    let revision = replica_read::revision(conn, &operation.entity_kind, &operation.entity_key)?;
    Ok(order(operation, revision.as_ref()))
}

/// Validate what the operation carries, independent of stored state.
fn payload_shape(operation: &Operation) -> Option<Disposition> {
    if !needs_payload(operation.op) {
        return (operation.payload.is_some()).then_some(Disposition::RejectedInvalid);
    }
    let (Some(payload), Some(digest)) = (
        operation.payload.as_ref(),
        operation.payload_digest.as_deref(),
    ) else {
        return Some(Disposition::RejectedInvalid);
    };
    let Ok((bytes, computed)) = canonical_json::bytes_and_digest(payload) else {
        return Some(Disposition::RejectedInvalid);
    };
    if computed != digest {
        return Some(Disposition::RejectedInvalid);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Some(Disposition::RejectedInvalid);
    };
    let Ok(decoded) = MemoryPayloadV1::decode(&text) else {
        return Some(Disposition::RejectedUnsupported);
    };
    memory_identity(operation, &decoded)
}

/// The content-derived identity rules a memory keeps on every surface.
fn memory_identity(operation: &Operation, payload: &MemoryPayloadV1) -> Option<Disposition> {
    let body_hash = sha256_hex(payload.body.trim_end().as_bytes());
    let mismatch = operation.entity_key != payload.id
        || !is_valid_memory_id(&payload.id)
        || payload.id != memory_id(&payload.body)
        || payload.content_hash != body_hash
        || !(1..=5).contains(&payload.quality)
        || payload.created_at().is_err();
    mismatch.then_some(Disposition::RejectedInvalid)
}

/// Order the operation against the entity's current revision.
///
/// Server acceptance orders edits, so "stale" is not about clocks: an upsert
/// that never saw the deletion cannot revive it, and a restore must name the
/// deletion it observed.
fn order(operation: &Operation, revision: Option<&replica_read::RevisionRow>) -> Disposition {
    let Some(revision) = revision else {
        return match operation.op {
            // A tombstone for an entity this engine has never seen is
            // recorded as a convergence fact, not refused: the peer deleted
            // something this side never received.
            ReplicaOp::Restore => Disposition::RejectedStale,
            ReplicaOp::Upsert | ReplicaOp::Tombstone => Disposition::Accepted,
        };
    };
    match operation.op {
        ReplicaOp::Upsert if revision.deleted => Disposition::RejectedStale,
        ReplicaOp::Restore => {
            if revision.deleted && operation.observed_sequence == revision.deleted_sequence {
                Disposition::Accepted
            } else {
                Disposition::RejectedStale
            }
        }
        ReplicaOp::Upsert | ReplicaOp::Tombstone => Disposition::Accepted,
    }
}

/// The detail a refusal carries back, so an operator can see why without
/// reading the feed.
#[must_use]
pub fn reason(disposition: Disposition) -> Option<String> {
    let text = match disposition {
        Disposition::RejectedStale => {
            "the observed revision is behind this entity's deletion, or this \
             machine still owes an unpushed change to it"
                .to_string()
        }
        Disposition::RejectedUnsupported => {
            "unknown entity kind or payload schema version".to_string()
        }
        Disposition::RejectedInvalid => "payload failed validation".to_string(),
        Disposition::PayloadErased => {
            "payload bytes were permanently erased; the barrier stands".to_string()
        }
        Disposition::PayloadExpired => "payload bytes are past retention".to_string(),
        Disposition::RejectedNotAllowed => "policy does not allow this entity".to_string(),
        Disposition::RejectedConflict => {
            "operation id already answered for different payload bytes".to_string()
        }
        Disposition::Accepted | Disposition::Duplicate => return None,
    };
    Some(text)
}

/// Read a stored disposition literal back.
///
/// An unrecognized literal reads as `rejected_invalid` rather than panicking:
/// a receipt written by a newer engine must still replay as a refusal.
#[must_use]
pub fn parse_disposition(raw: &str) -> Disposition {
    match raw {
        "accepted" => Disposition::Accepted,
        "duplicate" => Disposition::Duplicate,
        "rejected_stale" => Disposition::RejectedStale,
        "rejected_conflict" => Disposition::RejectedConflict,
        "rejected_unsupported" => Disposition::RejectedUnsupported,
        "rejected_not_allowed" => Disposition::RejectedNotAllowed,
        "payload_expired" => Disposition::PayloadExpired,
        "payload_erased" => Disposition::PayloadErased,
        _ => Disposition::RejectedInvalid,
    }
}

#[cfg(test)]
#[path = "tests/validate.rs"]
mod tests;
