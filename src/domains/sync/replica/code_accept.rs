//! Materialize an accepted code generation: record it, write its projection,
//! activate it and journal the position — all in one transaction.
//!
//! Atomicity is the point. A reader between a half-written projection and its
//! activation would see a repo that never existed: some files at the old head,
//! some at the new, with import edges pointing at both. Activation is the
//! single instant the whole generation becomes visible.

use crate::domains::code::replica_payload::{CODE_ENTITY_KIND, CodeGenerationV1};
use crate::domains::sync::replica::contract::{Disposition, Operation, OperationResult};
use crate::prelude::*;
use crate::store::code_generation::{self, Generation, State};
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use crate::store::replica_receipt::{self, Receipt};
use crate::store::{Connection, remote_code};
use crate::utilities::context::Ctx;

/// Whether this operation is a code generation.
#[must_use]
pub(crate) fn handles(operation: &Operation) -> bool {
    operation.entity_kind == CODE_ENTITY_KIND
}

/// Apply one already-decided code-generation operation.
///
/// # Errors
/// Propagates SQLite failures and payload decoding. A failure leaves no
/// receipt and no activation, so the peer's retry is a fresh attempt.
pub(crate) fn apply(
    ctx: &mut Ctx<'_>,
    epoch: &str,
    operation: &Operation,
    at: &str,
) -> Result<OperationResult> {
    let repo = operation.entity_key.clone();
    let payload = decode(operation)?;
    let (bytes, digest) = payload
        .as_ref()
        .map(CodeGenerationV1::canonical)
        .transpose()?
        .unzip();

    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    // The absent payload is a tombstone, which for this kind means the
    // authoritative generation is empty — the repo still exists, it just
    // holds nothing. It never means the sender lost its checkout.
    let sequence = if let Some(payload) = payload.as_ref() {
        activate_generation(&tx, &repo, payload, at)?;
        journal(
            &tx,
            epoch,
            operation,
            &repo,
            bytes.as_deref(),
            digest.as_deref(),
            at,
        )?
    } else {
        clear_projection(&tx, &repo)?;
        journal(&tx, epoch, operation, &repo, None, None, at)?
    };
    replica_receipt::record(
        &tx,
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
    tx.commit()?;
    Ok(OperationResult {
        operation_id: operation.operation_id.clone(),
        disposition: Disposition::Accepted,
        sequence: Some(sequence),
        payload_digest: operation.payload_digest.clone(),
        reason: None,
    })
}

/// Record the generation, write its projection and make it the active one.
fn activate_generation(
    tx: &Connection,
    repo: &str,
    payload: &CodeGenerationV1,
    at: &str,
) -> Result<()> {
    let projection = payload.projection();
    let file_count = i64::try_from(projection.files.len()).unwrap_or(i64::MAX);
    let manifest_digest = payload.canonical()?.1;
    code_generation::record(
        tx,
        &Generation {
            repo: repo.to_string(),
            generation_id: payload.generation_id.clone(),
            parent_id: payload.parent_id.clone(),
            head: payload.head.clone(),
            mined_commit: payload.mined_commit.clone(),
            origin: ReplicaOrigin::Sync,
            state: State::Staged,
            file_count,
            manifest_digest,
        },
        at,
    )?;
    remote_code::replace_generation(tx, repo, &payload.generation_id, &projection)?;
    code_generation::activate(tx, repo, &payload.generation_id, at)
}

/// Drop the repo's pulled projection for an authoritative empty generation.
fn clear_projection(tx: &Connection, repo: &str) -> Result<()> {
    for generation in code_generation::all(tx, repo)? {
        remote_code::purge_generation(tx, repo, &generation.generation_id)?;
    }
    Ok(())
}

/// Append the feed position this acceptance earns.
fn journal(
    tx: &Connection,
    epoch: &str,
    operation: &Operation,
    repo: &str,
    bytes: Option<&str>,
    digest: Option<&str>,
    at: &str,
) -> Result<i64> {
    let payload = bytes
        .zip(digest)
        .map(|(bytes, digest)| PayloadRef { digest, bytes });
    replica_journal::append(
        tx,
        epoch,
        &NewOperation {
            operation_id: &operation.operation_id,
            entity_kind: CODE_ENTITY_KIND,
            entity_key: repo,
            op: operation.op,
            payload,
            schema_version: operation.schema_version,
            repository: Some(repo),
            origin: ReplicaOrigin::Sync,
            at,
        },
    )
}

/// The payload a write carries, or `None` for a tombstone.
fn decode(operation: &Operation) -> Result<Option<CodeGenerationV1>> {
    if operation.op == ReplicaOp::Tombstone {
        return Ok(None);
    }
    let Some(value) = operation.payload.as_ref() else {
        return Err(Error::BadRequest(
            "a code generation upsert carries a payload".to_string(),
        ));
    };
    let text = serde_json::to_string(value)
        .map_err(|e| Error::Other(format!("code generation payload: {e}")))?;
    CodeGenerationV1::decode(&text).map(Some)
}

#[cfg(test)]
#[path = "tests/code_accept.rs"]
mod tests;
