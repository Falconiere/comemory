//! Declared schema — the `replica-v1` exchange client's per-key state: what
//! each `(api_url, workspace_id)` negotiated and how its network is doing, the
//! repository policy last loaded for it, the pull positions it holds back, the
//! entities it has exchanged, and the scratch rows of a compacting replay
//! (issue 255).
//!
//! Every table is keyed by the session key, never by the workspace alone: a
//! credential pointed at another origin, or at another workspace, starts from
//! nothing rather than inheriting a cursor, a selection or an approval.

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `sync_exchange`: one row per session key — the negotiated protocol, the
/// network state that gates the next request, and the progress markers a
/// pass resumes from (upgrade horizon, replay, stall).
#[table(name = "sync_exchange")]
#[primary_key(api_url, workspace_id)]
pub struct SyncExchange {
    /// Platform API base, trailing `/` removed.
    #[column(not_null)]
    pub api_url: Text,
    /// Workspace the credential is scoped to.
    #[column(not_null)]
    pub workspace_id: Text,
    /// `legacy` or `replica-v1`; `NULL` before the first negotiation.
    #[column(check = "protocol IN ('legacy', 'replica-v1')")]
    pub protocol: Text,
    /// Why coverage is partial on a legacy selection.
    pub coverage_reason: Text,
    /// RFC3339 time the current protocol was selected.
    pub selected_at: Text,
    /// Captured upstream head when `replica-v1` was selected; memory rows
    /// made before the selection wait until the cursor reaches it.
    pub upgrade_through: Integer,
    /// `rebootstrap`, or `repair:<entity kind>`, while a replay runs.
    pub replay_kind: Text,
    /// `scanning` or `applying` while a replay runs.
    #[column(check = "replay_state IN ('scanning', 'applying')")]
    pub replay_state: Text,
    /// Highest upstream position the replay's scan has read.
    pub replay_scan_through: Integer,
    /// Upstream head the replay reads to.
    pub replay_target: Integer,
    /// What gates the next request.
    #[column(
        not_null,
        default = "'ok'",
        check = "network_state IN ('ok', 'backoff', 'auth_suspended', 'protocol_error')"
    )]
    pub network_state: Text,
    /// RFC3339 time before which no request is made while backing off.
    pub retry_at: Text,
    /// Failed passes in a row, which widens the backoff window.
    #[column(not_null, default = "0")]
    pub consecutive_failures: Integer,
    /// Last failure detail, for the operator.
    pub last_error: Text,
    /// Fingerprint of the `auth.json` a `401`/`403` suspended.
    pub suspended_fingerprint: Text,
    /// Upstream head last seen.
    pub upstream_head: Integer,
    /// Upstream position the pull stalled before, if it did.
    pub stall_sequence: Integer,
    /// Why the pull stalled.
    pub stall_reason: Text,
    /// RFC3339 time the last pass under this key ended.
    pub last_session_at: Text,
    /// RFC3339 time of the last pass that reached the network cleanly.
    pub last_ok_at: Text,
    /// RFC3339 time the row last changed.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `sync_policy_snapshot`: the repository policy last loaded for a session
/// key — what an unmanaged engine's session uses, since no policy route
/// exists in front of it.
#[table(name = "sync_policy_snapshot")]
#[primary_key(api_url, workspace_id)]
pub struct SyncPolicySnapshot {
    /// Platform API base, trailing `/` removed.
    #[column(not_null)]
    pub api_url: Text,
    /// Workspace the policy governs.
    #[column(not_null)]
    pub workspace_id: Text,
    /// Monotonic policy revision.
    #[column(not_null)]
    pub revision: Integer,
    /// Digest over the revision, allowlist and mappings.
    #[column(not_null)]
    pub fingerprint: Text,
    /// Approved canonical repositories, as a JSON array of strings.
    #[column(not_null)]
    pub allowlist_json: Text,
    /// Confirmed label mappings, as a JSON object label → canonical.
    #[column(not_null)]
    pub mappings_json: Text,
    /// RFC3339 time the policy was loaded.
    #[column(not_null)]
    pub loaded_at: Text,
}

/// `replica_pull_hold`: upstream positions a pull passed without applying,
/// and why — reconsidered when the cause is gone.
///
/// `stream_epoch` is `legacy` for a legacy key's holds, which are counted but
/// never rewound.
#[table(name = "replica_pull_hold")]
#[primary_key(api_url, workspace_id, stream_epoch, from_sequence)]
#[index("idx_replica_pull_hold_reason", api_url, workspace_id, reason)]
pub struct ReplicaPullHold {
    /// Platform API base, trailing `/` removed.
    #[column(not_null)]
    pub api_url: Text,
    /// Workspace the position belongs to.
    #[column(not_null)]
    pub workspace_id: Text,
    /// Upstream epoch the position was issued under.
    #[column(not_null)]
    pub stream_epoch: Text,
    /// First held position.
    #[column(not_null)]
    pub from_sequence: Integer,
    /// Last held position (equal to `from_sequence` for one entry).
    #[column(not_null)]
    pub to_sequence: Integer,
    /// Why the position was held.
    #[column(
        not_null,
        check = "reason IN ('policy', 'pending_local', 'server_withheld', 'secret', 'id_collision')"
    )]
    pub reason: Text,
    /// Entity kind, for an entry hold.
    pub entity_kind: Text,
    /// Entity key, for an entry hold.
    pub entity_key: Text,
    /// Repository the entry named, for a `policy` hold.
    pub repository: Text,
    /// Policy revision a `server_withheld` range was read under.
    pub policy_revision: Integer,
    /// RFC3339 time the hold was recorded.
    #[column(not_null)]
    pub recorded_at: Text,
}

/// `replica_binding`: each entity a session key has exchanged, with the
/// revision the upstream last held for it — what verification compares and
/// what stops a replay putting an older revision over a newer one.
#[table(name = "replica_binding")]
#[primary_key(api_url, workspace_id, entity_kind, entity_key)]
#[index("idx_replica_binding_entity", entity_kind, entity_key)]
pub struct ReplicaBinding {
    /// Platform API base, trailing `/` removed.
    #[column(not_null)]
    pub api_url: Text,
    /// Workspace the entity was exchanged with.
    #[column(not_null)]
    pub workspace_id: Text,
    /// Entity kind.
    #[column(not_null)]
    pub entity_kind: Text,
    /// Entity key within its kind.
    #[column(not_null)]
    pub entity_key: Text,
    /// Payload digest the upstream last held; `NULL` for a tombstone.
    pub synced_digest: Text,
    /// `1` when the upstream's last revision is a tombstone.
    #[column(not_null, default = "0")]
    pub synced_deleted: Integer,
    /// Upstream position of that revision; cleared by a rebootstrap.
    pub synced_sequence: Integer,
    /// Upstream epoch of `synced_sequence`.
    pub synced_epoch: Text,
    /// RFC3339 time the row last changed.
    #[column(not_null)]
    pub updated_at: Text,
}

/// `replica_replay`: the scratch rows of a compacting replay — each entity's
/// LAST position in the replayed range, applied once the scan ends.
///
/// Copied by a rebuild, so a replay interrupted by one resumes instead of
/// falling back to a forward pull.
#[table(name = "replica_replay")]
#[primary_key(api_url, workspace_id, entity_kind, entity_key)]
pub struct ReplicaReplay {
    /// Platform API base, trailing `/` removed.
    #[column(not_null)]
    pub api_url: Text,
    /// Workspace being replayed.
    #[column(not_null)]
    pub workspace_id: Text,
    /// Entity kind.
    #[column(not_null)]
    pub entity_kind: Text,
    /// Entity key within its kind.
    #[column(not_null)]
    pub entity_key: Text,
    /// Upstream position of the entity's last entry in the range.
    #[column(not_null)]
    pub sequence: Integer,
    /// That entry as the upstream sent it, JSON.
    #[column(not_null)]
    pub entry_json: Text,
}

#[cfg(test)]
#[path = "tests/schema_exchange.rs"]
mod tests;
