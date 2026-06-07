//! Scoring helpers used by the retrieval pipeline. Kept allocation-light and
//! pure so they can be reused by later fusion / reranking layers.

use std::collections::HashMap;

/// Z-score normalize `xs` in place-returning form: each output element is
/// `(x - mean) / sd`. Empty input returns an empty vec. A tiny epsilon
/// (`1e-9`) protects against divide-by-zero when all inputs are equal.
pub fn z_normalize(xs: &[f32]) -> Vec<f32> {
    if xs.is_empty() {
        return Vec::new();
    }
    let n = xs.len() as f32;
    let mean: f32 = xs.iter().sum::<f32>() / n;
    let var: f32 = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n;
    let sd = var.sqrt().max(1e-9);
    xs.iter().map(|x| (x - mean) / sd).collect()
}

/// Confidence gap: `top1 - top2` on an already-descending slice. Returns the
/// only value when length 1, and `0.0` for an empty slice — both treated as
/// "no second result to compare against".
pub fn confidence_gap(sorted_desc: &[f32]) -> f32 {
    match sorted_desc {
        [] => 0.0,
        [a] => *a,
        [a, b, ..] => a - b,
    }
}

/// Reciprocal Rank Fusion. Each input is a ranking (best first); the score for
/// an id is the sum of `1 / (k + rank)` (1-indexed) across every ranking it
/// appears in. Output is sorted by score descending, with ascending id as a
/// stable tie-break so callers get deterministic ordering.
///
/// `k` is the RRF constant (typical value `60.0`); larger values flatten the
/// curve so deeper-rank hits matter more relative to top-of-list hits.
pub fn rrf_fuse<S>(rankings: &[&[S]], k: f32) -> Vec<(String, f32)>
where
    S: AsRef<str>,
{
    let mut scores: HashMap<String, f32> = HashMap::new();
    for ranking in rankings {
        for (i, id) in ranking.iter().enumerate() {
            let rank = (i + 1) as f32;
            *scores.entry(id.as_ref().to_string()).or_insert(0.0) += 1.0 / (k + rank);
        }
    }
    let mut out: Vec<(String, f32)> = scores.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}
