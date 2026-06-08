//! Routes a query to the vector path, the lexical path, or both
//! (hybrid) and returns a uniform [`RoutedHit`] list.
//!
//! When a caller supplies a vector, the router runs sqlite-vec ANN over
//! `memory_vec`; if fewer than `top_k` rows survive, it tops up with
//! FTS5 BM25 hits over `memory_fts` (the corrective fallback). With no
//! vector the router goes straight to the lexical path.

use rusqlite::Connection;

use crate::config::Config;
use crate::prelude::*;
use crate::store::{fts, vector};

/// One unified retrieval hit, regardless of which branch produced it.
#[derive(Debug, Clone)]
pub struct RoutedHit {
    /// Identifier of the matched memory row.
    pub memory_id: String,
    /// Higher-is-better score. Vector hits use `1.0 - distance`; lexical
    /// hits use `-bm25` so a smaller BM25 magnitude lifts toward the top.
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
}

/// Run the retrieval pipeline for `query`.
///
/// When `vec` is `Some`, ANN runs first; if the ANN result is shorter
/// than `top_k`, the router tops up with lexical hits not already in
/// the list. When `vec` is `None`, only the lexical path runs.
pub fn route(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let k = cfg.retrieval.top_k;
    if let Some(v) = vec {
        let ann = vector::knn_memory(conn, v, k, repo)?;
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
        // Corrective top-up with lexical hits.
        let mut routed: Vec<RoutedHit> = ann
            .into_iter()
            .map(|h| RoutedHit {
                memory_id: h.memory_id,
                score: 1.0 - h.distance,
                source: Source::Vector,
            })
            .collect();
        let need = k.saturating_sub(routed.len());
        if need > 0 {
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
        return Ok(routed);
    }
    // Pure lexical path.
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
