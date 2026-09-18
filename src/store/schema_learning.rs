//! Declared schema — the learning loop's `feedback` counters, the
//! `feedback_events` provenance log, the `retrieval_log` query log, and the
//! `bandit_arms` posteriors, plus the per-symbol `code_feedback` counters
//! and the mined `query_expansions`. The run-history tables live in
//! `schema_history.rs`.
//!
//! Since #209 it also declares the three candidate-observation tables —
//! `candidate_query_observations`, `candidate_observations` and
//! `candidate_judgments` — which persist the #208 candidate observation
//! contract for a real query and the reviewed verdicts resolved against it.
//! They are learning-loop tables like the rest: written by a retrieval run,
//! read by evaluation and training. Contract:
//! `docs/designs/2026-09-18-candidate-observation-capture.md`.

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
#[index("idx_retrieval_log_recent", desc(at), desc(query_id))]
#[index("idx_retrieval_log_source_at", source, at)]
pub struct RetrievalLog {
    /// Query id (`utilities::query_id::generate_query_id`).
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

/// `candidate_query_observations`: one row per captured `find` run — the
/// per-query envelope of the #208 candidate observation contract.
///
/// `at` is both the contract's `reference_time` and the retention key. It is
/// written in `store::memory_row::iso_format`, the same fixed-width ISO-8601
/// UTC rendering `retrieval_log.at` uses, so a plain string `<` against a
/// rendered cutoff is exactly chronological.
#[table(name = "candidate_query_observations")]
#[index("idx_candidate_query_observations_at", at)]
pub struct CandidateQueryObservations {
    /// `o-<yyyymmdd>-<8hex>`, minted through `utilities::dated_id`.
    #[column(primary_key)]
    pub observation_id: Text,
    /// The `candidate_observation::OBSERVATION_VERSION` this row was written
    /// at. A reader that meets an unknown value must refuse the record.
    #[column(not_null)]
    pub observation_version: Integer,
    /// The `retrieval_log.query_id` of the same run; NULL when that write
    /// failed, since capture never depends on telemetry succeeding.
    pub query_id: Text,
    /// The query text, verbatim as retrieval received it.
    #[column(not_null)]
    pub query: Text,
    /// Which surface produced the run (a `utilities::telemetry::source` const).
    #[column(not_null)]
    pub source: Text,
    /// `EffectiveFilters` as JSON: every filter that was in effect, per leg.
    #[column(not_null)]
    pub filters_json: Text,
    /// `RetrievalVersion` as JSON: binary, schema, knobs and corpus snapshot.
    #[column(not_null)]
    pub retrieval_json: Text,
    /// `RetrievalVersion::knobs_hash`, denormalized so two runs' ranking
    /// configuration can be compared without parsing the JSON.
    #[column(not_null)]
    pub knobs_hash: Text,
    /// `RetrievalVersion::corpus.digest`, denormalized for the same reason.
    #[column(not_null)]
    pub corpus_digest: Text,
    /// `1` when ACT-R activation was time-independent for this run.
    #[column(not_null)]
    pub decay_frozen: Integer,
    /// The shared pool size every leg was fetched at.
    #[column(not_null)]
    pub pool_size: Integer,
    /// Page size the `returned_position` values were cut at.
    #[column(not_null)]
    pub page_limit: Integer,
    /// Page offset the run was sliced from.
    #[column(not_null)]
    pub page_offset: Integer,
    /// How many `candidate_observations` rows this observation actually has.
    #[column(not_null)]
    pub candidate_count: Integer,
    /// `1` when the configured candidate bound cut the pool. A consumer must
    /// not compute candidate-pool recall from a truncated observation.
    #[column(not_null)]
    pub truncated: Integer,
    /// The run's start instant, `memory_row::iso_format`.
    #[column(not_null)]
    pub at: Text,
}

/// `candidate_observations`: one row per candidate retrieval produced for one
/// captured query, in retrieval's own order.
///
/// `candidate_ref` is the only matchable key: it is the domain-qualified
/// reference string `candidate_identity::parse_ref` inverts exactly, and it
/// carries the stable identity AND the content version. `locator_json` holds
/// the display fields, and nothing in this schema indexes or predicates on
/// them — the contract's "never match on a locator" rule is enforced by there
/// being no way to.
#[table(name = "candidate_observations")]
#[primary_key(observation_id, pool_position)]
#[index("idx_candidate_observations_ref", candidate_ref)]
pub struct CandidateObservations {
    /// The `candidate_query_observations` row this candidate belongs to.
    #[column(not_null)]
    pub observation_id: Text,
    /// 1-based position in the candidate pool, before any arm reorders it.
    #[column(not_null)]
    pub pool_position: Integer,
    /// `memory` | `code` | `document`.
    #[column(not_null, check = "domain IN ('memory','code','document')")]
    pub domain: Text,
    /// The domain-qualified reference string for this candidate's identity.
    #[column(not_null)]
    pub candidate_ref: Text,
    /// The content version inside `candidate_ref`, denormalized. Empty when
    /// the candidate is unresolved.
    #[column(not_null)]
    pub content_version: Text,
    /// `1` when no content snapshot exists for this candidate — its row
    /// vanished mid-run, or a purge redacted it. Never matchable.
    #[column(not_null, default = "0")]
    pub unresolved: Integer,
    /// 1-based position on the displayed page; NULL when the candidate was in
    /// the pool but below the page cut.
    pub returned_position: Integer,
    /// The fused score retrieval assigned.
    #[column(not_null)]
    pub retrieval_score: Real,
    /// 1-based position within this candidate's own leg, before fusion.
    #[column(not_null)]
    pub rank_in_domain: Integer,
    /// Memory lexical-ladder tier; NULL for code and document candidates.
    pub tier: Integer,
    /// `BoundedText::text` — the passage, bounded at a character boundary.
    #[column(not_null)]
    pub text: Text,
    /// `BoundedText::sha256` — the digest of the FULL text, before bounding.
    #[column(not_null)]
    pub text_sha256: Text,
    /// `BoundedText::full_bytes` — the untruncated byte length.
    #[column(not_null)]
    pub text_full_bytes: Integer,
    /// `BoundedText::truncated`.
    #[column(not_null)]
    pub text_truncated: Integer,
    /// `CandidateLocator` as JSON. Display only, never read back for matching.
    #[column(not_null)]
    pub locator_json: Text,
}

/// `candidate_judgments`: one reviewed relevance verdict per candidate per
/// captured observation.
///
/// Keyed by the candidate reference the verdict was made against, so a
/// judgment can only ever name something retrieval actually returned.
#[table(name = "candidate_judgments")]
#[primary_key(observation_id, candidate_ref)]
#[index("idx_candidate_judgments_ref", candidate_ref)]
pub struct CandidateJudgments {
    /// The captured query this verdict answers.
    #[column(not_null)]
    pub observation_id: Text,
    /// The candidate that was judged, exactly as it was observed.
    #[column(not_null)]
    pub candidate_ref: Text,
    /// `memory` | `code` | `document`.
    #[column(not_null, check = "domain IN ('memory','code','document')")]
    pub domain: Text,
    /// Graded relevance, `0..=judgment::MAX_RELEVANCE`. `0` is an explicit
    /// "reviewed and not relevant", which is not the same as unjudged.
    #[column(not_null, check = "relevance >= 0 AND relevance <= 3")]
    pub relevance: Integer,
    /// `manual` or `implicit` — `utilities::telemetry`'s provenance
    /// vocabulary, reached through `feedback_tracking::Source` so the
    /// `explicit -> manual` mapping has exactly one owner.
    #[column(not_null, check = "provenance IN ('manual','implicit')")]
    pub provenance: Text,
    /// RFC3339 time the verdict was recorded.
    #[column(not_null)]
    pub at: Text,
}
