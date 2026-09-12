//! Declared schema — cloud sync's `sync_log` change feed, `sync_state`
//! cursors and `sync_binding` rows.

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

/// `sync_log`: the ordered change feed `comemory sync` pushes and pulls.
#[table(name = "sync_log")]
#[index("idx_sync_log_memory", memory_id)]
#[index("idx_sync_log_origin_seq", origin, seq)]
pub struct SyncLog {
    /// Monotonic sequence number.
    #[column(primary_key, autoincrement)]
    pub seq: Integer,
    /// `upsert`, `tombstone` or `restore`.
    #[column(not_null, check = "op IN ('upsert', 'tombstone', 'restore')")]
    pub op: Text,
    /// Memory id.
    #[column(not_null)]
    pub memory_id: Text,
    /// Content hash at that change.
    #[column(not_null)]
    pub content_hash: Text,
    /// RFC3339 time.
    #[column(not_null)]
    pub at: Text,
    /// `local` or `sync`.
    #[column(not_null, check = "origin IN ('local', 'sync')")]
    pub origin: Text,
}

/// `sync_state`: per-workspace pull / push cursors.
#[table(name = "sync_state")]
pub struct SyncState {
    /// Workspace the cursors belong to.
    #[column(primary_key)]
    pub workspace_id: Text,
    /// Platform API base the cursors were taken against.
    #[column(not_null)]
    pub api_url: Text,
    /// Highest `sync_log.seq` pulled.
    #[column(not_null, default = "0")]
    pub pulled_seq: Integer,
    /// Highest `sync_log.seq` pushed.
    #[column(not_null, default = "0")]
    pub pushed_seq: Integer,
    /// RFC3339 time of the last sync.
    pub last_sync_at: Text,
}

/// `sync_binding`: which workspace first claimed a memory, plus any
/// `--allow-secret` override recorded for it.
#[table(name = "sync_binding")]
pub struct SyncBinding {
    /// Memory id.
    #[column(primary_key)]
    pub memory_id: Text,
    /// Workspace that holds it.
    #[column(not_null)]
    pub workspace_id: Text,
    /// Redaction rule the user overrode, if any.
    pub secret_override_rule: Text,
    /// RFC3339 time of that override.
    pub secret_override_at: Text,
}
