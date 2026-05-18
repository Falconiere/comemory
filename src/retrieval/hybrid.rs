//! Memory-layer hybrid search. The "hybrid" name anticipates the FTS branch
//! that lands in a later task; today it is vector-only with a deterministic
//! threshold filter on top of `MemoryIndex::search`.

use crate::index::{MemoryHit, MemoryIndex};
use crate::prelude::*;

/// Vector search for memories.
///
/// Over-fetches `limit * 2` from LanceDB, sorts by score descending, drops
/// anything below `threshold`, and truncates back to `limit`. The over-fetch
/// gives the threshold room to act without starving callers that want a
/// fixed number of results.
pub async fn search_memory(
    index: &MemoryIndex,
    query_emb: &[f32],
    limit: usize,
    threshold: f32,
) -> Result<Vec<MemoryHit>> {
    let mut hits = index.search(query_emb, limit * 2).await?;
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.retain(|h| h.score >= threshold);
    hits.truncate(limit);
    Ok(hits)
}
