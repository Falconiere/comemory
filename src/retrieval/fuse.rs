//! Reciprocal Rank Fusion across two ranked memory lists.
//!
//! Each input is ranked best-first. The fused score for a memory id is
//! the sum of `1 / (rrf_k + rank + 1)` across every list it appears in.
//! Output is sorted by score descending and truncated to `top_k`.

use std::collections::HashMap;

/// One row in a ranked memory list passed to [`rrf`] / [`rrf_k`]. `score` is
/// kept for compatibility with caller-side rendering; RRF itself uses ranks only.
#[derive(Debug, Clone)]
pub struct RankedHit {
    /// Identifier of the ranked memory.
    pub memory_id: String,
    /// Caller-supplied score (unused by RRF — RRF reads ranks).
    pub score: f32,
}

/// Default RRF constant. Matches the Cormack/Clarke/Buettcher original;
/// larger values flatten the curve so deeper-rank hits matter more.
const DEFAULT_RRF_K: f32 = 60.0;

/// Fuse two ranked lists with Reciprocal Rank Fusion using the default
/// constant (`60.0`) and return the top-`top_k` rows sorted by fused
/// score descending.
pub fn rrf(a: &[RankedHit], b: &[RankedHit], top_k: usize) -> Vec<RankedHit> {
    rrf_k(a, b, top_k, DEFAULT_RRF_K)
}

/// Fuse two ranked lists with Reciprocal Rank Fusion using a caller-supplied
/// constant `k` and return the top-`top_k` rows sorted by fused score
/// descending. Use this variant when the RRF constant comes from config
/// (`cfg.retrieval.rrf_k`).
pub fn rrf_k(a: &[RankedHit], b: &[RankedHit], top_k: usize, k: f32) -> Vec<RankedHit> {
    let mut acc: HashMap<String, f32> = HashMap::new();
    for (rank, h) in a.iter().enumerate() {
        *acc.entry(h.memory_id.clone()).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    for (rank, h) in b.iter().enumerate() {
        *acc.entry(h.memory_id.clone()).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    let mut merged: Vec<RankedHit> = acc
        .into_iter()
        .map(|(memory_id, score)| RankedHit { memory_id, score })
        .collect();
    merged.sort_by(|x, y| {
        y.score
            .partial_cmp(&x.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.memory_id.cmp(&y.memory_id))
    });
    merged.truncate(top_k);
    merged
}
