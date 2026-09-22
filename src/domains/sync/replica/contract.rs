//! The `replica-v1` wire contract — the shapes both repositories implement.
//!
//! Every request and response carries the stream epoch, so a restored or
//! replaced stream is visible rather than silently agreeing, and every
//! operation carries its own id and payload digest, so a replay is a replay
//! rather than a second effect.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::replica_journal::ReplicaOp;

/// Protocol tag carried by every envelope.
pub const PROTOCOL: &str = "replica-v1";

/// Maximum operations in one import envelope.
pub const MAX_OPERATIONS: usize = 500;

/// Maximum serialized envelope size, in bytes.
///
/// The engine enforces this before parsing, through `serve`'s global body
/// limit — a request above it never reaches a handler. The constant is
/// restated here because it is part of the contract the platform implements
/// too; a colocated route test asserts the two agree so they cannot drift.
pub const MAX_ENVELOPE_BYTES: usize = 5 * 1024 * 1024;

/// A peer's position in a stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CursorRef {
    /// Epoch the position was taken under.
    pub stream_epoch: String,
    /// Highest sequence the peer has applied.
    pub sequence: i64,
}

/// One operation offered for acceptance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    /// Client-unique id; a replay repeats it.
    pub operation_id: String,
    /// Entity kind (`memory` today).
    pub entity_kind: String,
    /// Entity key within its kind.
    pub entity_key: String,
    /// What the operation does.
    pub op: ReplicaOp,
    /// Payload schema version for this kind.
    pub schema_version: i64,
    /// Digest of the canonical payload bytes; absent for a tombstone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_digest: Option<String>,
    /// Payload; absent for a tombstone and for a staged activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Sequence the peer had applied when it made this change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_sequence: Option<i64>,
    /// Canonical repository, when the entity has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

/// `POST /sync/replica/import` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    /// Must be [`PROTOCOL`].
    pub protocol: String,
    /// The peer's position, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<CursorRef>,
    /// Operations in offer order, capped at [`MAX_OPERATIONS`].
    pub operations: Vec<Operation>,
    /// Rejected when present: the workspace comes from the authenticated
    /// credential, and a body that names one is claiming authority it does
    /// not have. Declared rather than merely unknown so the refusal says why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

/// What the accepting side decided about one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Applied and journalled at the returned sequence.
    Accepted,
    /// Already answered under this operation id; the original sequence stands.
    Duplicate,
    /// Refused — the peer had not observed a newer tombstone, or a restore
    /// named the wrong deletion.
    RejectedStale,
    /// Refused — the operation id was reused with different bytes.
    RejectedConflict,
    /// Refused — unknown entity kind or payload schema version.
    RejectedUnsupported,
    /// Refused — the payload failed validation.
    RejectedInvalid,
    /// Refused — policy does not allow this entity here.
    RejectedNotAllowed,
    /// Refused — retention removed the payload bytes this operation carries.
    PayloadExpired,
    /// Refused — the payload was permanently erased; the barrier stands.
    PayloadErased,
}

impl Disposition {
    /// Wire literal, also what `replica_receipt.disposition` stores.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Duplicate => "duplicate",
            Self::RejectedStale => "rejected_stale",
            Self::RejectedConflict => "rejected_conflict",
            Self::RejectedUnsupported => "rejected_unsupported",
            Self::RejectedInvalid => "rejected_invalid",
            Self::RejectedNotAllowed => "rejected_not_allowed",
            Self::PayloadExpired => "payload_expired",
            Self::PayloadErased => "payload_erased",
        }
    }

    /// Whether the decision wrote a feed position.
    #[must_use]
    pub fn is_accepted(self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// One decision on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationResult {
    /// The operation this answers.
    pub operation_id: String,
    /// What was decided.
    pub disposition: Disposition,
    /// Assigned sequence; absent when nothing was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i64>,
    /// Digest the decision was made against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_digest: Option<String>,
    /// Why, for a refusal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `POST /sync/replica/import` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportResponse {
    /// Always [`PROTOCOL`].
    pub protocol: String,
    /// The serving stream's epoch.
    pub stream_epoch: String,
    /// Highest sequence after the batch.
    pub head_sequence: i64,
    /// Decisions in request order.
    pub results: Vec<OperationResult>,
}
