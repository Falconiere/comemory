//! The acceptance decision — everything checked before any state moves.
//!
//! Kept apart from the write path so the rules can be read (and tested) as
//! rules: what the engine understands, what the digest must cover, what an
//! erased payload means, and how a tombstone orders against an edit.

use crate::domains::code::replica_payload::{
    CODE_ENTITY_KIND, CODE_PAYLOAD_VERSION, CodeGenerationV1,
};
use crate::domains::documents::replica_payload::{
    DOCUMENT_ENTITY_KIND, DOCUMENT_PAYLOAD_VERSION, DocumentRevisionV1,
};
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
    if let Some(problem) = kind_and_shape(operation) {
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

/// Refuse an entity kind or schema version this build cannot read, then
/// validate what the operation carries.
///
/// The replicated kinds share every ordering rule below — `order`,
/// `check_cursor` and `check_position` never look at the kind — so only the
/// payload's own shape and identity differ.
fn kind_and_shape(operation: &Operation) -> Option<Disposition> {
    match operation.entity_kind.as_str() {
        MEMORY_ENTITY_KIND if operation.schema_version == MEMORY_PAYLOAD_VERSION => {
            shape(operation, identity::<MemoryPayloadV1>)
        }
        CODE_ENTITY_KIND if operation.schema_version == CODE_PAYLOAD_VERSION => {
            shape(operation, identity::<CodeGenerationV1>)
        }
        DOCUMENT_ENTITY_KIND if operation.schema_version == DOCUMENT_PAYLOAD_VERSION => {
            shape(operation, identity::<DocumentRevisionV1>)
        }
        _ => Some(Disposition::RejectedUnsupported),
    }
}

/// What every kind's payload must satisfy before its own identity rules run:
/// a tombstone carries nothing, an upsert carries bytes, and the declared
/// digest covers the bytes that arrived. [`identity`] then applies the rule
/// only that kind can state.
fn shape(
    operation: &Operation,
    identity: impl FnOnce(&Operation, &str) -> Option<Disposition>,
) -> Option<Disposition> {
    if !needs_payload(operation.op) {
        return (operation.payload.is_some()).then_some(Disposition::RejectedInvalid);
    }
    let text = match canonical_text(operation) {
        Ok(text) => text,
        Err(problem) => return Some(problem),
    };
    identity(operation, &text)
}

/// What a replicated payload must satisfy before its kind's writer sees it.
///
/// One rule per kind, stated by the kind itself: the invariant is a property of
/// the payload rather than of the validator that happens to ask, so a third
/// kind adds a rule and not a third copy of the decode-and-refuse skeleton.
trait Identity: serde::de::DeserializeOwned {
    /// Whether the payload holds together under `entity_key`.
    fn holds_for(&self, entity_key: &str) -> bool;
}

impl Identity for MemoryPayloadV1 {
    /// The content-derived identity rules a memory keeps on every surface.
    fn holds_for(&self, entity_key: &str) -> bool {
        entity_key == self.id
            && is_valid_memory_id(&self.id)
            && self.id == memory_id(&self.body)
            && self.content_hash == sha256_hex(self.body.trim_end().as_bytes())
            && (1..=5).contains(&self.quality)
            && self.created_at().is_ok()
    }
}

impl Identity for CodeGenerationV1 {
    /// A code generation's identity is its manifest: the id must be the one its
    /// contents earn, exactly as a memory's id must hash its body.
    ///
    /// The repo is NOT checked against the payload, because the payload has no
    /// repo in it — the entity IS the repo, so `entity_key` names it and is the
    /// only place it appears. Folding a repo into the payload would fold it
    /// into the digest and therefore into the generation id, which would make
    /// the same tree indexed under two labels two different generations. So the
    /// only thing to require of the key is that there IS one.
    fn holds_for(&self, entity_key: &str) -> bool {
        self.owns_its_id().unwrap_or(false) && !entity_key.is_empty()
    }
}

impl Identity for DocumentRevisionV1 {
    /// A document revision's identity is its NAME: the id must be the one its
    /// repo and path earn and the key must equal it, so text filed under
    /// another document's name is refused before a reader can be shown it. The
    /// passages must also run contiguously from zero and every link must name a
    /// passage the revision has — a revision assembled from parts that did not
    /// all arrive fails both.
    fn holds_for(&self, entity_key: &str) -> bool {
        entity_key == self.shared_id
            && self.owns_its_id()
            && self.chunks_are_contiguous()
            && self.links_resolve()
    }
}

/// Decode and judge one kind's payload.
///
/// The two refusals are different answers, which is the whole reason this is
/// shared: a payload this build cannot READ is unsupported and the sender
/// should stop offering it, while one that reads but does not hold together is
/// invalid and the sender has a bug.
fn identity<T: Identity>(operation: &Operation, text: &str) -> Option<Disposition> {
    // Deserialized here rather than through each type's own `decode`: those
    // differ only in the error message they build, and the message is thrown
    // away — all this needs to know is whether the bytes read at all.
    let Ok(payload) = serde_json::from_str::<T>(text) else {
        return Some(Disposition::RejectedUnsupported);
    };
    (!payload.holds_for(&operation.entity_key)).then_some(Disposition::RejectedInvalid)
}

/// The operation's payload as canonical text, with the declared digest
/// verified against the bytes that actually arrived.
fn canonical_text(operation: &Operation) -> std::result::Result<String, Disposition> {
    let (Some(payload), Some(digest)) = (
        operation.payload.as_ref(),
        operation.payload_digest.as_deref(),
    ) else {
        return Err(Disposition::RejectedInvalid);
    };
    let Ok((bytes, computed)) = canonical_json::bytes_and_digest(payload) else {
        return Err(Disposition::RejectedInvalid);
    };
    if computed != digest {
        return Err(Disposition::RejectedInvalid);
    }
    String::from_utf8(bytes).map_err(|_| Disposition::RejectedInvalid)
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
