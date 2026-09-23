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
use crate::domains::sync::replica::code_accept;
use crate::domains::sync::replica::contract::{Disposition, Operation, OperationResult};
use crate::domains::sync::vector_rule;
use crate::prelude::*;
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::store::replica_receipt::{self, Receipt};
use crate::store::{Connection, memory_purge, memory_row};
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
    // A code generation has no markdown half: its whole state is rows, so it
    // records, writes its projection, activates and journals in one
    // transaction of its own.
    if code_accept::handles(operation) {
        return code_accept::apply(ctx, epoch, operation, &at);
    }
    // The markdown tree is the source of truth and cannot join a SQLite
    // transaction, so it moves first; the database half then commits as one
    // unit below.
    let prepared = prepare_markdown(ctx, operation)?;
    // Judged before the transaction opens, because it reads
    // `schema_meta.memory_vector_model`; carried out inside it, so a memory is
    // never accepted without its vector decision landing with it.
    let verdict = vector_rule::decide(ctx.conn()?, operation.vector.as_ref())?;
    let sequence = commit_acceptance(ctx, epoch, operation, &at, |tx| match &prepared {
        Prepared::Written(record) => {
            mirror::insert_row(
                tx,
                &record.frontmatter,
                &record.body,
                record.slug.as_str(),
                &record.path.to_string_lossy(),
                &record.frontmatter.tags,
            )?;
            vector_rule::apply(tx, &record.frontmatter.id, &verdict, &at)?;
            Ok(journal::record_write(
                tx,
                operation.op,
                &record.frontmatter,
                &record.body,
                &at,
                ReplicaOrigin::Sync,
                None,
            )?
            .sequence)
        }
        Prepared::Removed {
            existed,
            content_hash,
            repository,
        } => {
            if *existed {
                memory_purge::soft_delete(tx, &operation.entity_key, &at)?;
            }
            Ok(journal::record_tombstone(
                tx,
                &operation.entity_key,
                content_hash,
                repository.as_deref(),
                &at,
                ReplicaOrigin::Sync,
                None,
            )?
            .sequence)
        }
    })?;
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

/// What the markdown half of an acceptance produced, for the database half to
/// finish.
enum Prepared {
    /// The record now on disk. Boxed: it dwarfs the tombstone arm, and an
    /// enum sized by its largest variant would make every acceptance carry
    /// that cost.
    Written(Box<crate::domains::memories::MemoryRecord>),
    /// The entity this engine removed, or never held.
    Removed {
        /// Whether a markdown file was actually there to remove.
        existed: bool,
        /// Content hash the removed record carried; empty when it was absent.
        content_hash: String,
        /// Canonical repository, when the removed record had one.
        repository: Option<String>,
    },
}

/// Move the markdown tree to match the operation.
///
/// A tombstone for an id this engine never held still prepares: the peer's
/// deletion is a fact about the shared stream, and recording it is what stops
/// a later pull from re-creating what was deleted.
fn prepare_markdown(ctx: &mut Ctx<'_>, operation: &Operation) -> Result<Prepared> {
    let store = MemoryStore::new(ctx.paths.clone());
    match operation.op {
        ReplicaOp::Upsert | ReplicaOp::Restore => Ok(Prepared::Written(Box::new(write_markdown(
            &store,
            &decode(operation)?,
        )?))),
        ReplicaOp::Tombstone => {
            let removed = match store.delete(&operation.entity_key) {
                Ok(record) => Some(record),
                Err(Error::NotFound(_)) => None,
                Err(e) => return Err(e),
            };
            Ok(Prepared::Removed {
                existed: removed.is_some(),
                content_hash: removed
                    .as_ref()
                    .map_or_else(String::new, |r| r.frontmatter.content_hash.clone()),
                repository: removed
                    .as_ref()
                    .map(|r| r.frontmatter.repo.clone())
                    .filter(|repo| !repo.is_empty()),
            })
        }
    }
}

/// Run `apply` and the receipt it earns in ONE transaction.
///
/// Every accepted operation owes the same three writes — materialized state,
/// journal position, receipt — and the only part that differs is the state.
/// Keeping the transaction here means no acceptance path can forget the
/// receipt or commit it separately.
fn commit_acceptance<F>(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
    at: &str,
    apply: F,
) -> Result<i64>
where
    F: FnOnce(&Connection) -> Result<i64>,
{
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let sequence = apply(&tx)?;
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
            // `created` is part of the payload, so it is part of the
            // revision's identity. Keeping the local value here left the
            // engine acknowledging one digest and storing another: two
            // engines that had each written the same body independently could
            // never agree on a manifest digest, however many times they
            // imported from one another.
            existing.frontmatter.created = payload.created_at()?;
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
///
/// The digest is the one acceptance validated the bytes against, which for an
/// accepted operation is also what it claimed — a replay is matched on it.
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
