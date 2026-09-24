//! `ActivityEventV1` — one shared command run on the `replica-v1` wire, and
//! the allowlist of what its summary may carry (#254).
//!
//! The allowlist is explicit and closed: a key a future core adds to its local
//! summary stays local until it is listed here. The receiver checks the same
//! list, so a peer cannot smuggle anything else in.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domains::learning::replica_payload::{EVENT_ID_PREFIX, is_minted_id};
use crate::prelude::*;
use crate::store::activity::ActivityRow;
use crate::utilities::canonical_json;
use crate::utilities::shared_text::{self, Shared};
use crate::utilities::telemetry::entity;

/// `replica_feed.entity_kind` of a shared command run.
pub const ACTIVITY_ENTITY_KIND: &str = entity::ACTIVITY_EVENT;

/// Payload schema version this build writes and reads.
pub const ACTIVITY_PAYLOAD_VERSION: i64 = 1;

/// Marker a shared summary carries when its query was withheld.
pub const QUERY_WITHHELD: &str = "query_withheld";

/// The summary keys a shared run of `command` may carry, or `None` when the
/// command is never shared — `sync.import` above all, whose row is
/// replication bookkeeping and would echo between machines forever.
#[must_use]
pub fn shared_keys(command: &str) -> Option<&'static [&'static str]> {
    Some(match command {
        "save" => &["id", "kind", "tags", "supersedes"],
        "update" => &["id", "fields"],
        "delete" | "restore" => &["id"],
        "search" | "context" => &["query", "hits"],
        "find" => &["query", "hits", "total"],
        "search-code" => &["query", "hits", "lang"],
        "feedback" => &[
            "used",
            "irrelevant",
            "used_code",
            "irrelevant_code",
            "provenance",
        ],
        "index-code" => &["files", "mode"],
        _ => return None,
    })
}

/// One command run as a peer receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityEventV1 {
    /// Stable replica event id — the entity key.
    pub event_id: String,
    /// Device that ran the command.
    pub device: String,
    /// ISO-8601 time the run finished.
    pub at: String,
    /// The command, from [`shared_keys`]'s list.
    pub command: String,
    /// `cli`, `http` or `mcp`.
    pub source: String,
    /// Declared caller label, after the shared-text policy.
    #[serde(default)]
    pub actor: Option<String>,
    /// Canonical repository the run was scoped to.
    pub repo: String,
    /// Wall-clock duration.
    pub duration_ms: i64,
    /// Whether the run succeeded.
    pub ok: bool,
    /// Error slug for a failed run.
    #[serde(default)]
    pub error_code: Option<String>,
    /// The allowlisted summary; `None` when the run stored none.
    #[serde(default)]
    pub summary: Option<Value>,
}

impl ActivityEventV1 {
    /// The event a local run shares under `canonical`, or `None` when its
    /// command is never shared.
    #[must_use]
    pub fn from_row(
        row: &ActivityRow,
        canonical: &str,
        device: &str,
        event_id: &str,
    ) -> Option<Self> {
        let keys = shared_keys(&row.command)?;
        Some(Self {
            event_id: event_id.to_string(),
            device: device.to_string(),
            at: row.at.clone(),
            command: row.command.clone(),
            source: row.source.clone(),
            actor: row.actor.as_deref().and_then(shared_text::label_for_share),
            repo: canonical.to_string(),
            duration_ms: row.duration_ms,
            ok: row.ok,
            error_code: row.error_code.clone(),
            summary: row
                .summary
                .as_deref()
                .map(|text| shared_summary(text, keys)),
        })
    }

    /// Canonical bytes and their SHA-256 digest.
    ///
    /// # Errors
    /// Propagates serialization failures.
    pub fn canonical(&self) -> Result<(String, String)> {
        let value = serde_json::to_value(self)
            .map_err(|e| Error::Other(format!("activity event payload: {e}")))?;
        let (bytes, digest) = canonical_json::bytes_and_digest(&value)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| Error::Other(format!("activity event payload: {e}")))?;
        Ok((text, digest))
    }

    /// Whether the payload holds together under `entity_key`, and carries
    /// nothing the allowlist does not name.
    #[must_use]
    pub fn holds_for(&self, entity_key: &str) -> bool {
        let Some(keys) = shared_keys(&self.command) else {
            return false;
        };
        entity_key == self.event_id
            && is_minted_id(&self.event_id, EVENT_ID_PREFIX)
            && is_minted_id(&self.device, "")
            && matches!(self.source.as_str(), "cli" | "http" | "mcp")
            && !self.repo.is_empty()
            && self.duration_ms >= 0
            && time::OffsetDateTime::parse(
                &self.at,
                &time::format_description::well_known::Iso8601::DEFAULT,
            )
            .is_ok()
            && match &self.summary {
                None => true,
                Some(Value::Object(fields)) => fields
                    .keys()
                    .all(|k| keys.contains(&k.as_str()) || k == QUERY_WITHHELD),
                Some(_) => false,
            }
    }
}

/// The allowlisted part of a stored summary. The query is the one free-text
/// key, and it goes through the shared-text policy; a summary that is not a
/// JSON object shares as an empty one rather than as its text.
fn shared_summary(text: &str, keys: &[&str]) -> Value {
    let Ok(Value::Object(stored)) = serde_json::from_str::<Value>(text) else {
        return Value::Object(Map::new());
    };
    let mut shared = Map::new();
    for key in keys {
        let Some(value) = stored.get(*key) else {
            continue;
        };
        if *key == "query" {
            match value.as_str().map(shared_text::for_share) {
                Some(Shared::Kept(query)) => {
                    shared.insert("query".to_string(), Value::String(query));
                }
                _ => {
                    shared.insert(QUERY_WITHHELD.to_string(), Value::Bool(true));
                }
            }
        } else {
            shared.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(shared)
}

#[cfg(test)]
#[path = "tests/activity_payload.rs"]
mod tests;
