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
#[index("idx_index_runs_started", desc(started_at))]
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
