//! `DocumentRevisionV1` — what one document's extracted text *is* on the
//! replication wire.
//!
//! Lives in the documents domain, beside the extraction it describes, exactly
//! as `code::replica_payload` does: `domains::sync` may read a domain's
//! payload, but a domain may not reach into sync.
//!
//! Original-file-free by construction: the payload carries extracted text and
//! a repository-relative path, and has no field that could hold an absolute
//! path or the file itself.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domains::documents::share;
use crate::prelude::*;
use crate::utilities::canonical_json;

/// Schema version of this payload shape, carried on the wire so an older
/// engine refuses what it cannot read instead of half-applying it.
pub const DOCUMENT_PAYLOAD_VERSION: i64 = 1;

/// How many hex chars a shared document id carries.
pub const ID_LEN: usize = 32;

/// The entity kind document revisions replicate under. The entity KEY is the
/// `shared_id`: one document is the entity, each revision a version of it.
pub const DOCUMENT_ENTITY_KIND: &str = "document_revision";

/// One extracted passage, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChunkWire {
    /// Position within the document, from zero.
    pub ordinal: i64,
    /// Heading breadcrumb, joined with ` > `.
    pub heading_path: String,
    /// First character offset.
    pub char_start: i64,
    /// Last character offset.
    pub char_end: i64,
    /// First line (1-based).
    pub line_start: i64,
    /// Last line (1-based, inclusive).
    pub line_end: i64,
    /// 64-bit SimHash of the passage.
    pub simhash: i64,
    /// The passage itself.
    pub text: String,
}

/// One resolvable link the document carries, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LinkWire {
    /// Which chunk the link was found in.
    pub ordinal: i64,
    /// Target as a repository-relative path.
    pub target: String,
}

/// One revision of one shared document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DocumentRevisionV1 {
    /// 32-hex id over `repo` and `path`.
    pub shared_id: String,
    /// Canonical repository.
    pub repo: String,
    /// Normalized repository-relative path.
    pub path: String,
    /// Title as the extractor found it.
    pub title: String,
    /// Which extractor produced the chunks.
    pub format: String,
    /// The sender's revision hash for this content.
    pub revision_hash: String,
    /// Every passage, ascending by ordinal.
    pub chunks: Vec<ChunkWire>,
    /// Every resolvable link.
    pub links: Vec<LinkWire>,
}

impl DocumentRevisionV1 {
    /// Canonical bytes and their 64-hex digest.
    ///
    /// # Errors
    /// Propagates a serialization failure.
    pub fn canonical(&self) -> Result<(String, String)> {
        let value: Value = serde_json::to_value(self)
            .map_err(|e| Error::Other(format!("document revision payload: {e}")))?;
        let (bytes, digest) = canonical_json::bytes_and_digest(&value)?;
        let text =
            String::from_utf8(bytes).map_err(|e| Error::Other(format!("payload utf8: {e}")))?;
        Ok((text, digest))
    }

    /// Decode a payload from canonical bytes.
    ///
    /// # Errors
    /// Returns [`Error::BadRequest`] when the bytes are not this schema — the
    /// accepting side refuses what it cannot read rather than activating half
    /// a revision.
    pub fn decode(bytes: &str) -> Result<Self> {
        serde_json::from_str(bytes)
            .map_err(|e| Error::BadRequest(format!("document revision payload v1: {e}")))
    }

    /// The id this payload's `repo` and `path` earn.
    ///
    /// Unlike a code generation — whose id digests its whole contents — a
    /// document's id names WHICH document it is, not which revision. Two
    /// revisions of one file share an id and differ by `revision_hash`, which
    /// is what lets the later one supersede the earlier instead of arriving as
    /// an unrelated entity.
    #[must_use]
    pub fn mint_id(&self) -> String {
        share::shared_id(&self.repo, &self.path)
    }

    /// Whether this payload's `repo` and `path` earn the id it claims.
    ///
    /// This is the check a code generation could not make: the identity's
    /// inputs are both in the payload, so a sender that relabelled a document
    /// — same content, someone else's repository or path — no longer owns its
    /// key and is refused before anything is written.
    #[must_use]
    pub fn owns_its_id(&self) -> bool {
        self.shared_id.len() == ID_LEN && self.mint_id() == self.shared_id
    }

    /// Whether the chunk ordinals are `0..n` with no gap and no repeat.
    ///
    /// A gap means a part of the revision is missing, and a revision is only
    /// ever visible whole; accepting one with a hole would publish a document
    /// whose middle silently vanished.
    #[must_use]
    pub fn chunks_are_contiguous(&self) -> bool {
        self.chunks
            .iter()
            .enumerate()
            .all(|(index, chunk)| i64::try_from(index) == Ok(chunk.ordinal))
    }

    /// Whether every link points at a chunk this revision actually has.
    #[must_use]
    pub fn links_resolve(&self) -> bool {
        let count = i64::try_from(self.chunks.len()).unwrap_or(i64::MAX);
        self.links
            .iter()
            .all(|link| link.ordinal >= 0 && link.ordinal < count)
    }
}

#[cfg(test)]
#[path = "tests/replica_payload.rs"]
mod tests;
