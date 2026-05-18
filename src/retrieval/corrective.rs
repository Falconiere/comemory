//! Corrective-fallback signal: decide whether the pipeline should run a
//! second-pass query (graph expand, broader limit, looser filter) based on
//! the shape of the first-pass hit list.

use crate::index::MemoryHit;

/// Returns `true` when the caller should fire the corrective fallback.
///
/// Triggers on either of two conditions:
/// - Fewer than 3 hits returned — too thin to trust a single top result.
/// - `top1.score - top2.score < min_confidence` — top-1 not meaningfully
///   ahead of top-2.
pub fn should_fallback(hits: &[MemoryHit], min_confidence: f32) -> bool {
    if hits.len() < 3 {
        return true;
    }
    let gap = hits[0].score - hits[1].score;
    gap < min_confidence
}
