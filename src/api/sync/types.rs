//! Wire types for the sync HTTP surface.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::memory::frontmatter::{Kind, References, Relations};
pub use crate::store::sync_log::SyncOp;

/// Schema-1 frontmatter on the wire minus `author` (handled separately on
/// import and echoed top-level on pull).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WireFrontmatter {
    /// 8-hex content-derived id.
    pub id: String,
    /// Memory taxonomy kind.
    pub kind: Kind,
    /// Owning repo label.
    pub repo: String,
    /// Tag list.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Creation timestamp (ISO-8601).
    #[serde(with = "iso8601_serde")]
    pub created: OffsetDateTime,
    /// Quality rating `1..=5`.
    pub quality: u8,
    /// Frontmatter schema version (`1` today).
    pub schema: u32,
    /// 64-hex body digest.
    pub content_hash: String,
    /// Symbol/file references.
    #[serde(default)]
    pub references: References,
    /// Cross-memory relations.
    #[serde(default)]
    pub relations: Relations,
}

/// Optional dense vector carried on the wire for bootstrap efficiency.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SyncVector {
    /// Embedder model id recorded in `schema_meta.memory_vector_model`.
    pub model: String,
    /// Vector dimension (1024 for memories).
    pub dims: u32,
    /// Base64-encoded little-endian `f32` payload (`dims * 4` bytes).
    pub f32: String,
}

/// Markdown body + frontmatter (+ optional vector) attached to upsert/restore
/// entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SyncRecord {
    /// Schema-1 frontmatter minus `author`.
    pub frontmatter: WireFrontmatter,
    /// Markdown body.
    pub body: String,
    /// Optional BYO embedding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<SyncVector>,
}

/// One sync-log row enriched for `GET /sync/changes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncEntry {
    /// Server-assigned monotonic sequence.
    pub seq: i64,
    /// Operation kind.
    pub op: SyncOp,
    /// 8-hex memory id.
    pub id: String,
    /// 64-hex content hash at the time of the op.
    pub content_hash: String,
    /// Client provenance timestamp (ISO-8601).
    pub at: String,
    /// Author stamped by the server on import; from frontmatter on pull.
    pub author: String,
    /// Live record payload; absent for tombstones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<SyncRecord>,
}

/// Import batch entry (`Omit<SyncEntry,'seq'|'author'>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportEntry {
    /// Operation kind.
    pub op: SyncOp,
    /// 8-hex memory id.
    pub id: String,
    /// 64-hex content hash.
    pub content_hash: String,
    /// Client provenance timestamp (ISO-8601).
    pub at: String,
    /// Payload for upsert/restore ops.
    #[serde(default)]
    pub record: Option<SyncRecord>,
}

/// `POST /sync/import` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    /// Pusher's last successfully pulled server `seq`.
    pub cursor: i64,
    /// Batch of changes (≤500).
    pub entries: Vec<ImportEntry>,
}

/// Per-entry import disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    /// Applied and logged.
    Accepted,
    /// No-op — already present with the same bytes/metadata.
    Exists,
    /// Rejected — pusher has not seen a newer tombstone.
    Stale,
    /// Tombstone recorded for an unknown id (convergence log only).
    Deleted,
    /// Rejected — id already bound to a different body hash.
    IdCollision,
    /// Rejected — body matched the secret rule set.
    SecretDetected,
    /// Rejected — Worker allowlist gate (org repo / personal sync off).
    RepoNotAllowed,
    /// Rejected — schema/hash/id validation failed.
    Invalid,
}

/// One row of the import result list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportItemResult {
    /// Memory id from the wire entry.
    pub id: String,
    /// Content hash from the wire entry.
    pub content_hash: String,
    /// Import disposition.
    pub status: ImportStatus,
    /// Assigned server `seq` when a log row was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    /// Near-duplicate advisory from the SimHash scan (rule 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    /// Human-readable detail (validation errors, secret rule name, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `GET /sync/changes` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangesResponse {
    /// Enriched log rows above `since`.
    pub entries: Vec<SyncEntry>,
    /// Last returned `seq`, or `null` when `entries` is empty.
    pub next_seq: Option<i64>,
    /// Highest `seq` in the log (0 when empty).
    pub head_seq: i64,
}

/// `GET /sync/manifest` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestResponse {
    /// 256 bucket digests keyed by the first two hex chars of `content_hash`.
    pub buckets: Vec<String>,
    /// Highest `seq` in the log (0 when empty).
    pub head_seq: i64,
}

/// `POST /sync/import` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportResponse {
    /// Per-entry results in request order.
    pub results: Vec<ImportItemResult>,
    /// Highest `seq` after the batch.
    pub head_seq: i64,
}

mod iso8601_serde {
    use super::{Iso8601, OffsetDateTime};
    use serde::Deserializer;
    use serde::Serializer;

    /// Serialize an `OffsetDateTime` as ISO-8601.
    pub fn serialize<S: Serializer>(t: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
        let formatted = t
            .format(&Iso8601::DEFAULT)
            .map_err(serde::ser::Error::custom)?;
        s.serialize_str(&formatted)
    }

    /// Parse an ISO-8601 string into an `OffsetDateTime`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        OffsetDateTime::parse(&s, &Iso8601::DEFAULT).map_err(serde::de::Error::custom)
    }
}
