//! `GET /sync/replica/manifest` — what this engine holds, and whether it is
//! ready to be replicated from.
//!
//! `capabilities` is empty until journal seeding completes, which is how a
//! peer tells "nothing to send" from "not finished remembering what I have".

use crate::domains::code::replica_payload::{CODE_ENTITY_KIND, CODE_PAYLOAD_VERSION};
use crate::domains::documents::replica_payload::{DOCUMENT_ENTITY_KIND, DOCUMENT_PAYLOAD_VERSION};
use crate::domains::learning::replica_payload::{FEEDBACK_ENTITY_KIND, FEEDBACK_PAYLOAD_VERSION};
use crate::domains::memories::replica_payload::{MEMORY_ENTITY_KIND, MEMORY_PAYLOAD_VERSION};
use crate::domains::sync::exchange::manifest::bucket_digests;
use crate::domains::sync::replica::activity_payload::{
    ACTIVITY_ENTITY_KIND, ACTIVITY_PAYLOAD_VERSION,
};
use crate::domains::sync::replica::bootstrap;
use crate::domains::sync::replica::contract::PROTOCOL;
use crate::domains::sync::replica::contract_views::{
    BootstrapStatus, KindManifest, ManifestResponse,
};
use crate::domains::sync::replica::{event_capture, seed_documents, seed_trash};
use crate::prelude::*;
use crate::store::replica_journal::stream_epoch;
use crate::store::{needs_embedding, replica_read};
use crate::utilities::context::Ctx;

/// Report the stream, its head, the per-kind digests and seeding progress.
///
/// # Errors
/// Propagates SQLite failures.
pub fn run(ctx: &mut Ctx<'_>) -> Result<ManifestResponse> {
    let memory = bootstrap::advance(ctx)?;
    let trash = seed_trash::advance(ctx)?;
    let documents = seed_documents::advance(ctx)?;
    let verdicts = event_capture::advance(ctx)?;
    let complete =
        memory.complete() && trash.complete() && documents.complete() && verdicts.complete();
    let state = merged_state(
        complete,
        [
            memory.state != "pending",
            trash.state != "pending",
            documents.state != "pending",
            verdicts.state != "pending",
        ],
    );
    let conn = ctx.conn()?;
    let stream = stream_epoch(conn)?;
    let head_sequence = replica_read::head(conn)?;
    let entity_kinds: Vec<KindManifest> = replica_read::kind_digests(conn)?
        .into_iter()
        .map(|(kind, digests)| KindManifest {
            kind,
            schema_version: MEMORY_PAYLOAD_VERSION,
            count: i64::try_from(digests.len()).unwrap_or(i64::MAX),
            buckets: bucket_digests(digests),
        })
        .collect();
    let seeded = entity_kinds.iter().map(|k| k.count).sum();
    Ok(ManifestResponse {
        protocol: PROTOCOL.to_string(),
        stream_epoch: stream,
        head_sequence,
        capabilities: if complete { advertised() } else { Vec::new() },
        entity_kinds,
        bootstrap: BootstrapStatus { state, seeded },
        needs_embedding: i64::try_from(needs_embedding::pending(conn)?.len()).unwrap_or(i64::MAX),
    })
}

/// One seeding state out of the memory, trash, document and verdict walks,
/// in the wire's three values: `pending` until any has started, `seeding`
/// until every one of them reports complete, `complete` only then.
fn merged_state(all_complete: bool, started: [bool; 4]) -> String {
    if all_complete {
        bootstrap::STATE_COMPLETE.to_string()
    } else if started.into_iter().any(|s| s) {
        "seeding".to_string()
    } else {
        "pending".to_string()
    }
}

/// What a seeded engine advertises: the protocol, then one `<kind>@<version>`
/// per payload it can accept — how a client learns which of its operations
/// this engine can read before it spends an operation id on a refusal.
#[must_use]
pub fn advertised() -> Vec<String> {
    vec![
        PROTOCOL.to_string(),
        format!("{MEMORY_ENTITY_KIND}@{MEMORY_PAYLOAD_VERSION}"),
        format!("{CODE_ENTITY_KIND}@{CODE_PAYLOAD_VERSION}"),
        format!("{DOCUMENT_ENTITY_KIND}@{DOCUMENT_PAYLOAD_VERSION}"),
        format!("{FEEDBACK_ENTITY_KIND}@{FEEDBACK_PAYLOAD_VERSION}"),
        format!("{ACTIVITY_ENTITY_KIND}@{ACTIVITY_PAYLOAD_VERSION}"),
    ]
}

#[cfg(test)]
#[path = "tests/manifest.rs"]
mod tests;
