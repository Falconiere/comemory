//! The acceptance rules only the two event kinds have (#254): an event is
//! immutable, so it is only ever an upsert; an upsert that arrives without its
//! bytes is history its origin's retention already removed; and an event this
//! engine already holds is answered, never applied again — whatever operation
//! id carries it, which is what stops a reverse-sync echo counting twice.
//!
//! Split from [`super::validate`], which routes the two kinds here and keeps
//! every rule the kinds share.

use crate::domains::learning::replica_payload::{FeedbackEventV1, Target};
use crate::domains::memories::id::is_valid_memory_id;
use crate::domains::sync::replica::activity_payload::{ACTIVITY_ENTITY_KIND, ActivityEventV1};
use crate::domains::sync::replica::contract::{Disposition, Operation};
use crate::domains::sync::replica::validate::{Identity, canonical_text, identity};
use crate::prelude::*;
use crate::store::replica_journal::ReplicaOp;
use crate::store::{Connection, replica_read};
use crate::utilities::telemetry::entity::FEEDBACK_EVENT;

/// Whether `kind` is one of the two event kinds.
#[must_use]
pub(crate) fn is_event_kind(kind: &str) -> bool {
    kind == FEEDBACK_EVENT || kind == ACTIVITY_ENTITY_KIND
}

/// The shape an event operation must have before its payload's own rules run.
///
/// A digest with no payload is answered `payload_expired`: the only way an
/// event's bytes go missing upstream is retention, and a typed answer lets a
/// bootstrap walking expired history advance instead of stalling on
/// `rejected_invalid`.
pub(super) fn shape<T: Identity>(operation: &Operation) -> Option<Disposition> {
    if operation.op != ReplicaOp::Upsert {
        return Some(Disposition::RejectedInvalid);
    }
    if operation.payload.is_none() {
        return Some(if operation.payload_digest.is_some() {
            Disposition::PayloadExpired
        } else {
            Disposition::RejectedInvalid
        });
    }
    match canonical_text(operation) {
        Ok(text) => identity::<T>(operation, &text),
        Err(problem) => Some(problem),
    }
}

/// The answer for an event this engine already holds, or `None` when it is
/// new. The same bytes are a `duplicate`; different bytes under the same event
/// id are a `rejected_conflict` — one id cannot name two events. A redacted
/// copy was answered earlier in [`super::validate::decide`].
///
/// # Errors
/// Propagates SQLite failures.
pub(super) fn known_event(conn: &Connection, operation: &Operation) -> Result<Option<Disposition>> {
    if !is_event_kind(&operation.entity_kind) {
        return Ok(None);
    }
    let Some(revision) =
        replica_read::revision(conn, &operation.entity_kind, &operation.entity_key)?
    else {
        return Ok(None);
    };
    Ok(Some(
        if revision.payload_digest == operation.payload_digest {
            Disposition::Duplicate
        } else {
            Disposition::RejectedConflict
        },
    ))
}

impl Identity for FeedbackEventV1 {
    /// The payload's own rules, plus the memory-id shape the memories
    /// capability owns.
    fn holds_for(&self, entity_key: &str) -> bool {
        FeedbackEventV1::holds_for(self, entity_key)
            && match &self.target {
                Target::Memory { id } => is_valid_memory_id(id),
                Target::Code { .. } => true,
            }
    }
}

impl Identity for ActivityEventV1 {
    /// The payload's own rules, the allowlist included.
    fn holds_for(&self, entity_key: &str) -> bool {
        ActivityEventV1::holds_for(self, entity_key)
    }
}

#[cfg(test)]
#[path = "tests/validate_events.rs"]
mod tests;
