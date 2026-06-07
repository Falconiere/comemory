//! Dense + sparse retrieval fusion. Runs the vector path (`MemoryIndex`) and
//! the BM25 path (`retrieval::fts`) independently, RRF-fuses their ranked id
//! lists, then reifies `MemoryHit` rows for the top `limit` ids. Hits that
//! made it through the dense over-fetch window are reused directly; sparse-only
//! hits (BM25-ranked but outside the dense pool) are materialised by loading
//! the record from the markdown store, since markdown remains the source of
//! truth.

use std::collections::HashMap;

use crate::config::paths::Paths;
use crate::index::{MemoryHit, MemoryIndex};
use crate::memory::MemoryStore;
use crate::prelude::*;
use crate::retrieval::fts::search_fts_ids;
use crate::retrieval::rank::rrf_fuse;

/// Run vector + BM25 retrieval over the memory layer, fuse the rankings with
/// Reciprocal Rank Fusion, and return the top `limit` materialized hits.
///
/// Each underlying index is over-fetched by 4x so fusion has enough overlap
/// to act without inflating the SQL or vector query. Sparse-only hits (BM25
/// matches outside the dense over-fetch window) are reified through
/// `MemoryStore::load` so they are not silently dropped — markdown is the
/// source of truth. When a fused id has no matching markdown file (deleted
/// between FTS upsert and query) we log via `tracing::warn!` and skip it.
///
pub async fn search_memory_fused(
    idx: &MemoryIndex,
    paths: &Paths,
    query_emb: &[f32],
    query_text: &str,
    limit: usize,
    rrf_k: f32,
) -> Result<Vec<MemoryHit>> {
    let over = limit.saturating_mul(4).max(limit);

    let dense_hits = idx.search(query_emb, over).await?;
    let dense_ids: Vec<String> = dense_hits.iter().map(|h| h.id.clone()).collect();
    let fts_db = paths.index_dir().join("fts.sqlite");
    let sparse_ids = search_fts_ids(&fts_db, query_text, over)?;

    let dense_ref: &[String] = &dense_ids;
    let sparse_ref: &[String] = &sparse_ids;
    let fused = rrf_fuse(&[dense_ref, sparse_ref], rrf_k);

    let by_id: HashMap<String, MemoryHit> =
        dense_hits.into_iter().map(|h| (h.id.clone(), h)).collect();

    let store = MemoryStore::new(paths.clone());
    let mut out = Vec::with_capacity(limit);
    for (id, score) in fused {
        if let Some(mut hit) = by_id.get(&id).cloned() {
            hit.score = score;
            out.push(hit);
        } else {
            match store.load(&id) {
                Ok(rec) => out.push(MemoryHit {
                    id: rec.frontmatter.id.clone(),
                    score,
                    body: rec.body,
                    kind: rec.frontmatter.kind,
                    repo: rec.frontmatter.repo,
                }),
                Err(e) => {
                    tracing::warn!(
                        "fused search: sparse-only id {} not on disk ({}), skipping",
                        id,
                        e
                    );
                    continue;
                }
            }
        }
        if out.len() == limit {
            break;
        }
    }
    Ok(out)
}
