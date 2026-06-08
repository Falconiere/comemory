//! Reciprocal Rank Fusion across two ranked memory lists.
//!
//! Each input is ranked best-first. The fused score for a memory id is
//! the sum of `1 / (K + rank + 1)` across every list it appears in.
//! Output is sorted by score descending and truncated to `top_k`.

use std::collections::HashMap;

/// One row in a ranked memory list passed to [`rrf`]. `score` is kept
/// for compatibility with caller-side rendering; RRF itself uses ranks
/// only.
#[derive(Debug, Clone)]
pub struct RankedHit {
    /// Identifier of the ranked memory.
    pub memory_id: String,
    /// Caller-supplied score (unused by RRF — RRF reads ranks).
    pub score: f32,
}

/// RRF constant. Matches the Cormack/Clarke/Buettcher original; larger
/// values flatten the curve so deeper-rank hits matter more.
const K: f32 = 60.0;

/// Fuse two ranked lists with Reciprocal Rank Fusion and return the
/// top-`top_k` rows sorted by fused score descending.
pub fn rrf(a: &[RankedHit], b: &[RankedHit], top_k: usize) -> Vec<RankedHit> {
    let mut acc: HashMap<String, f32> = HashMap::new();
    for (rank, h) in a.iter().enumerate() {
        *acc.entry(h.memory_id.clone()).or_default() += 1.0 / (K + rank as f32 + 1.0);
    }
    for (rank, h) in b.iter().enumerate() {
        *acc.entry(h.memory_id.clone()).or_default() += 1.0 / (K + rank as f32 + 1.0);
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
