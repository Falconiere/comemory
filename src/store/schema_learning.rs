//! Declared schema — the learning loop's `feedback` counters, the
//! `feedback_events` provenance log, the `retrieval_log` query log, and the
//! `bandit_arms` posteriors, plus the per-symbol `code_feedback` counters
//! and the mined `query_expansions`. The run-history tables live in
//! `schema_history.rs`.

use toolu_orm::core::column::{Integer, Real, Text};
use toolu_orm::table;

/// `feedback`: aggregated per-memory used / irrelevant counters.
#[table(name = "feedback")]
pub struct Feedback {
    /// Memory id.
    #[column(primary_key)]
    pub memory_id: Text,
    /// Times marked used.
    #[column(not_null, default = "0")]
    pub used_count: Integer,
    /// Times marked irrelevant.
    #[column(not_null, default = "0")]
    pub irrelevant_count: Integer,
    /// RFC3339 time of the last `used` verdict.
    pub last_used: Text,
}

/// `code_feedback`: aggregated per-symbol used / irrelevant counters.
#[table(name = "code_feedback")]
#[primary_key(repo, path, symbol)]
pub struct CodeFeedback {
    /// Repo label.
    #[column(not_null)]
    pub repo: Text,
    /// Path relative to the repo root.
    #[column(not_null)]
    pub path: Text,
    /// Qualified symbol name.
    #[column(not_null)]
    pub symbol: Text,
    /// Times marked used.
    #[column(not_null, default = "0")]
    pub used_count: Integer,
    /// Times marked irrelevant.
    #[column(not_null, default = "0")]
    pub irrelevant_count: Integer,
    /// RFC3339 time of the last `used` verdict.
    pub last_used: Text,
}

/// `query_expansions`: mined `term → expansion` pairs behind the tier-4
/// lexical ladder.
#[table(name = "query_expansions")]
#[primary_key(term, expansion)]
pub struct QueryExpansions {
    /// Query term.
    #[column(not_null)]
    pub term: Text,
    /// Learned expansion.
    #[column(not_null)]
    pub expansion: Text,
    /// Reformulations supporting it.
    #[column(not_null, default = "1")]
    pub support: Integer,
    /// RFC3339 time of the last `mine --apply`.
    #[column(not_null)]
    pub last_mined: Text,
}

/// `feedback_events`: one row per feedback verdict, memory- or code-tagged,
/// with the provenance the auto-reinforcement pass stamps (v8).
#[table(name = "feedback_events")]
#[index("idx_feedback_events_query", query_id)]
pub struct FeedbackEvents {
    /// Rowid alias.
    #[column(primary_key)]
    pub id: Integer,
    /// The `retrieval_log` query this verdict answers.
    #[column(not_null)]
    pub query_id: Text,
    /// The memory (or, for code, the symbol identity) judged.
    #[column(not_null)]
    pub memory_id: Text,
    /// `used` or `irrelevant`.
    #[column(not_null, check = "verdict IN ('used','irrelevant')")]
    pub verdict: Text,
    /// RFC3339 time.
    #[column(not_null)]
    pub at: Text,
    /// `memory` or `code` (v6).
    #[column(not_null, default = "'memory'")]
    pub target_kind: Text,
    /// `manual` / `auto_search_edit` / … (v8).
    #[column(not_null, default = "'manual'")]
    pub provenance: Text,
}

/// `retrieval_log`: one row per `search` / `context` run, keyed by the
/// query id feedback later cites.
#[table(name = "retrieval_log")]
pub struct RetrievalLog {
    /// Query id (`stats::feedback::generate_query_id`).
    #[column(primary_key)]
    pub query_id: Text,
    /// The query text.
    #[column(not_null)]
    pub query: Text,
    /// JSON array of returned ids.
    #[column(not_null)]
    pub returned_ids: Text,
    /// RFC3339 time.
    #[column(not_null)]
    pub at: Text,
    /// Wall-clock duration (v5).
    pub duration_ms: Integer,
    /// `--repo` filter, if any (v6).
    pub repo: Text,
    /// `--kind` filter, if any (v6).
    pub kind: Text,
    /// Which surface logged it (v6).
    #[column(not_null, default = "'search'")]
    pub source: Text,
}

/// `bandit_arms`: one Beta posterior per `[tune]` grid point.
#[table(name = "bandit_arms")]
pub struct BanditArms {
    /// Hash of the knob tuple.
    #[column(primary_key)]
    pub arm_id: Text,
    /// `retrieval.rrf_k` of this arm.
    #[column(not_null)]
    pub rrf_k: Real,
    /// `rank.decay` of this arm.
    #[column(not_null)]
    pub decay: Real,
    /// `rank.mmr_lambda` of this arm.
    #[column(not_null)]
    pub mmr_lambda: Real,
    /// BM25 body weight of this arm.
    #[column(not_null)]
    pub bm25_body: Real,
    /// BM25 tags weight of this arm.
    #[column(not_null)]
    pub bm25_tags: Real,
    /// Beta α.
    #[column(not_null, default = "1.0")]
    pub alpha: Real,
    /// Beta β.
    #[column(not_null, default = "1.0")]
    pub beta: Real,
    /// Times pulled.
    #[column(not_null, default = "0")]
    pub pulls: Integer,
    /// MRR of the last pull.
    pub last_mrr: Real,
    /// RFC3339 last update.
    #[column(not_null)]
    pub updated_at: Text,
}
