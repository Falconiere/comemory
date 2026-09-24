//! Materialize an accepted feedback or activity event (#254): its row, its
//! counter contribution (a verdict only), its feed position and its receipt,
//! in ONE transaction. A process killed mid-envelope leaves each event fully
//! applied or absent, so a counter can never disagree with the verdicts
//! behind it.
//!
//! An imported event carries its origin device, which is how it is told apart
//! from a local one everywhere else: the capture sweep skips it, the recall
//! readers and the golden harvest ignore it, and nothing re-journals it as
//! this machine's.

use crate::domains::learning::replica_payload::{FeedbackEventV1, Target};
use crate::domains::sync::replica::activity_payload::{ACTIVITY_ENTITY_KIND, ActivityEventV1};
use crate::domains::sync::replica::contract::{Operation, OperationResult};
use crate::domains::sync::replica::document_accept::accept;
use crate::domains::sync::replica::validate_events::is_event_kind;
use crate::prelude::*;
use crate::store::activity::{self, NewActivityRow};
use crate::store::code_feedback::{self, SymbolIdentity};
use crate::store::feedback::{self, NewFeedbackEvent};
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use crate::store::{Connection, repository_approval};
use crate::utilities::canonical_json;
use crate::utilities::context::Ctx;
use crate::utilities::telemetry::target;

/// Whether this operation is a feedback or activity event.
#[must_use]
pub(crate) fn handles(operation: &Operation) -> bool {
    is_event_kind(&operation.entity_kind)
}

/// Apply one already-decided event operation.
///
/// # Errors
/// Propagates SQLite failures and payload decoding. A failure leaves no row,
/// no counter change, no position and no receipt: the retry is a fresh
/// attempt.
pub(crate) fn apply(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
    at: &str,
) -> Result<OperationResult> {
    let value = operation
        .payload
        .as_ref()
        .ok_or_else(|| Error::BadRequest("an event carries a payload".to_string()))?;
    let (bytes, digest) = canonical_json::bytes_and_digest(value)?;
    let bytes =
        String::from_utf8(bytes).map_err(|e| Error::BadRequest(format!("event payload: {e}")))?;
    let display = ctx.cfg.activity.clone();
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let (repository, event_at) = if operation.entity_kind == ACTIVITY_ENTITY_KIND {
        let event: ActivityEventV1 = decode(&bytes)?;
        // An imported run is shown only when this machine shows runs at all,
        // and with its summary only when this machine stores summaries.
        if display.enabled {
            let summary = display
                .summaries
                .then(|| event.summary.as_ref().map(serde_json::to_string))
                .flatten()
                .transpose()
                .map_err(|e| Error::Other(format!("activity summary: {e}")))?;
            let repo = local_label(&tx, &event.repo)?;
            activity::insert(
                &tx,
                &NewActivityRow {
                    at: &event.at,
                    command: &event.command,
                    source: &event.source,
                    actor: event.actor.as_deref(),
                    repo: Some(&repo),
                    duration_ms: event.duration_ms,
                    ok: event.ok,
                    error_code: event.error_code.as_deref(),
                    summary: summary.as_deref(),
                    device: Some(&event.device),
                    event_id: Some(&event.event_id),
                },
            )?;
        }
        (Some(event.repo), event.at)
    } else {
        let event: FeedbackEventV1 = decode(&bytes)?;
        materialize_verdict(&tx, &event)?;
        // A memory target names no repository of its own; the sender's
        // operation carries the canonical one it was shared under.
        (
            repository_of(&event).or_else(|| operation.repository.clone()),
            event.at,
        )
    };
    let sequence = replica_journal::append(
        &tx,
        epoch,
        &NewOperation {
            operation_id: &operation.operation_id,
            entity_kind: &operation.entity_kind,
            entity_key: &operation.entity_key,
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: &bytes,
            }),
            schema_version: operation.schema_version,
            repository: repository.as_deref(),
            origin: ReplicaOrigin::Sync,
            at: &event_at,
        },
    )?;
    let result = accept(&tx, operation, epoch, sequence, at)?;
    tx.commit()?;
    Ok(result)
}

/// Write an imported verdict's row and add its one contribution to a counter.
fn materialize_verdict(tx: &Connection, event: &FeedbackEventV1) -> Result<()> {
    let (memory_id, target_kind) = match &event.target {
        Target::Memory { id } => (id.clone(), target::MEMORY),
        Target::Code {
            repo, path, symbol, ..
        } => (format!("{repo}//{path}#{symbol}"), target::CODE),
    };
    feedback::insert_event(
        tx,
        &NewFeedbackEvent {
            query_id: &event.origin_query_id,
            memory_id: &memory_id,
            verdict: &event.verdict,
            at: &event.at,
            target_kind,
            provenance: &event.provenance,
            surface: event.surface.as_deref(),
            actor: event.actor.as_deref(),
            device: Some(&event.device),
            event_id: Some(&event.event_id),
        },
    )?;
    let used = event.verdict == "used";
    match &event.target {
        Target::Memory { id } if used => feedback::upsert_used(tx, id, &event.at),
        Target::Memory { id } => feedback::upsert_irrelevant(tx, id),
        Target::Code {
            repo,
            path,
            symbol,
            version,
        } => {
            let sym = SymbolIdentity {
                repo: local_label(tx, repo)?,
                path: path.clone(),
                symbol: symbol.clone(),
                version: version.clone(),
            };
            if used {
                code_feedback::upsert_used(tx, &sym, &event.at)
            } else {
                code_feedback::upsert_irrelevant(tx, &sym)
            }
        }
    }
}

/// This machine's label for `canonical`, or the canonical name itself when no
/// approved label resolves to it — the hosted engine's case.
fn local_label(tx: &Connection, canonical: &str) -> Result<String> {
    Ok(repository_approval::label_for(tx, canonical)?.unwrap_or_else(|| canonical.to_string()))
}

/// The canonical repository a verdict's target belongs to, when it names one.
fn repository_of(event: &FeedbackEventV1) -> Option<String> {
    match &event.target {
        Target::Code { repo, .. } => Some(repo.clone()),
        Target::Memory { .. } => None,
    }
}

/// Decode a payload the decision already validated.
fn decode<T: serde::de::DeserializeOwned>(bytes: &str) -> Result<T> {
    serde_json::from_str(bytes).map_err(|e| Error::BadRequest(format!("event payload: {e}")))
}

#[cfg(test)]
#[path = "tests/event_accept.rs"]
mod tests;
