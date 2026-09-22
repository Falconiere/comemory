//! `MemoryPayloadV1` — what a memory *is* on the replication wire.
//!
//! The payload carries the replicated metadata and the body, flat, with no
//! delivery framing: the replication contract wraps it, and the digest of its
//! canonical bytes is what a feed position names. `author` is excluded (the
//! accepting side stamps it) and so is the embedding — a re-embed must not
//! look like a new revision of the memory.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::domains::memories::frontmatter::{Kind, References, Relations};
use crate::domains::memories::{Frontmatter, MemoryRecord};
use crate::prelude::*;
use crate::utilities::canonical_json;

/// Schema version of this payload shape, carried on the wire so an older
/// engine refuses what it cannot read instead of half-applying it.
pub const MEMORY_PAYLOAD_VERSION: i64 = 1;

/// The entity kind memories replicate under.
pub const MEMORY_ENTITY_KIND: &str = "memory";

/// One memory's replicated state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryPayloadV1 {
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
    pub created: String,
    /// Quality rating `1..=5`.
    pub quality: u8,
    /// Frontmatter schema version.
    pub schema: u32,
    /// 64-hex body digest.
    pub content_hash: String,
    /// Symbol/file references.
    #[serde(default)]
    pub references: References,
    /// Cross-memory relations.
    #[serde(default)]
    pub relations: Relations,
    /// Markdown body.
    pub body: String,
}

impl MemoryPayloadV1 {
    /// Build a payload from frontmatter and body.
    ///
    /// # Errors
    /// Propagates an ISO-8601 formatting failure for `created`.
    pub fn new(fm: &Frontmatter, body: &str) -> Result<Self> {
        Ok(Self {
            id: fm.id.clone(),
            kind: fm.kind,
            repo: fm.repo.clone(),
            tags: fm.tags.clone(),
            created: fm
                .created
                .format(&Iso8601::DEFAULT)
                .map_err(|e| Error::Other(format!("created timestamp: {e}")))?,
            quality: fm.quality,
            schema: fm.schema,
            content_hash: fm.content_hash.clone(),
            references: fm.references.clone(),
            relations: fm.relations.clone(),
            body: body.to_string(),
        })
    }

    /// Build a payload from a loaded record.
    ///
    /// # Errors
    /// Propagates [`MemoryPayloadV1::new`].
    pub fn from_record(rec: &MemoryRecord) -> Result<Self> {
        Self::new(&rec.frontmatter, &rec.body)
    }

    /// Canonical bytes and their 64-hex digest.
    ///
    /// # Errors
    /// Propagates a serialization failure.
    pub fn canonical(&self) -> Result<(String, String)> {
        let value: Value =
            serde_json::to_value(self).map_err(|e| Error::Other(format!("memory payload: {e}")))?;
        let (bytes, digest) = canonical_json::bytes_and_digest(&value)?;
        let text =
            String::from_utf8(bytes).map_err(|e| Error::Other(format!("payload utf8: {e}")))?;
        Ok((text, digest))
    }

    /// Decode a payload from canonical bytes.
    ///
    /// # Errors
    /// Returns [`Error::BadRequest`] when the bytes are not this schema — the
    /// accepting side must refuse a payload it cannot read rather than
    /// materialize a partial memory.
    pub fn decode(bytes: &str) -> Result<Self> {
        serde_json::from_str(bytes)
            .map_err(|e| Error::BadRequest(format!("memory payload v1: {e}")))
    }

    /// The `created` timestamp as a parsed value.
    ///
    /// # Errors
    /// Returns [`Error::BadRequest`] for a timestamp that is not ISO-8601.
    pub fn created_at(&self) -> Result<OffsetDateTime> {
        OffsetDateTime::parse(&self.created, &Iso8601::DEFAULT)
            .map_err(|e| Error::BadRequest(format!("created timestamp: {e}")))
    }
}

#[cfg(test)]
#[path = "tests/replica_payload.rs"]
mod tests;
