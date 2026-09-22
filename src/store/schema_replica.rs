//! Declared schema — the `replica-v1` journal: this database's stream epoch,
//! immutable payload bytes, the ordered acceptance feed, per-entity revisions
//! and tombstones, the durable outgoing operation outbox, acceptance receipts,
//! per-workspace upstream cursors, and staged parts for oversized revisions.
//!
//! Entity-agnostic on purpose. `sync_log` ([`super::schema_sync`]) stays the
//! memory-only legacy feed both wire shapes keep writing; these tables are the
//! one journal every replicated entity kind joins (issues 251–255).

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `replica_stream`: the single-row identity of this database's change
/// stream. The `epoch` changes only when server state is restored or
/// replaced, which is how a peer tells "caught up" from "your stream is
/// gone" instead of reading an empty page as agreement.
#[table(name = "replica_stream")]
pub struct ReplicaStream {
    /// Always `1` — one stream per database.
    #[column(primary_key, check = "id = 1")]
    pub id: Integer,
    /// 32 lowercase hex chars minted at migration time.
    #[column(not_null)]
    pub epoch: Text,
    /// RFC3339 time the epoch was minted.
    #[column(not_null)]
    pub created_at: Text,
}

/// `replica_payload`: canonical payload bytes keyed by their SHA-256 digest.
///
/// Immutable by contract: a feed row references a digest and never live
/// state, so history describes the bytes that were accepted at that
/// sequence. Permanent erasure blanks `bytes` and stamps `redacted_at`; the
/// row itself stays as the barrier that stops an old upsert from
/// resurrecting erased content.
#[table(name = "replica_payload")]
pub struct ReplicaPayload {
    /// 64-hex SHA-256 of the canonical payload bytes.
    #[column(primary_key)]
    pub digest: Text,
    /// Entity kind the payload describes (`memory` today).
    #[column(not_null)]
    pub entity_kind: Text,
    /// Payload schema version within that kind.
    #[column(not_null)]
    pub schema_version: Integer,
    /// Canonical JSON bytes; `NULL` once redacted.
    pub bytes: Text,
    /// Byte length of the original payload, retained after redaction.
    #[column(not_null)]
    pub byte_len: Integer,
    /// RFC3339 time the payload was first stored.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 time the bytes were erased, if they were.
    pub redacted_at: Text,
}

/// `replica_feed`: the ordered acceptance record. One row per accepted
/// mutation, local or imported, written in the same transaction as the
/// materialized state.
///
/// `sequence` is assigned here and never carried from a client: server
/// acceptance, not a device clock, is what orders replicated edits.
#[table(name = "replica_feed")]
#[index("idx_replica_feed_entity", entity_kind, entity_key)]
#[unique_index("uq_replica_feed_operation", operation_id)]
pub struct ReplicaFeed {
    /// Server-assigned monotonic acceptance position.
    #[column(primary_key, autoincrement)]
    pub sequence: Integer,
    /// Stream epoch this position belongs to.
    #[column(not_null)]
    pub epoch: Text,
    /// Entity kind (`memory` today).
    #[column(not_null)]
    pub entity_kind: Text,
    /// Entity key within its kind (the 8-hex memory id for memories).
    #[column(not_null)]
    pub entity_key: Text,
    /// `upsert`, `tombstone` or `restore`.
    #[column(not_null, check = "op IN ('upsert', 'tombstone', 'restore')")]
    pub op: Text,
    /// Payload digest; `NULL` for a tombstone.
    pub payload_digest: Text,
    /// Payload schema version at acceptance.
    #[column(not_null)]
    pub schema_version: Integer,
    /// The operation whose acceptance this row is.
    #[column(not_null)]
    pub operation_id: Text,
    /// `local` for a mutation made here, `sync` for an imported one.
    #[column(not_null, check = "origin IN ('local', 'sync')")]
    pub origin: Text,
    /// Canonical repository the entity belongs to, when it has one.
    pub repository: Text,
    /// RFC3339 provenance time from the mutation.
    #[column(not_null)]
    pub at: Text,
}

/// `replica_revision`: the current state of one entity — its latest accepted
/// sequence, its payload digest, and whether it is tombstoned.
///
/// The deletion's own sequence is kept so a restore must name the deletion it
/// observed; an ordinary stale upsert cannot revive a tombstone.
#[table(name = "replica_revision")]
#[primary_key(entity_kind, entity_key)]
pub struct ReplicaRevision {
    /// Entity kind.
    #[column(not_null)]
    pub entity_kind: Text,
    /// Entity key within its kind.
    #[column(not_null)]
    pub entity_key: Text,
    /// Feed position of the newest accepted operation for this entity.
    #[column(not_null)]
    pub sequence: Integer,
    /// Digest of the newest accepted payload; `NULL` while tombstoned.
    pub payload_digest: Text,
    /// `1` while tombstoned.
    #[column(not_null, default = "0")]
    pub deleted: Integer,
    /// Feed position of the tombstone a restore must reference.
    pub deleted_sequence: Integer,
    /// RFC3339 time of the newest accepted operation.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `replica_operation`: the durable outbox. One row per mutation made here,
/// committed in the same transaction as the mirror write, so a crash after a
/// successful save can never lose the fact that the save owes an upload.
#[table(name = "replica_operation")]
#[index("idx_replica_operation_state", state, created_at)]
pub struct ReplicaOperation {
    /// Client-unique operation id (`op-<yyyymmdd>-<8hex>`).
    #[column(primary_key)]
    pub operation_id: Text,
    /// Entity kind.
    #[column(not_null)]
    pub entity_kind: Text,
    /// Entity key within its kind.
    #[column(not_null)]
    pub entity_key: Text,
    /// `upsert`, `tombstone` or `restore`.
    #[column(not_null, check = "op IN ('upsert', 'tombstone', 'restore')")]
    pub op: Text,
    /// Payload digest; `NULL` for a tombstone.
    pub payload_digest: Text,
    /// Payload schema version.
    #[column(not_null)]
    pub schema_version: Integer,
    /// Canonical repository, when the entity has one.
    pub repository: Text,
    /// Upstream sequence this mutation was made against.
    pub observed_sequence: Integer,
    /// `pending`, `accepted` or `rejected`.
    #[column(
        not_null,
        check = "state IN ('pending', 'accepted', 'rejected')",
        default = "'pending'"
    )]
    pub state: Text,
    /// Sequence the upstream assigned once accepted.
    pub upstream_sequence: Integer,
    /// Upstream disposition once answered.
    pub disposition: Text,
    /// Upload attempts made so far.
    #[column(not_null, default = "0")]
    pub attempts: Integer,
    /// Last transport or rejection detail, for operator diagnosis.
    pub last_error: Text,
    /// RFC3339 time the operation was enqueued.
    #[column(not_null)]
    pub created_at: Text,
    /// RFC3339 time the row last changed.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `replica_receipt`: what this engine answered for an operation it accepted
/// or refused, written in the accept transaction.
///
/// A replay reads its receipt back verbatim, so a lost acknowledgement costs
/// a round trip rather than a second effect or a newer position.
#[table(name = "replica_receipt")]
pub struct ReplicaReceipt {
    /// The operation this receipt answers.
    #[column(primary_key)]
    pub operation_id: Text,
    /// Stream epoch the decision was made under.
    #[column(not_null)]
    pub epoch: Text,
    /// Assigned feed position; `NULL` when nothing was written.
    pub sequence: Integer,
    /// Wire disposition (`accepted`, `rejected_stale`, …).
    #[column(not_null)]
    pub disposition: Text,
    /// Payload digest the decision was made against.
    pub payload_digest: Text,
    /// Human-readable detail for a refusal.
    pub reason: Text,
    /// RFC3339 time of the decision.
    #[column(not_null)]
    pub accepted_at: Text,
}

/// `replica_cursor`: how far this machine has applied one workspace's
/// upstream feed, and under which epoch.
///
/// The workspace comes from the authenticated credential, never from a
/// request body.
#[table(name = "replica_cursor")]
pub struct ReplicaCursor {
    /// Workspace the cursor belongs to.
    #[column(primary_key)]
    pub workspace_id: Text,
    /// Platform API base the cursor was taken against.
    #[column(not_null)]
    pub api_url: Text,
    /// Upstream stream epoch the cursor is valid under.
    #[column(not_null)]
    pub stream_epoch: Text,
    /// Highest upstream sequence applied here.
    #[column(not_null, default = "0")]
    pub applied_sequence: Integer,
    /// RFC3339 time of the last advance.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `replica_staged_part`: parts of a revision too large for one envelope.
///
/// Staged parts are invisible to `changes` and `manifest` until activation
/// verifies the assembled digest, so an interrupted upload publishes nothing.
#[table(name = "replica_staged_part")]
#[primary_key(staging_id, part_index)]
#[index("idx_replica_staged_part_created", staging_id, created_at)]
pub struct ReplicaStagedPart {
    /// Upload this part belongs to.
    #[column(not_null)]
    pub staging_id: Text,
    /// Zero-based part position.
    #[column(not_null)]
    pub part_index: Integer,
    /// How many parts the upload declares.
    #[column(not_null)]
    pub part_count: Integer,
    /// This part's bytes.
    #[column(not_null)]
    pub bytes: Text,
    /// RFC3339 time the part arrived.
    #[column(not_null)]
    pub created_at: Text,
}
