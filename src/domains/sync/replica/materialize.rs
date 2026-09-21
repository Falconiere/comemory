//! The write half of acceptance — markdown first, then one transaction that
//! carries the mirror row, both journal feeds and the receipt together.
//!
//! The markdown file is the source of truth and cannot join a SQLite
//! transaction, so it is written first; recovering an interrupted markdown
//! write is issue 251. Everything inside the database commits as one unit: a
//! feed position never exists without its receipt, and a receipt never exists
//! without the state it describes. A failure before the commit leaves no
//! receipt at all, so the peer's retry is a fresh attempt rather than a lost
//! acknowledgement.

use crate::domains::memories::replica_payload::MemoryPayloadV1;
use crate::domains::memories::{MemoryStore, SaveParams, journal, mirror};
use crate::domains::sync::replica::contract::{Disposition, Operation, OperationResult};
use crate::prelude::*;
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::replica_receipt::{self, Receipt};
use crate::store::{memory_purge, memory_row};
use crate::utilities::canonical_json;
use crate::utilities::context::Ctx;

/// Apply one already-decided operation.
///
/// # Errors
/// Propagates markdown and SQLite failures. A failure here leaves no receipt,
/// so the peer's retry is a fresh attempt rather than a lost acknowledgement.
pub(crate) fn apply(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
) -> Result<OperationResult> {
    let at = memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    let sequence = match operation.op {
        ReplicaOp::Upsert | ReplicaOp::Restore => write_memory(ctx, epoch, operation, &at)?,
        ReplicaOp::Tombstone => remove_memory(ctx, epoch, operation, &at)?,
    };
    // After the commit: a failed refresh must not roll back an acceptance the
    // peer has already been told about.
    let _stale = crate::domains::graph::derived::refresh_derived_best_effort(ctx.conn()?);
    Ok(OperationResult {
        operation_id: operation.operation_id.clone(),
        disposition: Disposition::Accepted,
        sequence: Some(sequence),
        payload_digest: operation.payload_digest.clone(),
        reason: None,
    })
}

/// Write (or rewrite) the memory this operation carries, then commit its
/// mirror row, journal rows and receipt as one unit.
fn write_memory(ctx: &mut Ctx<'_>, epoch: &str, operation: &Operation, at: &str) -> Result<i64> {
    let payload = decode(operation)?;
    let record = write_markdown(&MemoryStore::new(ctx.paths.clone()), &payload)?;
    let md_path = record.path.to_string_lossy().into_owned();
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    mirror::insert_row(
        &tx,
        &record.frontmatter,
        &record.body,
        record.slug.as_str(),
        &md_path,
        &record.frontmatter.tags,
    )?;
    let sequence = journal::record_write(
        &tx,
        operation.op,
        &record.frontmatter,
        &record.body,
        at,
        ReplicaOrigin::Sync,
    )?
    .sequence;
    replica_receipt::record(&tx, &accepted(operation, epoch, sequence), at)?;
    tx.commit()?;
    Ok(sequence)
}

/// Write the payload to the markdown tree — rewriting the existing file when
/// this engine already holds the id, otherwise creating it.
///
/// The author is deliberately left empty on a create: the accepting side
/// stamps authorship, so a peer cannot claim it through the payload.
fn write_markdown(
    store: &MemoryStore,
    payload: &MemoryPayloadV1,
) -> Result<crate::domains::memories::MemoryRecord> {
    match store.load(&payload.id) {
        Ok(mut existing) => {
            existing.frontmatter.kind = payload.kind;
            existing.frontmatter.repo.clone_from(&payload.repo);
            existing.frontmatter.tags.clone_from(&payload.tags);
            existing.frontmatter.quality = payload.quality;
            existing
                .frontmatter
                .references
                .clone_from(&payload.references);
            existing
                .frontmatter
                .relations
                .clone_from(&payload.relations);
            // The id is derived from the body, so an upsert under an existing
            // id always carries the same body — copying the hash and schema
            // anyway keeps the file from drifting if that ever changes.
            existing
                .frontmatter
                .content_hash
                .clone_from(&payload.content_hash);
            existing.frontmatter.schema = payload.schema;
            existing.body.clone_from(&payload.body);
            store.rewrite(&existing)?;
            Ok(existing)
        }
        Err(Error::NotFound(_)) => store.save(SaveParams {
            body: &payload.body,
            kind: payload.kind,
            repo: &payload.repo,
            tags: &payload.tags,
            author: "",
            quality: payload.quality,
            relations: payload.relations.clone(),
            references: payload.references.clone(),
            created: Some(payload.created_at()?),
        }),
        Err(e) => Err(e),
    }
}

/// Soft-delete the memory this tombstone names, then commit the mirror
/// delete, journal rows and receipt as one unit.
///
/// A tombstone for an id this engine never held still commits its journal
/// rows: the peer's deletion is a fact about the shared stream, and recording
/// it is what stops a later pull from re-creating what was deleted.
fn remove_memory(ctx: &mut Ctx<'_>, epoch: &str, operation: &Operation, at: &str) -> Result<i64> {
    let paths = ctx.paths.clone();
    let store = MemoryStore::new(paths);
    let removed = match store.delete(&operation.entity_key) {
        Ok(record) => Some(record),
        Err(Error::NotFound(_)) => None,
        Err(e) => return Err(e),
    };
    let content_hash = removed
        .as_ref()
        .map_or_else(String::new, |r| r.frontmatter.content_hash.clone());
    let repository = removed
        .as_ref()
        .map(|r| r.frontmatter.repo.clone())
        .filter(|repo| !repo.is_empty());

    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    if removed.is_some() {
        memory_purge::soft_delete(&tx, &operation.entity_key, at)?;
    }
    let sequence = journal::record_tombstone(
        &tx,
        &operation.entity_key,
        &content_hash,
        repository.as_deref(),
        at,
        ReplicaOrigin::Sync,
    )?
    .sequence;
    replica_receipt::record(&tx, &accepted(operation, epoch, sequence), at)?;
    tx.commit()?;
    Ok(sequence)
}

/// Decode the payload the decision already validated.
fn decode(operation: &Operation) -> Result<MemoryPayloadV1> {
    let value = operation
        .payload
        .as_ref()
        .ok_or_else(|| Error::BadRequest("operation carries no payload".to_string()))?;
    let bytes = canonical_json::to_bytes(value)?;
    let text = String::from_utf8(bytes).map_err(|e| Error::BadRequest(format!("payload: {e}")))?;
    MemoryPayloadV1::decode(&text)
}

/// The receipt an acceptance writes.
fn accepted(operation: &Operation, epoch: &str, sequence: i64) -> Receipt {
    Receipt {
        operation_id: operation.operation_id.clone(),
        epoch: epoch.to_string(),
        sequence: Some(sequence),
        disposition: Disposition::Accepted.as_str().to_string(),
        payload_digest: operation.payload_digest.clone(),
        reason: None,
    }
}
