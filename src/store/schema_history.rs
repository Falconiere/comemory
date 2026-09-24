//! Declared schema — the run-history and log tables: `eval_runs` (one row
//! per `eval` / `tune` / `bandit` run), `gc_runs` (one per `gc` sweep),
//! `index_runs` (one per `index-code` run) and the append-only
//! `index_failures` log. The three history tables carry a newest-first
//! `DESC` index and the log an `AUTOINCREMENT` key, both expressible since
//! toolu-orm 0.6.0 (#70, #65).

use toolu_orm::core::column::{Integer, Real, Text};
use toolu_orm::table;

/// `eval_runs`: one row per learning-loop run, never per scored candidate.
#[table(name = "eval_runs")]
#[index("idx_eval_runs_at", desc(at))]
pub struct EvalRuns {
    /// Random hex run id.
    #[column(primary_key)]
    pub id: Text,
    /// `eval`, `tune` or `bandit`.
    #[column(not_null, check = "kind IN ('eval', 'tune', 'bandit')")]
    pub kind: Text,
    /// RFC3339 time.
    #[column(not_null)]
    pub at: Text,
    /// Golden pairs scored.
    #[column(not_null)]
    pub golden_pairs: Integer,
    /// The `k` of recall@k.
    #[column(not_null)]
    pub k: Integer,
    /// recall@k.
    #[column(not_null)]
    pub recall: Real,
    /// Mean reciprocal rank.
    #[column(not_null)]
    pub mrr: Real,
    /// JSON of the knobs scored.
    #[column(not_null)]
    pub knobs: Text,
    /// `1` when `--apply` wrote them to `config.toml`.
    #[column(not_null, default = "0")]
    pub applied: Integer,
    /// `1` when the console dismissed the proposal (v15).
    #[column(not_null, default = "0")]
    pub discarded: Integer,
}

/// `gc_runs`: one row per `comemory gc` sweep.
#[table(name = "gc_runs")]
#[index("idx_gc_runs_at", desc(at))]
pub struct GcRuns {
    /// Random hex run id.
    #[column(primary_key)]
    pub id: Text,
    /// RFC3339 time.
    #[column(not_null)]
    pub at: Text,
    /// Soft-deleted memories purged.
    #[column(not_null)]
    pub removed: Integer,
    /// `retrieval_log` rows evicted.
    #[column(not_null)]
    pub log_rows: Integer,
    /// `feedback_events` rows evicted.
    #[column(not_null)]
    pub event_rows: Integer,
    /// Bytes reclaimed.
    #[column(not_null)]
    pub bytes_freed: Integer,
    /// `activity_log` rows evicted past `prune.learning_retention_days`.
    /// Defaulted so a sweep recorded before the activity feed shipped reads
    /// back as "evicted nothing" rather than as a missing column.
    #[column(not_null, default = "0")]
    pub activity_rows: Integer,
}

/// `activity_log`: one row per instrumented command run, whatever surface ran
/// it (`utilities::activity::record`). The feed behind `GET /api/v1/activity`
/// and its SSE twin.
///
/// The `AUTOINCREMENT` key is the stream's cursor: ids are monotonic and never
/// reused, so a client that saw `id` can ask for everything above it and a
/// `gc` sweep in between cannot make an old id reappear. `desc(at)` serves the
/// newest-first snapshot.
#[table(name = "activity_log")]
#[index("idx_activity_log_at", desc(at), id)]
#[index("idx_activity_log_command_at", command, desc(at), id)]
#[unique_index("uq_activity_log_event_id", event_id)]
pub struct ActivityLog {
    /// Monotonic id, and the SSE cursor.
    #[column(primary_key, autoincrement)]
    pub id: Integer,
    /// RFC3339 UTC time the run finished (`memory_row::iso_format`).
    #[column(not_null)]
    pub at: Text,
    /// The command that ran, from the `utilities::activity::command`
    /// vocabulary (`save`, `find`, `sync.import`, …).
    #[column(not_null)]
    pub command: Text,
    /// Which delivery surface ran it.
    #[column(not_null, check = "source IN ('cli', 'http', 'mcp')")]
    pub source: Text,
    /// Caller label the caller declared: MCP `clientInfo`, HTTP `User-Agent`
    /// or `COMEMORY_ACTOR`. NULL when none was supplied — never inferred.
    pub actor: Text,
    /// Repo label the run was scoped to, NULL when unscoped.
    pub repo: Text,
    /// Wall-clock duration of the core call.
    #[column(not_null)]
    pub duration_ms: Integer,
    /// `1` when the core returned `Ok`, `0` when it returned an error.
    #[column(not_null, default = "1")]
    pub ok: Integer,
    /// `utilities::error_code::classify` slug when `ok` is `0`.
    pub error_code: Text,
    /// Bounded per-command JSON summary; NULL when `activity.summaries` is
    /// off (the row still records that the command ran).
    pub summary: Text,
    /// Device that ran the command; `NULL` means this machine (v26).
    pub device: Text,
    /// Stable replica event id (`ev-<32 hex>`), minted when the row is
    /// journalled; `NULL` for a row that never left this machine (v26).
    pub event_id: Text,
}

/// `index_failures`: swallowed indexing failures, appended in order.
#[table(name = "index_failures")]
pub struct IndexFailures {
    /// Monotonic id.
    #[column(primary_key, autoincrement)]
    pub id: Integer,
    /// RFC3339 time.
    #[column(not_null)]
    pub ts: Text,
    /// The error text.
    #[column(not_null)]
    pub error: Text,
}

/// `index_runs`: one row per `index-code` run, outcomes included.
#[table(name = "index_runs")]
#[index("idx_index_runs_started", desc(started_at), id)]
#[index("idx_index_runs_repo_started", repo, desc(started_at), id)]
pub struct IndexRuns {
    /// Random hex run id.
    #[column(primary_key)]
    pub id: Text,
    /// Repo label.
    #[column(not_null)]
    pub repo: Text,
    /// Working-tree root at index time.
    pub root_path: Text,
    /// `full` or `incremental`.
    #[column(not_null, check = "mode IN ('full', 'incremental')")]
    pub mode: Text,
    /// RFC3339 start.
    #[column(not_null)]
    pub started_at: Text,
    /// RFC3339 finish.
    #[column(not_null)]
    pub finished_at: Text,
    /// Wall-clock duration.
    #[column(not_null)]
    pub duration_ms: Integer,
    /// Files walked.
    #[column(not_null)]
    pub files_indexed: Integer,
    /// Symbols written.
    #[column(not_null)]
    pub symbols: Integer,
    /// `ok`, `error` or `cancelled`.
    #[column(not_null, check = "outcome IN ('ok', 'error', 'cancelled')")]
    pub outcome: Text,
    /// Error text when `outcome` is `error`.
    pub error: Text,
}
