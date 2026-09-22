//! The `replica-v1` read and staging shapes — what `changes`, `manifest`,
//! `stage` and `activate` carry.
//!
//! Split from [`super::contract`] (the import envelope) to stay under the
//! file-size ceiling; both halves are one protocol version.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domains::sync::replica::contract::{CursorRef, Operation};
use crate::store::replica_journal::ReplicaOp;

/// Whether a change entry carries its payload, and why not when it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadState {
    /// Bytes are attached.
    Present,
    /// The operation never had a payload (a tombstone).
    Absent,
    /// The bytes were permanently erased; the digest remains as the barrier.
    Erased,
}

/// One accepted position on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeEntry {
    /// Server-assigned position.
    pub sequence: i64,
    /// The operation accepted here.
    pub operation_id: String,
    /// Entity kind.
    pub entity_kind: String,
    /// Entity key within its kind.
    pub entity_key: String,
    /// What the operation did.
    pub op: ReplicaOp,
    /// Payload schema version at acceptance.
    pub schema_version: i64,
    /// Digest of the payload this position named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_digest: Option<String>,
    /// The bytes accepted at this position — never today's bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Why a payload is absent, when it is.
    pub payload_state: PayloadState,
    /// Canonical repository, when the entity has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// RFC3339 provenance time.
    pub at: String,
}

/// `GET /sync/replica/changes` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangesResponse {
    /// Always [`PROTOCOL`].
    pub protocol: String,
    /// The serving stream's epoch.
    pub stream_epoch: String,
    /// Highest sequence in the feed.
    pub head_sequence: i64,
    /// Cursor for the next page; `null` when the page is empty.
    pub next_sequence: Option<i64>,
    /// Positions above the requested cursor, ascending.
    pub entries: Vec<ChangeEntry>,
}

/// One entity kind's manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindManifest {
    /// Entity kind.
    pub kind: String,
    /// Payload schema version this engine writes.
    pub schema_version: i64,
    /// Live entities of this kind.
    pub count: i64,
    /// 256 digests keyed by the first two hex characters of `payload_digest`.
    pub buckets: Vec<String>,
}

/// How far journal seeding has progressed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapStatus {
    /// `pending`, `seeding` or `complete`.
    pub state: String,
    /// Entities journalled so far — what a peer can see progressing.
    pub seeded: i64,
}

/// `GET /sync/replica/manifest` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestResponse {
    /// Always [`PROTOCOL`].
    pub protocol: String,
    /// The serving stream's epoch.
    pub stream_epoch: String,
    /// Highest sequence in the feed.
    pub head_sequence: i64,
    /// `["replica-v1"]` once seeding is complete; empty until then.
    pub capabilities: Vec<String>,
    /// One manifest per entity kind present.
    pub entity_kinds: Vec<KindManifest>,
    /// Seeding progress.
    pub bootstrap: BootstrapStatus,
    /// How many memories hold text but no usable vector.
    ///
    /// Reported here so an operator sees the backlog without a second call:
    /// a peer whose embedder differs replicates every memory correctly and
    /// still answers semantic search short until these are re-embedded.
    pub needs_embedding: i64,
}

/// `POST /sync/replica/stage` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRequest {
    /// Must be [`PROTOCOL`].
    pub protocol: String,
    /// Upload this part belongs to.
    pub staging_id: String,
    /// Zero-based part position.
    pub part_index: i64,
    /// How many parts the upload has.
    pub part_count: i64,
    /// This part's bytes.
    pub bytes: String,
}

/// `POST /sync/replica/stage` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageResponse {
    /// Always [`PROTOCOL`].
    pub protocol: String,
    /// Upload the part belongs to.
    pub staging_id: String,
    /// Parts received so far.
    pub received: i64,
    /// Parts declared.
    pub declared: i64,
    /// Whether every declared part has arrived.
    pub complete: bool,
}

/// `POST /sync/replica/activate` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateRequest {
    /// Must be [`PROTOCOL`].
    pub protocol: String,
    /// Upload to assemble.
    pub staging_id: String,
    /// The operation the assembled bytes complete. Its `payload` must be
    /// absent — the staged parts are the payload.
    pub operation: Operation,
    /// The peer's position, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<CursorRef>,
}
