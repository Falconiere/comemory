//! Learning-loop evaluation: golden sets, metrics, the eval runner,
//! reformulation mining, and blend-weight tuning.
//!
//! Named `evaluation` rather than `eval` because the capability's
//! `comemory eval` command core already holds that name; these are the
//! algorithms it drives. The runner calls the real retrieval pipeline
//! exactly as a CLI caller would and never reimplements ranking.

/// Eval-gated Thompson bandit over the `[tune]` discrete grid.
pub mod bandit;
/// Dependency-free SplitMix64 plus Beta/Gamma sampling for the bandit.
pub mod bandit_rng;
/// Benchmark arms: the deterministic baseline and scored arms from a file.
pub mod benchmark_arm;
/// One arm's metrics, latency, paired delta and budget verdict.
pub mod benchmark_arm_report;
/// Benchmark metrics: pool recall, recall@k, MRR, nDCG@k, paired bootstrap.
pub mod benchmark_metrics;
/// Turn a fused pool into the contract's observations and filter record.
pub mod benchmark_observe;
/// The replayable benchmark artifact and its assembly.
pub mod benchmark_report;
/// Capture one task's candidate pool through the real retrieval legs.
pub mod benchmark_runner;
/// The versioned benchmark set: reviewed tasks, filters, ranking and budgets.
pub mod benchmark_set;
/// Identity, content version and bounded text, read off the leg rows.
pub mod candidate_facts;
/// Domain-qualified candidate identity and its reference-string codec.
pub mod candidate_identity;
/// The candidate observation contract: one candidate, and its query envelope.
pub mod candidate_observation;
/// Golden-set model: hand-written YAML pairs plus the feedback harvest.
pub mod golden;
/// Reviewed relevance judgments and how a target matches an observation.
pub mod judgment;
/// Pure retrieval-quality metrics: recall@k, MRR, percentile bootstrap CI.
pub mod metrics;
/// Reformulation mining: failed-to-fixed query pairs into expansions.
pub mod mine;
/// Hardware, memory use and the pinned corpus/index snapshot.
pub mod run_environment;
/// Drive the real retrieval pipeline over a golden set and score it.
pub mod runner;
/// The per-domain filters one benchmark task applies, and their inert-filter rule.
pub mod task_filters;
/// Deterministic and sampled search over blend weights, scored by eval MRR.
pub mod tune;
/// Seeded uniform sampling over the `[tune]` grid pools.
pub mod tune_sample;
