//! Dense + sparse retrieval fusion. Runs the vector path (`MemoryIndex`) and
//! the BM25 path (`retrieval::fts`) independently, RRF-fuses their ranked id
//! lists, then reifies `MemoryHit` rows for the top `limit` ids out of the
//! dense table. The dense table is treated as the canonical source for body
//! text and metadata so we never re-read the markdown file.

use std::collections::HashMap;
use std::path::Path;

use crate::index::{MemoryHit, MemoryIndex};
use crate::prelude::*;
use crate::retrieval::fts::search_fts_ids;
use crate::retrieval::rank::rrf_fuse;

/// Run vector + BM25 retrieval over the memory layer, fuse the rankings with
/// Reciprocal Rank Fusion, and return the top `limit` materialized hits.
///
/// Each underlying index is over-fetched by 4x so fusion has enough overlap
/// to act without inflating the SQL or vector query.
pub async fn search_memory_fused(
    idx: &MemoryIndex,
    fts_db: impl AsRef<Path>,
    query_emb: &[f32],
    query_text: &str,
    limit: usize,
    rrf_k: f32,
) -> Result<Vec<MemoryHit>> {
    let over = limit.saturating_mul(4).max(limit);

    let dense_hits = idx.search(query_emb, over).await?;
    let dense_ids: Vec<String> = dense_hits.iter().map(|h| h.id.clone()).collect();
    let sparse_ids = search_fts_ids(fts_db, query_text, over)?;

    let dense_ref: &[String] = &dense_ids;
    let sparse_ref: &[String] = &sparse_ids;
    let fused = rrf_fuse(&[dense_ref, sparse_ref], rrf_k);

    let by_id: HashMap<String, MemoryHit> =
        dense_hits.into_iter().map(|h| (h.id.clone(), h)).collect();

    let mut out = Vec::with_capacity(limit);
    for (id, score) in fused {
        if let Some(mut hit) = by_id.get(&id).cloned() {
            hit.score = score;
            out.push(hit);
            if out.len() == limit {
                break;
            }
        }
    }
    Ok(out)
}
