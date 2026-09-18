//! One arm's result over a captured run: its metrics overall and per corpus,
//! its latency distribution, its paired delta against the baseline, and the
//! verdict that delta earns against the set's declared budgets.
//!
//! A verdict is read off the paired interval, never the point estimate, and
//! insufficient data is `Inconclusive` rather than a small positive number.

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::benchmark_arm::{ArmOrder, ArmScores};
use crate::domains::learning::evaluation::benchmark_metrics::{
    self, MetricSummary, PairedDelta, TaskMetrics,
};
use crate::domains::learning::evaluation::benchmark_runner::TaskCapture;
use crate::domains::learning::evaluation::benchmark_set::Budgets;
use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;

/// How one arm's paired delta reads against the declared budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// The reference ordering every other arm is compared against.
    Baseline,
    /// The delta interval's lower bound clears `min_ndcg_gain`.
    Improved,
    /// The delta interval's upper bound falls below `-max_ndcg_regression`.
    Regressed,
    /// A real comparison that cleared neither threshold.
    Neutral,
    /// Fewer judged tasks than `min_tasks`: no verdict is available at all.
    Inconclusive,
}

/// A latency distribution over the run's tasks, in milliseconds.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LatencyMs {
    /// Median.
    pub p50: u64,
    /// 90th percentile.
    pub p90: u64,
    /// 95th percentile.
    pub p95: u64,
    /// Slowest task.
    pub max: u64,
    /// Arithmetic mean.
    pub mean: f64,
}

/// Percentiles of `samples` at the pinned index `((p/100) * (len-1)).round()`,
/// the same rule the existing bootstrap percentile uses.
pub fn latency(samples: &[u64]) -> LatencyMs {
    if samples.is_empty() {
        return LatencyMs::default();
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let at = |p: f64| {
        let last = sorted.len().saturating_sub(1);
        let index = ((p / 100.0) * last as f64).round() as usize;
        sorted.get(index.min(last)).copied().unwrap_or(0)
    };
    LatencyMs {
        p50: at(50.0),
        p90: at(90.0),
        p95: at(95.0),
        max: sorted.last().copied().unwrap_or(0),
        mean: sorted.iter().sum::<u64>() as f64 / sorted.len() as f64,
    }
}

/// One arm's captured ordering of every task, ready to be scored.
pub struct ArmInput<'a> {
    /// Arm name; the baseline's is `benchmark_arm::BASELINE_ARM`.
    pub name: String,
    /// The scores file this arm came from; `None` for the baseline.
    pub scores: Option<&'a ArmScores>,
    /// Per-task orderings, aligned with the captures.
    pub orders: Vec<ArmOrder>,
}

/// One arm's whole result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmReport {
    /// Arm name.
    pub name: String,
    /// Free-form scorer identity, for an arm that came from a scores file.
    pub scorer_version: Option<String>,
    /// Mean share of each task's pool this arm supplied a score for.
    pub scored_fraction: f64,
    /// Metrics over every judgment, regardless of corpus.
    pub overall: MetricSummary,
    /// Metrics restricted to each corpus's own judgments.
    pub per_domain: Vec<(String, MetricSummary)>,
    /// Latency distribution over the run's tasks.
    pub latency_ms: LatencyMs,
    /// The paired nDCG@k delta against the baseline; `None` on the baseline.
    pub paired_vs_baseline: Option<PairedDelta>,
    /// How that delta reads against the budgets.
    pub verdict: Verdict,
    /// Whether `latency_ms.p95` is within `max_p95_task_ms`. Reported apart
    /// from `verdict` so a quality result and a latency result never merge.
    pub latency_within_budget: bool,
}

/// What one arm's scoring pass shares with the run around it.
pub struct ArmContext<'a> {
    /// The recall@k / nDCG@k cut.
    pub k: usize,
    /// Whether this arm is the reference ordering.
    pub is_baseline: bool,
    /// The baseline's per-task nDCG@k, for the paired delta.
    pub baseline_ndcg: &'a [f64],
    /// The budgets the verdict is read against.
    pub budgets: Budgets,
}

/// Score one arm over every capture and read its verdict.
pub fn build(captures: &[TaskCapture], arm: &ArmInput<'_>, ctx: &ArmContext<'_>) -> ArmReport {
    let per_task = score_arm(captures, arm, ctx.k, None);
    let seed = summary_seed(&arm.name, captures.len(), ctx.k);
    let overall = benchmark_metrics::summarize(&per_task, &judged_flags(captures, None), seed);
    let latency_ms = latency(&captures.iter().map(|c| c.retrieval_ms).collect::<Vec<_>>());
    let paired = (!ctx.is_baseline).then(|| {
        let mine: Vec<f64> = per_task.iter().map(|m| m.ndcg_at_k).collect();
        benchmark_metrics::paired_delta("ndcg_at_k", &mine, ctx.baseline_ndcg, seed)
    });
    ArmReport {
        scored_fraction: mean_fraction(arm),
        verdict: verdict(ctx, overall.judged_tasks, paired.as_ref()),
        latency_within_budget: latency_ms.p95 <= ctx.budgets.max_p95_task_ms,
        name: arm.name.clone(),
        scorer_version: arm.scores.and_then(|s| s.scorer_version.clone()),
        per_domain: per_domain(captures, arm, ctx.k, seed),
        overall,
        latency_ms,
        paired_vs_baseline: paired,
    }
}

/// Metrics restricted to each corpus's own judgments, so a mixed-domain task
/// contributes to every domain it carries a judgment for.
fn per_domain(
    captures: &[TaskCapture],
    arm: &ArmInput<'_>,
    k: usize,
    seed: u64,
) -> Vec<(String, MetricSummary)> {
    CandidateDomain::all()
        .into_iter()
        .map(|domain| {
            let scoped = score_arm(captures, arm, k, Some(domain));
            let flags = judged_flags(captures, Some(domain));
            (
                domain.as_str().to_string(),
                benchmark_metrics::summarize(&scoped, &flags, seed),
            )
        })
        .collect()
}

/// Which captures carry at least one reviewed-relevant judgment, optionally
/// restricted to one corpus. Means are taken over these.
fn judged_flags(captures: &[TaskCapture], domain: Option<CandidateDomain>) -> Vec<bool> {
    captures
        .iter()
        .map(|c| {
            c.matched
                .iter()
                .any(|j| j.relevance > 0 && domain.is_none_or(|d| j.domain == d))
        })
        .collect()
}

/// Score one arm over every capture, optionally restricted to one corpus.
pub fn score_arm(
    captures: &[TaskCapture],
    arm: &ArmInput<'_>,
    k: usize,
    domain: Option<CandidateDomain>,
) -> Vec<TaskMetrics> {
    captures
        .iter()
        .enumerate()
        .map(|(index, capture)| {
            let order = arm.orders.get(index).map(|o| o.order.as_slice());
            benchmark_metrics::score(&capture.matched, order.unwrap_or_default(), k, domain)
        })
        .collect()
}

/// Mean share of each task's pool an arm supplied a score for.
fn mean_fraction(arm: &ArmInput<'_>) -> f64 {
    if arm.orders.is_empty() {
        return 0.0;
    }
    arm.orders.iter().map(|o| o.scored_fraction).sum::<f64>() / arm.orders.len() as f64
}

/// Read a verdict off the paired interval, never off the point estimate. Too
/// few judged tasks is `Inconclusive` whatever the numbers say.
fn verdict(ctx: &ArmContext<'_>, judged_tasks: usize, paired: Option<&PairedDelta>) -> Verdict {
    if judged_tasks < ctx.budgets.min_tasks {
        return Verdict::Inconclusive;
    }
    match paired {
        None if ctx.is_baseline => Verdict::Baseline,
        None => Verdict::Inconclusive,
        Some(delta) if delta.ci.0 > ctx.budgets.min_ndcg_gain => Verdict::Improved,
        Some(delta) if delta.ci.1 < -ctx.budgets.max_ndcg_regression => Verdict::Regressed,
        Some(_) => Verdict::Neutral,
    }
}

/// Deterministic bootstrap seed per arm: the arm name, the task count and the
/// cut, so one arm's intervals reproduce and two arms do not share a stream.
fn summary_seed(arm: &str, tasks: usize, k: usize) -> u64 {
    let mut seed: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in arm
        .as_bytes()
        .iter()
        .chain(&tasks.to_le_bytes())
        .chain(&k.to_le_bytes())
    {
        seed ^= u64::from(*byte);
        seed = seed.wrapping_mul(0x0000_0100_0000_01b3);
    }
    seed
}

#[cfg(test)]
#[path = "tests/benchmark_arm_report.rs"]
mod tests;
