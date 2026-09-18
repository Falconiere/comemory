//! The learning loop: whether comemory's ranking was any good, and what it
//! does with the answer.
//!
//! Feedback records used/irrelevant verdicts for memories and code symbols
//! under an explicit provenance, so a stated judgement and an observed
//! signal never become the same row. Evaluation scores the real retrieval
//! pipeline against a golden set and searches the blend knobs —
//! deterministically, by seeded sample, or by an eval-gated Thompson
//! bandit. Mining distils reformulated queries into stored expansions.
//!
//! The persisted vocabularies and the `q-<yyyymmdd>-<8hex>` query-id
//! contract are shared leaves in `utilities::telemetry` and
//! [`crate::utilities::query_id`] (#166), which retrieval, graph and store
//! consume directly rather than through here. Every SQL statement belongs
//! to [`crate::store`]; what stays here is the policy it serves — the
//! provenance a verdict is stored under, the chunk-to-parent identity a
//! code counter is keyed by, and each batch's transaction boundary.

/// `comemory bandit`: Thompson-sample the `[tune]` grid, confirm, apply.
pub mod bandit;
/// `comemory benchmark` — the domain-aware offline retrieval benchmark.
pub mod benchmark;
/// Per-symbol code feedback counters and the rowid-to-identity rule.
pub mod code_feedback;
/// `GET /learning/{summary,evals,golden-set,expansions}`: the console
/// read model.
pub mod console;
/// `comemory eval`: score retrieval quality against a golden set.
pub mod eval;
/// Golden sets, metrics, the eval runner, mining and the tuning searches.
pub mod evaluation;
/// `comemory feedback`: record which hits were used.
pub mod feedback;
/// Per-memory feedback counters and the caller-facing `Source` vocabulary.
pub mod feedback_tracking;
/// `GET /learning/proposals`, `POST /learning/proposals/{id}/{apply,discard}`.
pub mod learning_proposals;
/// `comemory mine`: distill query-reformulation term mappings.
pub mod mine;
/// The shared `comemory.db` connection handle the feedback writers borrow.
pub mod telemetry;
/// `comemory tune`: grid-search the blend knobs, confirm, apply.
pub mod tune;

pub use telemetry::StatsDb;
