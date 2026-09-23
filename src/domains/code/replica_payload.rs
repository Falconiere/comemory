//! `CodeGenerationV1` — what one repo's code index *is* on the replication
//! wire.
//!
//! Lives in the code domain, beside the index it describes, exactly as
//! `memories::replica_payload` does: `domains::sync` may read a domain's
//! payload, but a domain may not reach into sync.
//!
//! Snippet-free by construction: the payload has no field that could carry
//! source text, so the no-source rule holds even if a future writer forgets
//! it. The digest of its canonical bytes is both the feed position's name and
//! the generation's own id.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::prelude::*;
use crate::store::remote_code::{Edge, File, Projection, Symbol};
use crate::utilities::canonical_json;

/// Schema version of this payload shape, carried on the wire so an older
/// engine refuses what it cannot read instead of half-applying it.
pub const CODE_PAYLOAD_VERSION: i64 = 1;

/// How many hex chars a generation id carries.
pub const ID_LEN: usize = 32;

/// The entity kind code generations replicate under. The entity KEY is the
/// canonical repo label: the repo is the entity, each generation a revision.
pub const CODE_ENTITY_KIND: &str = "code_generation";

/// One file of the manifest, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileWire {
    /// Path relative to the repo root.
    pub path: String,
    /// Git blob OID the sender indexed it at.
    pub blob_oid: String,
}

/// One symbol, on the wire. No snippet, by construction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SymbolWire {
    /// Path relative to the repo root.
    pub path: String,
    /// Qualified symbol name.
    pub symbol: String,
    /// `function` / `struct` / ….
    pub kind: String,
    /// `rust` / `typescript` / ….
    pub lang: String,
    /// First line (1-based).
    pub line_start: i64,
    /// Last line (1-based, inclusive).
    pub line_end: i64,
}

/// One graph edge, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EdgeWire {
    /// `imports` or `co_changed`.
    pub rel: String,
    /// Source file path.
    pub src_path: String,
    /// Target file path.
    pub dst_path: String,
    /// Co-change count, or `1` for a resolved import.
    pub weight: i64,
    /// The revision the edge was derived at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

/// One generation's replicated state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CodeGenerationV1 {
    /// 32-hex id, the prefix of this payload's own digest.
    pub generation_id: String,
    /// The generation this was planned against; absent for a repo's first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// HEAD commit the index was built at.
    pub head: String,
    /// Co-change cursor at build time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mined_commit: Option<String>,
    /// The manifest, ascending by path.
    pub files: Vec<FileWire>,
    /// Every symbol, snippet-free.
    pub symbols: Vec<SymbolWire>,
    /// Every import and co-change edge.
    pub edges: Vec<EdgeWire>,
}

impl CodeGenerationV1 {
    /// Build a wire payload from a planned generation's projection.
    #[must_use]
    pub fn new(
        generation_id: &str,
        parent_id: Option<&str>,
        head: &str,
        mined_commit: Option<&str>,
        projection: &Projection,
    ) -> Self {
        Self {
            generation_id: generation_id.to_string(),
            parent_id: parent_id.map(str::to_string),
            head: head.to_string(),
            mined_commit: mined_commit.map(str::to_string),
            files: projection
                .files
                .iter()
                .map(|f| FileWire {
                    path: f.path.clone(),
                    blob_oid: f.blob_oid.clone(),
                })
                .collect(),
            symbols: projection
                .symbols
                .iter()
                .map(|s| SymbolWire {
                    path: s.path.clone(),
                    symbol: s.symbol.clone(),
                    kind: s.kind.clone(),
                    lang: s.lang.clone(),
                    line_start: s.line_start,
                    line_end: s.line_end,
                })
                .collect(),
            edges: projection
                .edges
                .iter()
                .map(|e| EdgeWire {
                    rel: e.rel.clone(),
                    src_path: e.src_path.clone(),
                    dst_path: e.dst_path.clone(),
                    weight: e.weight,
                    anchor: e.anchor.clone(),
                })
                .collect(),
        }
    }

    /// The projection this payload describes, ready for `remote_code`.
    #[must_use]
    pub fn projection(&self) -> Projection {
        Projection {
            files: self
                .files
                .iter()
                .map(|f| File {
                    path: f.path.clone(),
                    blob_oid: f.blob_oid.clone(),
                })
                .collect(),
            symbols: self
                .symbols
                .iter()
                .map(|s| Symbol {
                    path: s.path.clone(),
                    symbol: s.symbol.clone(),
                    kind: s.kind.clone(),
                    lang: s.lang.clone(),
                    line_start: s.line_start,
                    line_end: s.line_end,
                })
                .collect(),
            edges: self
                .edges
                .iter()
                .map(|e| Edge {
                    rel: e.rel.clone(),
                    src_path: e.src_path.clone(),
                    dst_path: e.dst_path.clone(),
                    weight: e.weight,
                    anchor: e.anchor.clone(),
                })
                .collect(),
        }
    }

    /// Canonical bytes and their 64-hex digest.
    ///
    /// # Errors
    /// Propagates a serialization failure.
    pub fn canonical(&self) -> Result<(String, String)> {
        let value: Value = serde_json::to_value(self)
            .map_err(|e| Error::Other(format!("code generation payload: {e}")))?;
        let (bytes, digest) = canonical_json::bytes_and_digest(&value)?;
        let text =
            String::from_utf8(bytes).map_err(|e| Error::Other(format!("payload utf8: {e}")))?;
        Ok((text, digest))
    }

    /// Decode a payload from canonical bytes.
    ///
    /// # Errors
    /// Returns [`Error::BadRequest`] when the bytes are not this schema — the
    /// accepting side refuses what it cannot read rather than activating a
    /// partial generation.
    pub fn decode(bytes: &str) -> Result<Self> {
        serde_json::from_str(bytes)
            .map_err(|e| Error::BadRequest(format!("code generation payload v1: {e}")))
    }

    /// The id this payload's contents earn: the 32-hex prefix of its digest
    /// taken with `generation_id` blank.
    ///
    /// Blanking the field first is what makes the id checkable — a digest
    /// that included the id could never be recomputed from the payload.
    ///
    /// # Errors
    /// Propagates the canonical serialization.
    pub fn mint_id(&self) -> Result<String> {
        let (_, digest) = self.with_id("").canonical()?;
        Ok(digest.get(..ID_LEN).unwrap_or(digest.as_str()).to_string())
    }

    /// The same payload carrying `generation_id`.
    #[must_use]
    pub fn with_id(&self, generation_id: &str) -> Self {
        Self {
            generation_id: generation_id.to_string(),
            ..self.clone()
        }
    }

    /// Whether this payload's contents earn the id it claims.
    ///
    /// The id is content-derived, so a payload whose contents were edited no
    /// longer owns it — the same identity rule memories apply to a body that
    /// no longer hashes to its id.
    ///
    /// # Errors
    /// Propagates the canonical serialization.
    pub fn owns_its_id(&self) -> Result<bool> {
        Ok(self.generation_id.len() == ID_LEN && self.mint_id()? == self.generation_id)
    }
}

#[cfg(test)]
#[path = "tests/replica_payload.rs"]
mod tests;
