//! `GET /sync/replica/manifest` — what this engine holds, and whether it is
//! ready to be replicated from.
//!
//! `capabilities` is empty until journal seeding completes, which is how a
//! peer tells "nothing to send" from "not finished remembering what I have".

use crate::domains::memories::replica_payload::MEMORY_PAYLOAD_VERSION;
use crate::domains::sync::exchange::manifest::bucket_digests;
use crate::domains::sync::replica::bootstrap;
use crate::domains::sync::replica::contract::PROTOCOL;
use crate::domains::sync::replica::contract_views::{
    BootstrapStatus, KindManifest, ManifestResponse,
};
use crate::prelude::*;
use crate::store::replica_journal::stream_epoch;
use crate::store::replica_read;
use crate::utilities::context::Ctx;

/// Report the stream, its head, the per-kind digests and seeding progress.
///
/// # Errors
/// Propagates SQLite failures.
pub fn run(ctx: &mut Ctx<'_>) -> Result<ManifestResponse> {
    let progress = bootstrap::advance(ctx)?;
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
        capabilities: if progress.complete() {
            vec![PROTOCOL.to_string()]
        } else {
            Vec::new()
        },
        entity_kinds,
        bootstrap: BootstrapStatus {
            state: progress.state,
            seeded,
        },
    })
}

#[cfg(test)]
#[path = "tests/manifest.rs"]
mod tests;
