//! Benchmark metrics: candidate-pool recall measured separately from final
//! recall@k, MRR and nDCG@k, plus the paired bootstrap that puts an interval
//! around one arm's delta against the baseline.
//!
//! **The unit of relevance is the judgment, not the candidate.** One judgment
//! can match several pooled candidates, and a judgment that matched nothing at
//! all still belongs in the denominator — that is precisely what pool recall
//! has to measure. Missing positives are never inserted into the pool.

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::bandit_rng::SplitMix64;
use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;
use crate::domains::learning::evaluation::metrics;

/// One live judgment's contribution to a task: its grade, the corpus it
/// addresses, and every 1-based pool position it matched.
#[derive(Debug, Clone)]
pub struct MatchedJudgment {
    /// Graded relevance, `0..=3`. `0` is reviewed-and-not-relevant.
    pub relevance: u8,
    /// Which corpus the judgment's target addresses.
    pub domain: CandidateDomain,
    /// 1-based pool positions this judgment matched; empty when retrieval
    /// never produced the target.
    pub pool_positions: Vec<usize>,
}

impl MatchedJudgment {
    /// Whether this judgment counts toward a recall denominator.
    fn is_positive(&self) -> bool {
        self.relevance > 0
    }
}

/// The metrics one arm earned on one task.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TaskMetrics {
    /// Reviewed-relevant judgments retrieval produced anywhere in the pool,
    /// over all reviewed-relevant judgments. Arm-independent: every arm
    /// reorders the same pool, so this is retrieval's own ceiling.
    pub pool_recall: f64,
    /// The same fraction, restricted to the arm's first `k`.
    pub recall_at_k: f64,
    /// Reciprocal rank of the arm's first reviewed-relevant hit; `0.0` on a
    /// miss.
    pub mrr: f64,
    /// Normalized discounted cumulative gain at `k`, `gain = 2^relevance - 1`.
    pub ndcg_at_k: f64,
    /// Page candidates some judgment matched, at any grade.
    pub judged_in_page: usize,
    /// Page candidates no judgment matched at all.
    pub unjudged_in_page: usize,
    /// `judged_in_page / page length`; `0.0` for an empty page.
    pub judged_page_fraction: f64,
}

/// Score one arm's ordering of one task's pool.
///
/// `order` holds the 1-based pool positions in the arm's own order. `domain`
/// restricts every figure to the judgments addressing that corpus, which is how
/// a mixed-domain task contributes to each per-domain block; `None` scores all
/// of them.
pub fn score(
    judgments: &[MatchedJudgment],
    order: &[usize],
    k: usize,
    domain: Option<CandidateDomain>,
) -> TaskMetrics {
    let live: Vec<&MatchedJudgment> = judgments
        .iter()
        .filter(|j| domain.is_none_or(|d| j.domain == d))
        .collect();
    let page: &[usize] = order.get(..k.min(order.len())).unwrap_or_default();
    let positives: Vec<&&MatchedJudgment> = live.iter().filter(|j| j.is_positive()).collect();
    let judged_in_page = page
        .iter()
        .filter(|pos| live.iter().any(|j| j.pool_positions.contains(pos)))
        .count();
    TaskMetrics {
        pool_recall: fraction(&positives, |j| !j.pool_positions.is_empty()),
        recall_at_k: fraction(&positives, |j| {
            page.iter().any(|pos| j.pool_positions.contains(pos))
        }),
        mrr: reciprocal_rank(&positives, page),
        ndcg_at_k: ndcg(&live, &positives, page, k),
        judged_in_page,
        unjudged_in_page: page.len() - judged_in_page,
        judged_page_fraction: ratio(judged_in_page, page.len()),
    }
}

/// The fraction of reviewed-relevant judgments satisfying `hit`. An empty
/// positive set scores `0.0`, matching [`metrics::recall_at_k`]'s convention:
/// a task with no relevant judgment carries no signal and must not inflate an
/// average.
fn fraction(positives: &[&&MatchedJudgment], hit: impl Fn(&MatchedJudgment) -> bool) -> f64 {
    if positives.is_empty() {
        return 0.0;
    }
    let hits = positives.iter().filter(|j| hit(j)).count();
    hits as f64 / positives.len() as f64
}

/// `1 / rank` of the first page entry any reviewed-relevant judgment matched,
/// or `0.0` — [`metrics::first_hit_rank`]'s convention.
fn reciprocal_rank(positives: &[&&MatchedJudgment], page: &[usize]) -> f64 {
    page.iter()
        .position(|pos| positives.iter().any(|j| j.pool_positions.contains(pos)))
        .map_or(0.0, |i| 1.0 / (i as f64 + 1.0))
}

/// `DCG@k / IDCG@k`. The gain at a page position is the highest grade any live
/// judgment gave that candidate, so an unjudged candidate contributes `0` and a
/// judged-`0` candidate contributes `0` too — the report distinguishes them
/// through `judged_in_page` rather than through the gain.
fn ndcg(
    live: &[&MatchedJudgment],
    positives: &[&&MatchedJudgment],
    page: &[usize],
    k: usize,
) -> f64 {
    let dcg: f64 = page
        .iter()
        .enumerate()
        .map(|(i, pos)| {
            let grade = live
                .iter()
                .filter(|j| j.pool_positions.contains(pos))
                .map(|j| j.relevance)
                .max()
                .unwrap_or(0);
            discounted(grade, i)
        })
        .sum();
    let mut ideal: Vec<u8> = positives.iter().map(|j| j.relevance).collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let idcg: f64 = ideal
        .into_iter()
        .take(k)
        .enumerate()
        .map(|(i, grade)| discounted(grade, i))
        .sum();
    if idcg <= 0.0 { 0.0 } else { dcg / idcg }
}

/// `(2^grade - 1) / log2(position + 2)` for a 0-based `position`.
fn discounted(grade: u8, position: usize) -> f64 {
    let gain = f64::from((1u32 << u32::from(grade.min(31))) - 1);
    gain / (position as f64 + 2.0).log2()
}

/// `numerator / denominator`, `0.0` when the denominator is zero.
fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// One arm's aggregate over the tasks it scored. Means are taken over JUDGED
/// tasks — those with at least one reviewed-relevant judgment — because a task
/// with none has an undefined recall; `tasks` reports the full count so the
/// difference is visible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSummary {
    /// Tasks scored.
    pub tasks: usize,
    /// Tasks with at least one reviewed-relevant judgment.
    pub judged_tasks: usize,
    /// Mean candidate-pool recall.
    pub pool_recall: f64,
    /// 95% percentile-bootstrap interval around `pool_recall`.
    pub pool_recall_ci: (f64, f64),
    /// Mean recall@k.
    pub recall_at_k: f64,
    /// 95% interval around `recall_at_k`.
    pub recall_at_k_ci: (f64, f64),
    /// Mean reciprocal rank.
    pub mrr: f64,
    /// 95% interval around `mrr`.
    pub mrr_ci: (f64, f64),
    /// Mean nDCG@k.
    pub ndcg_at_k: f64,
    /// 95% interval around `ndcg_at_k`.
    pub ndcg_at_k_ci: (f64, f64),
    /// Page candidates some judgment matched, summed over tasks.
    pub judged_in_page: usize,
    /// Page candidates no judgment matched, summed over tasks.
    pub unjudged_in_page: usize,
    /// Judged share of every scored page, the coverage statement.
    pub judged_page_fraction: f64,
}

/// Aggregate `per_task` over the tasks flagged judged in `judged`, drawing
/// every interval from one seeded stream so a report reproduces byte for byte.
pub fn summarize(per_task: &[TaskMetrics], judged: &[bool], seed: u64) -> MetricSummary {
    let kept: Vec<&TaskMetrics> = per_task
        .iter()
        .zip(judged.iter())
        .filter(|(_, keep)| **keep)
        .map(|(m, _)| m)
        .collect();
    let mut rng = SplitMix64::new(seed);
    let mut interval = |pick: fn(&TaskMetrics) -> f64| {
        let values: Vec<f64> = kept.iter().map(|m| pick(m)).collect();
        let point = mean(&values);
        let ci = metrics::bootstrap_ci(&values, metrics::BOOTSTRAP_ITERS, &mut rng);
        (point, ci)
    };
    let (pool_recall, pool_recall_ci) = interval(|m| m.pool_recall);
    let (recall_at_k, recall_at_k_ci) = interval(|m| m.recall_at_k);
    let (mrr, mrr_ci) = interval(|m| m.mrr);
    let (ndcg_at_k, ndcg_at_k_ci) = interval(|m| m.ndcg_at_k);
    let judged_in_page = per_task.iter().map(|m| m.judged_in_page).sum();
    let unjudged_in_page = per_task.iter().map(|m| m.unjudged_in_page).sum::<usize>();
    MetricSummary {
        tasks: per_task.len(),
        judged_tasks: kept.len(),
        pool_recall,
        pool_recall_ci,
        recall_at_k,
        recall_at_k_ci,
        mrr,
        mrr_ci,
        ndcg_at_k,
        ndcg_at_k_ci,
        judged_in_page,
        unjudged_in_page,
        judged_page_fraction: ratio(judged_in_page, judged_in_page + unjudged_in_page),
    }
}

/// One arm's paired delta against the baseline on a single metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDelta {
    /// Which metric the delta is on.
    pub metric: String,
    /// Mean of the per-task `arm - baseline` differences.
    pub mean_delta: f64,
    /// 95% percentile-bootstrap interval around `mean_delta`.
    pub ci: (f64, f64),
    /// Tasks the delta was taken over.
    pub tasks: usize,
}

/// Bootstrap the per-task differences `arm - baseline`. Pairing over identical
/// candidate snapshots removes the corpus variance an unpaired comparison
/// carries. Positions beyond the shorter slice are ignored.
pub fn paired_delta(metric: &str, arm: &[f64], baseline: &[f64], seed: u64) -> PairedDelta {
    let deltas: Vec<f64> = arm
        .iter()
        .zip(baseline.iter())
        .map(|(a, b)| a - b)
        .collect();
    let mut rng = SplitMix64::new(seed);
    PairedDelta {
        metric: metric.to_string(),
        mean_delta: mean(&deltas),
        ci: metrics::bootstrap_ci(&deltas, metrics::BOOTSTRAP_ITERS, &mut rng),
        tasks: deltas.len(),
    }
}

/// Arithmetic mean, `0.0` for an empty slice.
fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

#[cfg(test)]
#[path = "tests/benchmark_metrics.rs"]
mod tests;
