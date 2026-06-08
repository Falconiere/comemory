//! Routes a query to the vector path, the lexical path, or the hybrid
//! path, and returns a uniform [`RoutedHit`] list.
//!
//! Decision table:
//! - vec = Some, query non-empty → **hybrid**: run both ANN and FTS5 BM25
//!   independently, fuse via RRF, truncate to `top_k`. This is the correct
//!   path when the caller supplies both a semantic vector *and* a text query.
//! - vec = Some, query empty → **pure vector**: ANN only; corrective lexical
//!   top-up when ANN returns fewer than `top_k` rows.
//! - vec = None → **pure lexical**: FTS5 BM25 only.

use rusqlite::Connection;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::fuse::{self, RankedHit};
use crate::store::{fts, vector};

/// One unified retrieval hit, regardless of which branch produced it.
#[derive(Debug, Clone)]
pub struct RoutedHit {
    /// Identifier of the matched memory row.
    pub memory_id: String,
    /// Higher-is-better score. Vector hits use `1.0 - distance`; lexical
    /// hits use `-bm25`; hybrid hits use the RRF fused score.
    pub score: f32,
    /// Which branch produced this hit.
    pub source: Source,
}

/// Which retrieval branch produced a [`RoutedHit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// sqlite-vec KNN result.
    Vector,
    /// FTS5 BM25 result.
    Lexical,
    /// RRF fusion of vector + lexical branches.
    Hybrid,
}

/// Run the retrieval pipeline for `query`.
///
/// When both `vec` and a non-empty `query` are provided, both ANN and
/// FTS5 BM25 run independently and their results are fused via RRF.
/// When only `vec` is provided (empty `query`), ANN runs with a
/// corrective lexical top-up when fewer than `top_k` rows survive.
/// When only `query` is provided (no `vec`), only the lexical path runs.
pub fn route(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let k = cfg.retrieval.top_k;

    match vec {
        Some(v) if !query.is_empty() => route_hybrid(cfg, conn, query, v, k, repo),
        Some(v) => route_vector_with_corrective(conn, query, v, k, repo),
        None => route_lexical(conn, query, k),
    }
}

/// Hybrid path: run ANN + FTS5 independently and fuse via RRF.
fn route_hybrid(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: &[f32],
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let ann = vector::knn_memory(conn, vec, k, repo)?;
    let lex = fts::search_memory(conn, query, k)?;

    let ann_ranked: Vec<RankedHit> = ann
        .into_iter()
        .map(|h| RankedHit {
            memory_id: h.memory_id,
            score: 1.0 - h.distance,
        })
        .collect();
    let lex_ranked: Vec<RankedHit> = lex
        .into_iter()
        .map(|h| RankedHit {
            memory_id: h.memory_id,
            score: -h.score,
        })
        .collect();

    let fused = fuse::rrf_k(&ann_ranked, &lex_ranked, k, cfg.retrieval.rrf_k);
    Ok(fused
        .into_iter()
        .map(|h| RoutedHit {
            memory_id: h.memory_id,
            score: h.score,
            source: Source::Hybrid,
        })
        .collect())
}

/// Pure-vector path with corrective lexical top-up when ANN is short.
fn route_vector_with_corrective(
    conn: &Connection,
    query: &str,
    vec: &[f32],
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let ann = vector::knn_memory(conn, vec, k, repo)?;
    if ann.len() >= k {
        return Ok(ann
            .into_iter()
            .map(|h| RoutedHit {
                memory_id: h.memory_id,
                score: 1.0 - h.distance,
                source: Source::Vector,
            })
            .collect());
    }
    let mut routed: Vec<RoutedHit> = ann
        .into_iter()
        .map(|h| RoutedHit {
            memory_id: h.memory_id,
            score: 1.0 - h.distance,
            source: Source::Vector,
        })
        .collect();
    let need = k.saturating_sub(routed.len());
    if need > 0 && !query.is_empty() {
        let lex = fts::search_memory(conn, query, need)?;
        for hit in lex {
            if !routed.iter().any(|h| h.memory_id == hit.memory_id) {
                routed.push(RoutedHit {
                    memory_id: hit.memory_id,
                    score: -hit.score,
                    source: Source::Lexical,
                });
            }
        }
    }
    Ok(routed)
}

/// Pure-lexical path via FTS5 BM25.
fn route_lexical(conn: &Connection, query: &str, k: usize) -> Result<Vec<RoutedHit>> {
    let lex = fts::search_memory(conn, query, k)?;
    Ok(lex
        .into_iter()
        .map(|h| RoutedHit {
            memory_id: h.memory_id,
            score: -h.score,
            source: Source::Lexical,
        })
        .collect())
}
