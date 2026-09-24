//! `FeedbackEventV1` — one shared verdict on the `replica-v1` wire (#254).
//!
//! Lives in `learning` because the verdict writer journals it at record time,
//! inside the transaction that also bumps the counter; `domains::sync` decodes
//! it on acceptance through the `sync -> learning` dependency.
//!
//! Also the one home of the event-id shape both shared event kinds use.

use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::utilities::canonical_json;
use crate::utilities::telemetry::{PROV_AUTO_SEARCH_EDIT, PROV_IMPLICIT, PROV_MANUAL, entity};

/// `replica_feed.entity_kind` of a shared verdict.
pub const FEEDBACK_ENTITY_KIND: &str = entity::FEEDBACK_EVENT;

/// Payload schema version this build writes and reads.
pub const FEEDBACK_PAYLOAD_VERSION: i64 = 1;

/// Provenances a shared verdict may carry. `auto_coactivation` is absent on
/// purpose: every machine that indexes a repository mines the same commits
/// and mints that reward itself, so sharing it would count one commit once
/// per machine.
pub const SHAREABLE_PROVENANCE: [&str; 3] = [PROV_MANUAL, PROV_IMPLICIT, PROV_AUTO_SEARCH_EDIT];

/// What a verdict judged, named so every machine resolves it the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    /// A memory, by its content-derived id.
    Memory {
        /// 8-hex memory id.
        id: String,
    },
    /// A code symbol, by its canonical repository and repo-relative name.
    Code {
        /// Canonical repository (`owner/name`).
        repo: String,
        /// Repository-relative path.
        path: String,
        /// Symbol name — the parent's, for a cAST chunk.
        symbol: String,
        /// The file's blob OID when the verdict was recorded, when known.
        #[serde(default)]
        version: Option<String>,
    },
}

/// One verdict as a peer receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackEventV1 {
    /// Stable replica event id — the entity key.
    pub event_id: String,
    /// Device that recorded the verdict.
    pub device: String,
    /// ISO-8601 time the verdict was recorded.
    pub at: String,
    /// `used` or `irrelevant`.
    pub verdict: String,
    /// One of [`SHAREABLE_PROVENANCE`].
    pub provenance: String,
    /// `cli`, `http` or `mcp`, when known.
    #[serde(default)]
    pub surface: Option<String>,
    /// Declared caller label, after the shared-text policy.
    #[serde(default)]
    pub actor: Option<String>,
    /// `<device>:<query id>` — never equal to a local query id.
    pub origin_query_id: String,
    /// What was judged.
    pub target: Target,
}

impl FeedbackEventV1 {
    /// Canonical bytes and their SHA-256 digest.
    ///
    /// # Errors
    /// Propagates serialization failures.
    pub fn canonical(&self) -> Result<(String, String)> {
        let value = serde_json::to_value(self)
            .map_err(|e| Error::Other(format!("feedback event payload: {e}")))?;
        let (bytes, digest) = canonical_json::bytes_and_digest(&value)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| Error::Other(format!("feedback event payload: {e}")))?;
        Ok((text, digest))
    }

    /// Whether the payload holds together under `entity_key`: its own id, a
    /// device, a namespaced query id, known vocabulary, and a nameable target.
    /// A memory target's id shape is the memories capability's rule, checked
    /// by acceptance (`domains::sync::replica::validate_events`) rather than
    /// here, where `learning` may not reach `memories`.
    #[must_use]
    pub fn holds_for(&self, entity_key: &str) -> bool {
        entity_key == self.event_id
            && is_minted_id(&self.event_id, EVENT_ID_PREFIX)
            && is_minted_id(&self.device, "")
            && matches!(self.verdict.as_str(), "used" | "irrelevant")
            && SHAREABLE_PROVENANCE.contains(&self.provenance.as_str())
            && self
                .surface
                .as_deref()
                .is_none_or(|s| matches!(s, "cli" | "http" | "mcp"))
            && time::OffsetDateTime::parse(
                &self.at,
                &time::format_description::well_known::Iso8601::DEFAULT,
            )
            .is_ok()
            && self
                .origin_query_id
                .strip_prefix(self.device.as_str())
                .and_then(|rest| rest.strip_prefix(':'))
                .is_some_and(|query| !query.is_empty())
            && match &self.target {
                Target::Memory { id } => !id.is_empty(),
                Target::Code {
                    repo, path, symbol, ..
                } => !repo.is_empty() && !path.is_empty() && !symbol.is_empty(),
            }
    }
}

/// Mint a fresh `ev-<32 hex>` event id.
///
/// # Errors
/// Propagates a failure to read the system's random source.
pub fn mint_event_id() -> Result<String> {
    Ok(format!(
        "{EVENT_ID_PREFIX}{}",
        crate::store::random_id::random_hex(16)?
    ))
}

/// Prefix of every replica event id.
pub const EVENT_ID_PREFIX: &str = "ev-";

/// Whether `value` is `prefix` followed by 32 lowercase hex characters: an
/// event id under [`EVENT_ID_PREFIX`], a device id under `""`.
#[must_use]
pub fn is_minted_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|hex| {
        hex.len() == 32
            && hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    })
}

#[cfg(test)]
#[path = "tests/replica_payload.rs"]
mod tests;
