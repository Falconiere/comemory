//! Candidate stage for code search: weighted BM25 over `code_fts`,
//! optional BYO-vector ANN over `code_vec` floored by
//! `cfg.retrieval.code_threshold`, RRF-fused when both legs are present.
//! Mirrors [`crate::retrieval::router`] (the memory side); there is no
//! relaxation ladder — the identifier tokenizer already splits code
//! tokens, so the strict tier reaches subtoken matches directly.

use rusqlite::Connection;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::fuse::{self, RankedHit};
use crate::retrieval::router::{Source, CANDIDATE_POOL};
use crate::store::{fts, vector};

/// One unified code-retrieval hit, regardless of which branch produced it.
#[derive(Debug, Clone)]
pub struct CodeRoutedHit {
    /// `code_symbols.id` of the matched row (may be a cAST chunk row).
    pub symbol_id: i64,
    /// Higher-is-better candidate score (same conventions as the memory
    /// side: vector hits use `1.0 - distance`, lexical hits use `-bm25`,
    /// hybrid hits carry the RRF fused score).
    pub score: f32,
    /// Which branch produced the hit.
    pub source: Source,
}

/// Route a code query: lexical BM25 (whenever the query is non-empty),
/// ANN (whenever a vector is supplied), RRF fusion when both contribute.
///
/// The fetch size is [`CANDIDATE_POOL`] (or `top_k` when configured
/// larger) — this stage produces a candidate pool, not the final cut.
/// Both legs share the same `repo` / `lang` scope: the lexical leg
/// filters in SQL via the `code_symbols` JOIN while the ANN leg
/// post-filters inside [`vector::knn_code`]. ANN hits below
/// `cfg.retrieval.code_threshold` cosine similarity are dropped before
/// use, mirroring the memory router's threshold floor.
///
/// A whitespace-only query is lexically empty (FTS5 returns nothing for
/// it), so `vector + blank query` routes pure-ANN and `no vector + blank
/// query` returns empty — the same dispatch the memory router uses.
pub fn route_code(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
    lang: Option<&str>,
) -> Result<Vec<CodeRoutedHit>> {
    let k = CANDIDATE_POOL.max(cfg.retrieval.top_k);
    let lex = if query.trim().is_empty() {
        Vec::new()
    } else {
        fts::search_code(conn, query, k, repo, lang, cfg.retrieval.code_bm25_weights)?
    };
    let ann = match vec {
        Some(v) => above_code_threshold(
            vector::knn_code(conn, v, k, repo, lang)?,
            cfg.retrieval.code_threshold,
        ),
        None => Vec::new(),
    };
    Ok(match (ann.is_empty(), lex.is_empty()) {
        (true, true) => Vec::new(),
        (false, true) => ann.into_iter().map(ann_to_hit).collect(),
        (true, false) => lex.into_iter().map(lex_to_hit).collect(),
        (false, false) => fuse_legs(ann, lex, k, cfg.retrieval.rrf_k),
    })
}

/// Drop ANN hits whose cosine similarity (`1.0 - distance`) falls below
/// `threshold` (`cfg.retrieval.code_threshold`, default 0.50). vec0 KNN
/// always returns the k nearest rows regardless of distance, so without
/// this floor a query vector far from the whole corpus pads the candidate
/// pool with nearest-but-irrelevant noise — the code-side sibling of the
/// memory router's `above_memory_threshold`.
fn above_code_threshold(hits: Vec<vector::CodeHit>, threshold: f32) -> Vec<vector::CodeHit> {
    hits.into_iter()
        .filter(|h| (1.0 - h.distance) >= threshold)
        .collect()
}

/// RRF-fuse the ANN and lexical legs and tag the result [`Source::Hybrid`].
///
/// [`fuse::rrf_k`] is id-type-agnostic by string (`RankedHit.memory_id`),
/// so the i64 symbol ids are stringified for fusion and parsed back
/// afterwards — pragmatic, and cheaper than generalizing `fuse` over the
/// id type for one caller. The parse-back cannot fail (every id went
/// through `to_string` above); a failure would mean `fuse` invented an id,
/// so it is skipped defensively with a warning rather than unwrapped.
fn fuse_legs(
    ann: Vec<vector::CodeHit>,
    lex: Vec<fts::CodeFtsHit>,
    k: usize,
    rrf_k: f32,
) -> Vec<CodeRoutedHit> {
    let ann_ranked: Vec<RankedHit> = ann
        .into_iter()
        .map(|h| RankedHit {
            memory_id: h.symbol_id.to_string(),
            score: 1.0 - h.distance,
        })
        .collect();
    let lex_ranked: Vec<RankedHit> = lex
        .into_iter()
        .map(|h| RankedHit {
            memory_id: h.symbol_id.to_string(),
            score: -h.score,
        })
        .collect();
    fuse::rrf_k(&ann_ranked, &lex_ranked, k, rrf_k)
        .into_iter()
        .filter_map(|h| match h.memory_id.parse::<i64>() {
            Ok(symbol_id) => Some(CodeRoutedHit {
                symbol_id,
                score: h.score,
                source: Source::Hybrid,
            }),
            Err(e) => {
                tracing::warn!(id = %h.memory_id, error = %e, "skipping non-numeric fused code id");
                None
            }
        })
        .collect()
}

/// Map a [`vector::CodeHit`] to a [`CodeRoutedHit`] tagged
/// [`Source::Vector`]; distance becomes the higher-is-better
/// `1.0 - distance` cosine similarity.
fn ann_to_hit(h: vector::CodeHit) -> CodeRoutedHit {
    CodeRoutedHit {
        symbol_id: h.symbol_id,
        score: 1.0 - h.distance,
        source: Source::Vector,
    }
}

/// Map an [`fts::CodeFtsHit`] to a [`CodeRoutedHit`] tagged
/// [`Source::Lexical`]; BM25 is lower-is-better, so it is negated.
fn lex_to_hit(h: fts::CodeFtsHit) -> CodeRoutedHit {
    CodeRoutedHit {
        symbol_id: h.symbol_id,
        score: -h.score,
        source: Source::Lexical,
    }
}
