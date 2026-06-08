//! Routes a query to the vector path, the lexical path, or the hybrid
//! path, and returns a uniform [`RoutedHit`] list.
//!
//! Decision table:
//! - vec = Some, query non-empty → **hybrid**: run both ANN and FTS5 BM25
//!   independently, fuse via RRF, truncate to `top_k`. This is the correct
//!   path when the caller supplies both a semantic vector *and* a text query.
//! - vec = Some, query empty → **pure vector**: ANN only. A lexical top-up
//!   is impossible here because FTS5 returns nothing for an empty query;
//!   callers that want the dense + sparse fusion must pass both `vec` and
//!   a non-empty `query` so the hybrid arm is taken.
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
/// When only `vec` is provided (empty `query`), only the ANN path runs:
/// FTS5 returns nothing for an empty query, so no lexical top-up is
/// possible. When only `query` is provided (no `vec`), only the lexical
/// path runs.
pub fn route(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let k = cfg.retrieval.top_k;

    // Trim the query before dispatching: a whitespace-only query like
    // `"   "` is lexically empty (FTS5 returns no rows for it) so the
    // hybrid arm would mislabel a vector-only result as `Source::Hybrid`
    // and downstream consumers would assume lexical contributed signal.
    let lex_meaningful = !query.trim().is_empty();
    match vec {
        Some(v) if lex_meaningful => route_hybrid(cfg, conn, query, v, k, repo),
        Some(v) => route_vector_only(conn, v, k, repo),
        None => route_lexical(conn, query, k, repo),
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
    let lex = fts::search_memory(conn, query, k, repo)?;

    let ann_ranked: Vec<RankedHit> = ann.into_iter().map(ann_to_ranked).collect();
    let lex_ranked: Vec<RankedHit> = lex.into_iter().map(lex_to_ranked).collect();

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

/// Pure-vector path. The lexical top-up that previously lived here was
/// dead: this arm is only reached when `query` is empty (the dispatcher
/// routes `vec + non-empty query` to [`route_hybrid`]), and FTS5 BM25
/// returns no rows for an empty query. Callers that want sparse+dense
/// fusion must pass a non-empty `query` so the hybrid arm fires.
fn route_vector_only(
    conn: &Connection,
    vec: &[f32],
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let ann = vector::knn_memory(conn, vec, k, repo)?;
    Ok(ann.into_iter().map(ann_to_routed).collect())
}

/// Pure-lexical path via FTS5 BM25.
fn route_lexical(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let lex = fts::search_memory(conn, query, k, repo)?;
    Ok(lex.into_iter().map(lex_to_routed).collect())
}

/// Map a `vector::MemoryHit` to a [`RankedHit`] for RRF fusion. Vector
/// distance is converted to a higher-is-better score via `1.0 - distance`.
fn ann_to_ranked(h: vector::MemoryHit) -> RankedHit {
    RankedHit {
        memory_id: h.memory_id,
        score: 1.0 - h.distance,
    }
}

/// Map an `fts::MemoryFtsHit` to a [`RankedHit`] for RRF fusion. BM25 is
/// lower-is-better, so we negate to get a higher-is-better score.
fn lex_to_ranked(h: fts::MemoryFtsHit) -> RankedHit {
    RankedHit {
        memory_id: h.memory_id,
        score: -h.score,
    }
}

/// Map a `vector::MemoryHit` directly to a [`RoutedHit`] tagged
/// `Source::Vector`. Used by the pure-vector path.
fn ann_to_routed(h: vector::MemoryHit) -> RoutedHit {
    RoutedHit {
        memory_id: h.memory_id,
        score: 1.0 - h.distance,
        source: Source::Vector,
    }
}

/// Map an `fts::MemoryFtsHit` directly to a [`RoutedHit`] tagged
/// `Source::Lexical`. Used by the pure-lexical path.
fn lex_to_routed(h: fts::MemoryFtsHit) -> RoutedHit {
    RoutedHit {
        memory_id: h.memory_id,
        score: -h.score,
        source: Source::Lexical,
    }
}
