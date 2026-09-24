//! Record-time journalling of a shareable verdict (#254), inside the
//! transaction that writes its `feedback_events` row and bumps its counter.
//! That transaction is the only moment a code rowid is known to mean the
//! symbol the payload names, and it keeps a verdict from existing here while
//! owing no feed position. A verdict that may not be shared journals nothing
//! and is counted locally as before. Nothing is enqueued on the outbox: the
//! feed position is what a push reads (#255).

use crate::domains::learning::replica_payload::{
    FEEDBACK_ENTITY_KIND, FEEDBACK_PAYLOAD_VERSION, FeedbackEventV1, SHAREABLE_PROVENANCE, Target,
    mint_event_id,
};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::code_feedback::SymbolIdentity;
use crate::store::feedback_share::{self, LegacyVerdict};
use crate::store::replica_journal::{self, LocalEvent, PayloadRef};
use crate::store::{memory_repository, replica_device, repository_approval};
use crate::utilities::activity::Origin;
use crate::utilities::shared_text;

/// Who stated a verdict: the delivery surface and the declared caller label,
/// both unknown for an internal reward and for a verdict recorded before v26.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Caller<'a> {
    /// `cli`, `http` or `mcp`.
    pub(crate) surface: Option<&'a str>,
    /// Declared caller label.
    pub(crate) actor: Option<&'a str>,
}

impl<'a> Caller<'a> {
    /// The caller behind a command core's [`Origin`].
    #[must_use]
    pub(crate) fn of(origin: &'a Origin) -> Self {
        Self {
            surface: Some(origin.source),
            actor: origin.actor.as_deref(),
        }
    }
}

/// What one recorded verdict judged.
pub(crate) enum Judged<'a> {
    /// A memory id.
    Memory(&'a str),
    /// A code symbol, already resolved to its stable identity.
    Code(&'a SymbolIdentity),
}

/// One verdict just written to `feedback_events`, as the journal needs it.
pub(crate) struct Recorded<'a> {
    /// The row's id, stamped with the event id once journalled.
    pub(crate) row_id: i64,
    /// The local `q-…` id the verdict cites.
    pub(crate) query_id: &'a str,
    /// `used` or `irrelevant`.
    pub(crate) verdict: &'a str,
    /// The row's `at`.
    pub(crate) at: &'a str,
    /// The row's provenance.
    pub(crate) provenance: &'a str,
    /// Who stated it.
    pub(crate) caller: Caller<'a>,
    /// What it judged.
    pub(crate) target: Judged<'a>,
}

/// Journal one just-recorded verdict if it may be shared, in the caller's
/// transaction. Returns the feed sequence, or `None` when it stays local.
///
/// # Errors
/// Propagates SQLite failures and payload serialization; the caller's
/// transaction then rolls the verdict back with it.
pub(crate) fn journal(tx: &Connection, verdict: &Recorded<'_>) -> Result<Option<i64>> {
    if !SHAREABLE_PROVENANCE.contains(&verdict.provenance) {
        return Ok(None);
    }
    let Some((canonical, target)) = scope(tx, &verdict.target)? else {
        return Ok(None);
    };
    let device = replica_device::id(tx)?;
    let event_id = mint_event_id()?;
    let payload = FeedbackEventV1 {
        event_id: event_id.clone(),
        origin_query_id: format!("{device}:{}", verdict.query_id),
        device,
        at: verdict.at.to_string(),
        verdict: verdict.verdict.to_string(),
        provenance: verdict.provenance.to_string(),
        surface: verdict.caller.surface.map(str::to_string),
        actor: verdict.caller.actor.and_then(shared_text::label_for_share),
        target,
    };
    let (bytes, digest) = payload.canonical()?;
    let sequence = replica_journal::append_local_event(
        tx,
        &LocalEvent {
            entity_kind: FEEDBACK_ENTITY_KIND,
            event_id: &event_id,
            schema_version: FEEDBACK_PAYLOAD_VERSION,
            repository: &canonical,
            payload: PayloadRef {
                digest: &digest,
                bytes: &bytes,
            },
            at: verdict.at,
        },
    )?;
    feedback_share::stamp_event_id(tx, verdict.row_id, &event_id)?;
    Ok(Some(sequence))
}

/// Journal one verdict recorded before sharing existed — the backfill's unit.
/// It never touches a counter: this machine's counters already include it.
///
/// # Errors
/// As [`journal`].
pub(crate) fn journal_retained(tx: &Connection, legacy: &LegacyVerdict) -> Result<Option<i64>> {
    journal(
        tx,
        &Recorded {
            row_id: legacy.id,
            query_id: &legacy.query_id,
            verdict: &legacy.verdict,
            at: &legacy.at,
            provenance: &legacy.provenance,
            caller: Caller::default(),
            target: Judged::Memory(&legacy.memory_id),
        },
    )
}

/// The canonical repository a target is shared under, and the target as a
/// peer names it — or `None` when the target has no approved repository.
fn scope(tx: &Connection, judged: &Judged<'_>) -> Result<Option<(String, Target)>> {
    let label = match judged {
        Judged::Memory(id) => memory_repository::label(tx, id)?,
        Judged::Code(sym) => Some(sym.repo.clone()),
    };
    let Some(label) = label else {
        return Ok(None);
    };
    let Some(canonical) = repository_approval::canonical_for(tx, &label)? else {
        return Ok(None);
    };
    let target = match judged {
        Judged::Memory(id) => Target::Memory {
            id: (*id).to_string(),
        },
        Judged::Code(sym) => Target::Code {
            repo: canonical.clone(),
            path: sym.path.clone(),
            symbol: sym.symbol.clone(),
            version: sym.version.clone(),
        },
    };
    Ok(Some((canonical, target)))
}

#[cfg(test)]
#[path = "tests/feedback_share.rs"]
mod tests;
