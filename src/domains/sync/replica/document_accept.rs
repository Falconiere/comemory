//! Materialize an accepted document revision: replace the pulled cache for one
//! document, journal the position and record the receipt in one transaction.
//!
//! That commit IS the activation — there is no `staged` state to flip, and an
//! upload still arriving never reaches here (its parts sit in
//! `replica_staged_part`). Between a half-written revision and the rest of it a
//! reader would see passages from two texts of one document.
//!
//! Every table touched here is one only a pull writes, which is what makes "an
//! import writes no local row" structural rather than remembered.
//!
//! Shape and identity are already decided by `validate::document_identity`, so
//! nothing is re-checked here: a revision whose key, ordinals or links do not
//! hold together never reaches this module.

use crate::domains::documents::replica_payload::{DOCUMENT_ENTITY_KIND, DocumentRevisionV1};
use crate::domains::sync::replica::contract::{Disposition, Operation, OperationResult};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::remote_document::{self, Chunk, Link, Revision};
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use crate::store::replica_receipt::{self, Receipt};
use crate::utilities::context::Ctx;

/// Whether this operation is a document revision.
#[must_use]
pub(crate) fn handles(operation: &Operation) -> bool {
    operation.entity_kind == DOCUMENT_ENTITY_KIND
}

/// Apply one already-decided document-revision operation.
///
/// # Errors
/// Propagates SQLite failures and payload decoding. A failure leaves no
/// receipt and no rows, so the peer's retry is a fresh attempt.
pub(crate) fn apply(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
    at: &str,
) -> Result<OperationResult> {
    let payload = decode(operation)?;
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let sequence = materialize(&tx, epoch, operation, payload.as_ref(), at)?;
    let result = accept(&tx, operation, epoch, sequence, at)?;
    tx.commit()?;
    Ok(result)
}

/// Write what arrived and append the feed position it earns.
///
/// An absent payload is a tombstone: an authoritative deletion of that one
/// document. The local `documents` row, if this machine has one, is untouched
/// — the peer is saying what IT no longer holds.
fn materialize(
    tx: &Connection,
    epoch: &str,
    operation: &Operation,
    payload: Option<&DocumentRevisionV1>,
    at: &str,
) -> Result<i64> {
    let shared_id = operation.entity_key.as_str();
    let (bytes, digest) = payload
        .map(DocumentRevisionV1::canonical)
        .transpose()?
        .unzip();
    let repo = match payload {
        Some(payload) => payload.repo.clone(),
        None => operation.repository.clone().unwrap_or_default(),
    };
    match payload {
        Some(payload) => write_revision(tx, payload, at)?,
        None => remote_document::purge(tx, &repo, shared_id)?,
    }
    journal(
        tx,
        &Journalled {
            epoch,
            operation,
            shared_id,
            repo: &repo,
            payload: bytes.as_deref().zip(digest.as_deref()),
            at,
        },
    )
}

/// Replace the pulled cache for this one document.
fn write_revision(tx: &Connection, payload: &DocumentRevisionV1, at: &str) -> Result<()> {
    let revision = Revision {
        repo: payload.repo.clone(),
        shared_id: payload.shared_id.clone(),
        path: payload.path.clone(),
        title: payload.title.clone(),
        format: payload.format.clone(),
        revision_hash: payload.revision_hash.clone(),
        chunk_count: i64::try_from(payload.chunks.len()).unwrap_or(i64::MAX),
    };
    let chunks: Vec<Chunk> = payload
        .chunks
        .iter()
        .map(|c| Chunk {
            ordinal: c.ordinal,
            heading_path: c.heading_path.clone(),
            char_range: (c.char_start, c.char_end),
            line_range: (c.line_start, c.line_end),
            simhash: c.simhash,
            text: c.text.clone(),
        })
        .collect();
    let links: Vec<Link> = payload
        .links
        .iter()
        .map(|l| Link {
            ordinal: l.ordinal,
            target: l.target.clone(),
        })
        .collect();
    remote_document::replace_revision(tx, &revision, &chunks, &links, at)
}

/// Record the acceptance receipt and the answer the sender reads back.
///
/// Shared with [`super::event_accept`]: every kind whose whole state is rows
/// owes the same receipt, in the transaction that wrote those rows.
pub(super) fn accept(
    tx: &Connection,
    operation: &Operation,
    epoch: &str,
    sequence: i64,
    at: &str,
) -> Result<OperationResult> {
    replica_receipt::record(
        tx,
        &Receipt {
            operation_id: operation.operation_id.clone(),
            epoch: epoch.to_string(),
            sequence: Some(sequence),
            disposition: Disposition::Accepted.as_str().to_string(),
            payload_digest: operation.payload_digest.clone(),
            reason: None,
        },
        at,
    )?;
    Ok(OperationResult {
        operation_id: operation.operation_id.clone(),
        disposition: Disposition::Accepted,
        sequence: Some(sequence),
        payload_digest: operation.payload_digest.clone(),
        reason: None,
    })
}

/// One feed append's inputs.
struct Journalled<'a> {
    epoch: &'a str,
    operation: &'a Operation,
    shared_id: &'a str,
    repo: &'a str,
    /// `(bytes, digest)`; `None` for a tombstone.
    payload: Option<(&'a str, &'a str)>,
    at: &'a str,
}

/// Append the feed position this acceptance earns.
fn journal(tx: &Connection, new: &Journalled<'_>) -> Result<i64> {
    let payload = new
        .payload
        .map(|(bytes, digest)| PayloadRef { digest, bytes });
    replica_journal::append(
        tx,
        new.epoch,
        &NewOperation {
            operation_id: &new.operation.operation_id,
            entity_kind: DOCUMENT_ENTITY_KIND,
            entity_key: new.shared_id,
            op: new.operation.op,
            payload,
            schema_version: new.operation.schema_version,
            repository: Some(new.repo),
            origin: ReplicaOrigin::Sync,
            at: new.at,
        },
    )
}

/// The payload a write carries, or `None` for a tombstone.
fn decode(operation: &Operation) -> Result<Option<DocumentRevisionV1>> {
    if operation.op == ReplicaOp::Tombstone {
        return Ok(None);
    }
    let Some(value) = operation.payload.as_ref() else {
        return Err(Error::BadRequest(
            "a document revision upsert carries a payload".to_string(),
        ));
    };
    let text = serde_json::to_string(value)
        .map_err(|e| Error::Other(format!("document revision payload: {e}")))?;
    DocumentRevisionV1::decode(&text).map(Some)
}

#[cfg(test)]
#[path = "tests/document_accept.rs"]
mod tests;
