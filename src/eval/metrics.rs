//! Pure retrieval-quality metrics: recall@k, MRR building blocks, and the
//! percentile bootstrap that puts an uncertainty interval around them.

use crate::eval::bandit_rng::SplitMix64;

/// Resample count used by the eval runner. 1000 keeps the 2.5/97.5
/// percentile indices stable to three decimals without a visible cost on a
/// golden set of a few dozen queries.
pub const BOOTSTRAP_ITERS: usize = 1000;

/// Fraction of `relevant` ids appearing in the first `k` of `returned`.
/// An empty `relevant` set scores 0.0 — a golden pair with no live
/// relevant ids carries no signal and must not inflate the average.
pub fn recall_at_k(relevant: &[String], returned: &[String], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top: std::collections::HashSet<&str> =
        returned.iter().take(k).map(String::as_str).collect();
    let hit = relevant.iter().filter(|r| top.contains(r.as_str())).count();
    hit as f64 / relevant.len() as f64
}

/// One-based rank of the first relevant id in `returned`, or `None`.
/// `1 / rank` summed over queries / query count = MRR.
pub fn first_hit_rank(relevant: &[String], returned: &[String]) -> Option<usize> {
    let rel: std::collections::HashSet<&str> = relevant.iter().map(String::as_str).collect();
    returned
        .iter()
        .position(|r| rel.contains(r.as_str()))
        .map(|p| p + 1)
}

/// Percentile bootstrap: resample `scores` with replacement `iters` times
/// and return the (2.5th, 97.5th) percentile interval of the resampled
/// means. Fewer than two scores (or `iters == 0`) carry no spread, so the
/// interval collapses to the point estimate — the mean of what exists,
/// `(0.0, 0.0)` when empty. `rng` is advanced only when a real resample
/// runs, so callers sharing one stream stay reproducible either way.
///
/// Trade-off: draw indices come from `next_u64() % len`, matching the tune
/// sampler; the modulo bias at golden-set lengths is orders of magnitude
/// below the score noise the interval is meant to expose.
pub fn bootstrap_ci(scores: &[f64], iters: usize, rng: &mut SplitMix64) -> (f64, f64) {
    if scores.len() < 2 || iters == 0 {
        let point = mean(scores);
        return (point, point);
    }
    let n = scores.len();
    let mut means = Vec::with_capacity(iters);
    for _ in 0..iters {
        let mut sum = 0.0;
        for _ in 0..n {
            // `% n` keeps `pick` inside `0..n`, so index directly: a
            // defaulted miss would fold a silent 0.0 into the resampled
            // mean and hide the broken invariant instead of surfacing it.
            let pick = (rng.next_u64() % n as u64) as usize;
            sum += scores[pick];
        }
        means.push(sum / n as f64);
    }
    means.sort_by(f64::total_cmp);
    (percentile(&means, 2.5), percentile(&means, 97.5))
}

/// Arithmetic mean of `xs`, `0.0` for an empty slice.
fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// The `p`-th percentile of an ascending, non-empty slice, at the pinned
/// index `((p / 100) × (len − 1)).round()`.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let last = sorted.len().saturating_sub(1);
    let idx = ((p / 100.0) * last as f64).round() as usize;
    sorted.get(idx.min(last)).copied().unwrap_or(0.0)
}

#[cfg(test)]
#[path = "tests/metrics.rs"]
mod tests;
