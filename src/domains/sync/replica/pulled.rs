//! Apply ONE entry pulled from an upstream to this database, in the order the
//! upstream already chose (#255).
//!
//! The import route decides; a client following a pulled feed does not. So
//! this path keeps everything acceptance validates about the entry itself —
//! a receipt replay answers `duplicate`, the kind and payload must read and
//! hold together, an erased payload stays erased, an event already held is
//! never counted again — and skips what acceptance
//! decides against local state: the tombstone/restore ordering (its observed
//! sequences are in the upstream's sequence space) and a code generation's
//! parent check. A refusal records no receipt, so a pull that stalled on it
//! succeeds once the cause is fixed.
//!
//! The derived graph is NOT refreshed per entry; the pull refreshes once per
//! page.

use crate::domains::sync::replica::accept;
use crate::domains::sync::replica::contract::{Disposition, Operation};
use crate::domains::sync::replica::contract_views::{ChangeEntry, PayloadState};
use crate::domains::sync::replica::materialize::{self, Order};
use crate::domains::sync::replica::{validate, validate_events};
use crate::prelude::*;
use crate::store::replica_journal::stream_epoch;
use crate::store::replica_read::Redaction;
use crate::store::replica_redaction;
use crate::utilities::context::Ctx;
use crate::utilities::operation_id;

/// What applying one pulled entry did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pulled {
    /// State, feed row and receipt were written.
    Applied,
    /// A receipt already answered this operation; nothing was written.
    Duplicate,
    /// The entry cannot be applied as it stands; nothing was written.
    Refused(Disposition),
}

/// The operation a pulled entry describes, as the import route would have
/// received it.
#[must_use]
pub fn operation_of(entry: &ChangeEntry) -> Operation {
    Operation {
        operation_id: entry.operation_id.clone(),
        entity_kind: entry.entity_kind.clone(),
        entity_key: entry.entity_key.clone(),
        op: entry.op,
        schema_version: entry.schema_version,
        payload_digest: entry.payload_digest.clone(),
        payload: entry.payload.clone(),
        observed_sequence: None,
        repository: entry.repository.clone(),
        vector: None,
    }
}

/// How a pulled entry is journalled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Under its own operation id; a receipt replay answers `duplicate`.
    Received,
    /// Under a fresh id, rewriting local state to the entry's payload — a
    /// compacting replay found this engine holding a different revision than
    /// the one the upstream ends with, while the entry's own id already has a
    /// receipt (or is this engine's own operation).
    Rewrite,
}

/// Apply one pulled entry in upstream order.
///
/// # Errors
/// Propagates markdown and SQLite failures, which the caller treats as a
/// stall; a refusal is [`Pulled::Refused`], not an error.
pub fn apply(ctx: &mut Ctx<'_>, entry: &ChangeEntry, landing: Landing) -> Result<Pulled> {
    let mut operation = operation_of(entry);
    match landing {
        Landing::Rewrite => {
            operation.operation_id =
                operation_id::mint(&entry.entity_kind, &entry.entity_key, entry.op.as_str());
        }
        Landing::Received if accept::replayed(ctx.conn()?, &operation)?.is_some() => {
            return Ok(Pulled::Duplicate);
        }
        Landing::Received => {}
    }
    if let Some(refusal) = validate::kind_and_shape(&operation) {
        return Ok(Pulled::Refused(refusal));
    }
    let redaction = match operation.payload_digest.as_deref() {
        Some(digest) => replica_redaction::redaction_of(ctx.conn()?, digest)?,
        None => None,
    };
    match (entry.payload_state, redaction) {
        (PayloadState::Erased, _) | (_, Some(Redaction::Erased)) => {
            return Ok(Pulled::Refused(Disposition::PayloadErased));
        }
        (PayloadState::Expired, _) | (_, Some(Redaction::Expired)) => {
            return Ok(Pulled::Refused(Disposition::PayloadExpired));
        }
        _ => {}
    }
    // An event is immutable and counted once (#254): one this engine already
    // holds is answered, never applied again — whatever id carries it, and
    // under either landing.
    if let Some(known) = validate_events::known_event(ctx.conn()?, &operation)? {
        return Ok(match known {
            Disposition::Duplicate => Pulled::Duplicate,
            other => Pulled::Refused(other),
        });
    }
    let epoch = stream_epoch(ctx.conn()?)?;
    materialize::apply(ctx, &epoch, &operation, Order::Upstream)?;
    Ok(Pulled::Applied)
}

#[cfg(test)]
#[path = "tests/pulled.rs"]
mod tests;
