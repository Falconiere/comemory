//! Dense + sparse retrieval fusion. Runs the vector path (`MemoryIndex`) and
//! the BM25 path (`retrieval::fts`) independently, RRF-fuses their ranked id
//! lists, then reifies `MemoryHit` rows for the top `limit` ids. Hits that
//! made it through the dense over-fetch window are reused directly; sparse-only
//! hits (BM25-ranked but outside the dense pool) are materialised by loading
//! the record from the markdown store, since markdown remains the source of
//! truth.

use std::collections::HashMap;

use crate::config::paths::Paths;
use crate::index::{Fts, MemoryHit, MemoryIndex};
use crate::memory::MemoryStore;
use crate::prelude::*;
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
/// `dense_threshold` is the cosine-score floor applied to dense candidates
/// **before** fusion. RRF only uses ranks (not scores), so this is the only
/// place we can prune weak-similarity rows without distorting the BM25 path.
/// Pass `0.0` to disable filtering (the bench harness does this so the
/// measurement reflects pure fusion cost, not threshold sensitivity).
///
/// Opens the FTS5 database on every call. Callers that already hold an
/// `Fts` handle (long-lived servers, benches that want to measure fusion
/// latency without re-paying the connection cost) should call
/// [`search_memory_fused_with_fts`] directly.
pub async fn search_memory_fused(
    idx: &MemoryIndex,
    paths: &Paths,
    query_emb: &[f32],
    query_text: &str,
    limit: usize,
    dense_threshold: f32,
    rrf_k: f32,
) -> Result<Vec<MemoryHit>> {
    let fts_db = paths.fts_db();
    let fts = if fts_db.exists() {
        Some(Fts::open(&fts_db)?)
    } else {
        None
    };
    search_memory_fused_with_fts(
        idx,
        fts.as_ref(),
        paths,
        query_emb,
        query_text,
        limit,
        dense_threshold,
        rrf_k,
    )
    .await
}

/// Variant of [`search_memory_fused`] that accepts a pre-opened FTS handle so
/// callers can amortise the `Fts::open` cost across many queries.
///
/// `fts = None` means "FTS unavailable" and the function transparently
/// degrades to dense-only retrieval. This mirrors the on-disk-missing
/// fallback that [`search_memory_fused`] applies when `fts.sqlite` does not
/// exist yet.
///
/// `dense_threshold` is a cosine-similarity floor applied to the dense
/// candidates **before** they feed into RRF. Use `0.0` to disable.
pub async fn search_memory_fused_with_fts(
    idx: &MemoryIndex,
    fts: Option<&Fts>,
    paths: &Paths,
    query_emb: &[f32],
    query_text: &str,
    limit: usize,
    dense_threshold: f32,
    rrf_k: f32,
) -> Result<Vec<MemoryHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let over = limit.saturating_mul(4).max(limit);

    let mut dense_hits = idx.search(query_emb, over).await?;
    // Prune weak-similarity dense candidates before fusion. RRF uses ranks
    // only, so this is the one place where the cosine threshold can act
    // without distorting the BM25 side of the fused list.
    dense_hits.retain(|h| h.score >= dense_threshold);
    let dense_ids: Vec<String> = dense_hits.iter().map(|h| h.id.clone()).collect();
    let sparse_ids: Vec<String> = match fts {
        Some(handle) => handle
            .search(query_text, over)?
            .into_iter()
            .map(|h| h.id)
            .collect(),
        None => Vec::new(),
    };

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
