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
/// Golden-set model: hand-written YAML pairs plus the feedback harvest.
pub mod golden;
/// Pure retrieval-quality metrics: recall@k, MRR, percentile bootstrap CI.
pub mod metrics;
/// Reformulation mining: failed-to-fixed query pairs into expansions.
pub mod mine;
/// Drive the real retrieval pipeline over a golden set and score it.
pub mod runner;
/// Deterministic and sampled search over blend weights, scored by eval MRR.
pub mod tune;
/// Seeded uniform sampling over the `[tune]` grid pools.
pub mod tune_sample;
