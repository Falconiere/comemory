//! Declared schema — cloud sync's `sync_state` cursors and `sync_binding`
//! rows. `sync_log` stays hand-SQL until toolu-orm can express
//! `AUTOINCREMENT` (#65).

use toolu_orm::core::column::{Integer, Text};
use toolu_orm::table;

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
