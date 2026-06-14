//! Routes a query to the vector path, the lexical path, or the hybrid
//! path, and returns a uniform [`RoutedHit`] list.
//!
//! Decision table:
//! - vec = Some, query non-empty → **hybrid**: run both ANN and FTS5 BM25
//!   independently, fuse via RRF, truncate to the candidate pool size.
//!   This is the correct path when the caller supplies both a semantic
//!   vector *and* a text query.
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

/// Candidate pool fed to the rerank stage; the pipeline cuts to top_k
/// after diversification.
pub const CANDIDATE_POOL: usize = 50;

/// [`RoutedHit::tier`] value of the learned-expansion ladder tier — the
/// only tier output rendering treats specially (the `[expanded]` flag in
/// `output::search`). The JSON contract serializes `tier` as a bare u8
/// (1..=4), so a full enum is deliberately not used.
pub const TIER_EXPANDED: u8 = 4;

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
    /// Lexical ladder tier that produced the candidate set: 1 strict
    /// (also the vector/hybrid default), 2 word-OR, 3 subtoken-OR,
    /// 4 learned expansion. Hybrid fused hits carry the lexical leg's
    /// ladder tier when the ladder fired.
    pub tier: u8,
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
///
/// The fetch size is the caller-supplied `pool` (see
/// [`crate::retrieval::pipeline::pool_size`], which clamps it into
/// `[CANDIDATE_POOL, max_page_window]` for the requested page): `route`
/// produces a candidate pool for the rerank + diversify stages, which
/// perform the final paginated cut. When the strict lexical
/// leg comes back empty (in either the pure-lexical or the hybrid path),
/// the relaxed ladder in [`lexical_ladder`] retries it: a word-level OR
/// tier (queries with ≥ 2 terms) so a single absent term cannot zero out
/// the result set, then an identifier-subtoken OR tier so an identifier
/// query like `VecDimMismatch` can still reach prose that only mentions
/// its parts, and finally a learned-expansion tier that ORs in mined
/// reformulation mappings from `query_expansions`. ANN hits below
/// `cfg.retrieval.memory_threshold` cosine
/// similarity are dropped before use in both vector-consuming paths.
///
/// `kind` restricts every path to one memory kind (canonical lowercase
/// string, e.g. `decision`): the lexical legs filter in SQL via the
/// `memories` join, while the ANN legs post-filter through [`filter_kind`]
/// because vec0 cannot filter inside the KNN query.
pub fn route(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: Option<&[f32]>,
    repo: Option<&str>,
    kind: Option<&str>,
    pool: usize,
) -> Result<Vec<RoutedHit>> {
    let k = pool;

    // Trim the query before dispatching: a whitespace-only query like
    // `"   "` is lexically empty (FTS5 returns no rows for it) so the
    // hybrid arm would mislabel a vector-only result as `Source::Hybrid`
    // and downstream consumers would assume lexical contributed signal.
    let lex_meaningful = !query.trim().is_empty();
    match vec {
        Some(v) if lex_meaningful => route_hybrid(cfg, conn, query, v, k, repo, kind),
        Some(v) => route_vector_only(cfg, conn, v, k, repo, kind),
        None => route_lexical(conn, query, k, repo, kind, cfg.retrieval.bm25_weights),
    }
}

/// Hybrid path: run ANN + FTS5 independently and fuse via RRF.
///
/// The lexical leg goes through the same [`strict_then_ladder`] policy
/// as the pure-lexical path, *before* fusion — otherwise a noisy ANN
/// leg would suppress the fallback entirely (the fused result is non-empty,
/// so memories reachable only via the relaxed/subtoken tiers would never
/// surface). When the ANN leg contributes nothing (empty vector table, or
/// every hit below the similarity threshold), the result is tagged
/// [`Source::Lexical`] because only the FTS branch produced signal.
/// Fused hits carry the lexical leg's tier (1 when the leg was empty —
/// the strict default).
///
/// The `kind` filter is applied to the ANN leg *before* fusion (the
/// lexical leg already filters in SQL) so RRF only ranks eligible
/// candidates — a wrong-kind ANN hit must not eat a fusion rank slot.
fn route_hybrid(
    cfg: &Config,
    conn: &Connection,
    query: &str,
    vec: &[f32],
    k: usize,
    repo: Option<&str>,
    kind: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let ann = filter_kind(
        conn,
        above_similarity_threshold(
            vector::knn_memory(conn, vec, k, repo)?,
            |h| h.distance,
            cfg.retrieval.memory_threshold,
        ),
        kind,
    )?;
    let weights = cfg.retrieval.bm25_weights;
    let (lex, lex_tier) = strict_then_ladder(conn, query, k, repo, kind, weights)?;
    if ann.is_empty() {
        return Ok(lex
            .into_iter()
            .map(|h| lex_to_routed(h, lex_tier))
            .collect());
    }

    let ann_ranked: Vec<RankedHit> = ann.into_iter().map(ann_to_ranked).collect();
    let lex_ranked: Vec<RankedHit> = lex.into_iter().map(lex_to_ranked).collect();

    let fused = fuse::rrf_k(&ann_ranked, &lex_ranked, k, cfg.retrieval.rrf_k);
    Ok(fused
        .into_iter()
        .map(|h| RoutedHit {
            memory_id: h.memory_id,
            score: h.score,
            source: Source::Hybrid,
            tier: lex_tier,
        })
        .collect())
}

/// Pure-vector path. The lexical top-up that previously lived here was
/// dead: this arm is only reached when `query` is empty (the dispatcher
/// routes `vec + non-empty query` to [`route_hybrid`]), and FTS5 BM25
/// returns no rows for an empty query. Callers that want sparse+dense
/// fusion must pass a non-empty `query` so the hybrid arm fires. Hits
/// below `cfg.retrieval.memory_threshold` are dropped — KNN always
/// returns the k nearest rows no matter how far away they are. The `kind`
/// filter runs post-threshold via [`filter_kind`].
fn route_vector_only(
    cfg: &Config,
    conn: &Connection,
    vec: &[f32],
    k: usize,
    repo: Option<&str>,
    kind: Option<&str>,
) -> Result<Vec<RoutedHit>> {
    let ann = filter_kind(
        conn,
        above_similarity_threshold(
            vector::knn_memory(conn, vec, k, repo)?,
            |h| h.distance,
            cfg.retrieval.memory_threshold,
        ),
        kind,
    )?;
    Ok(ann.into_iter().map(ann_to_routed).collect())
}

/// Batch kind filter for ANN-routed hits: the vec0 KNN leg cannot filter
/// by kind inside the virtual-table query, so non-matching ids are dropped
/// afterwards in one `IN` query against `memories`. The lexical legs
/// filter in SQL via the `memories` join and must NOT pass through here —
/// double-filtering would cost a redundant round trip.
fn filter_kind(
    conn: &Connection,
    hits: Vec<vector::MemoryHit>,
    kind: Option<&str>,
) -> Result<Vec<vector::MemoryHit>> {
    let Some(kind) = kind else { return Ok(hits) };
    if hits.is_empty() {
        return Ok(hits);
    }
    let qmarks = crate::store::qmarks(hits.len());
    let sql = format!("SELECT id FROM memories WHERE kind = ? AND id IN ({qmarks})");
    let mut stmt = conn.prepare(&sql)?;
    let params = std::iter::once(kind).chain(hits.iter().map(|h| h.memory_id.as_str()));
    let keep: std::collections::HashSet<String> = stmt
        .query_map(rusqlite::params_from_iter(params), |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(hits
        .into_iter()
        .filter(|h| keep.contains(&h.memory_id))
        .collect())
}

/// Drop ANN hits whose cosine similarity (`1.0 - distance(hit)`) falls
/// below `threshold`. vec0 KNN always returns the k nearest rows
/// regardless of distance, so without this floor a query vector far from
/// the whole corpus pads the candidate pool with k nearest-but-irrelevant
/// noise hits. Shared by the memory ANN floor here
/// (`cfg.retrieval.memory_threshold`, default 0.55) and the code ANN
/// floor in [`crate::retrieval::code_route`]
/// (`cfg.retrieval.code_threshold`, default 0.50), generic over the hit
/// type since the two legs return different row structs.
pub(crate) fn above_similarity_threshold<H>(
    hits: Vec<H>,
    distance: impl Fn(&H) -> f32,
    threshold: f32,
) -> Vec<H> {
    hits.into_iter()
        .filter(|h| (1.0 - distance(h)) >= threshold)
        .collect()
}

/// Pure-lexical path via FTS5 BM25, with the relaxed fallback ladder.
/// `weights` is `cfg.retrieval.bm25_weights`, threaded explicitly because
/// this arm needs no other config.
fn route_lexical(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
    kind: Option<&str>,
    weights: (f32, f32),
) -> Result<Vec<RoutedHit>> {
    let (lex, tier) = strict_then_ladder(conn, query, k, repo, kind, weights)?;
    Ok(lex.into_iter().map(|h| lex_to_routed(h, tier)).collect())
}

/// The full lexical policy shared by [`route_lexical`] and the hybrid
/// arm's FTS leg: run the strict AND tier first (tier 1 on hits), then
/// walk the relaxed [`lexical_ladder`] when it found nothing. Returning
/// the tier alongside the hits keeps both callers free of fallback
/// bookkeeping, so they cannot drift on when the ladder fires.
fn strict_then_ladder(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
    kind: Option<&str>,
    weights: (f32, f32),
) -> Result<(Vec<fts::MemoryFtsHit>, u8)> {
    let lex = fts::search_memory(conn, query, k, repo, kind, weights)?;
    if !lex.is_empty() {
        return Ok((lex, 1));
    }
    lexical_ladder(conn, query, k, repo, kind, weights)
}

/// Relaxed fallback ladder shared by the lexical and hybrid paths, walked
/// only when the strict AND query found nothing:
///
/// 1. **Word-level OR** ([`fts::search_memory_relaxed`]) — only when the
///    query has at least two *sanitized* terms ([`fts::term_count`] — the
///    same terms the MATCH builders quote, so the guard and the builders
///    cannot disagree on what counts as a term). OR of one term is the
///    same query as the strict tier, so retrying would only waste a round
///    trip.
/// 2. **Subtoken OR** ([`fts::search_memory_subtokens`]) — when the word
///    tier was skipped or empty, OR the identifier sub-tokens of each term
///    so a prose body mentioning `dim mismatch` is still reachable from
///    the identifier query `VecDimMismatch`. Queries with no splittable
///    term build an empty expression and stay empty.
/// 3. **Learned expansion** ([`fts::search_memory_expanded`]) — when the
///    subtoken tier is also empty, OR each term with its mined
///    reformulation expansions (`query_expansions`, support ≥
///    [`fts::EXPANSION_MIN_SUPPORT`]). Queries with no applicable
///    expansion build an empty expression and stay empty.
///
/// The ladder only *adds* recall on previously-empty results — a non-empty
/// earlier tier always short-circuits, so it can never reorder hits the
/// stricter tiers already produced.
///
/// Returns the hits plus the [`RoutedHit::tier`] that produced them
/// (word-OR → 2, subtoken → 3, expansion → [`TIER_EXPANDED`]). When
/// every tier comes back empty the returned tier is 1 — the strict
/// default — so an empty ladder leaves the hybrid arm's fused
/// (ANN-only) hits tagged exactly like a strict run.
fn lexical_ladder(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
    kind: Option<&str>,
    weights: (f32, f32),
) -> Result<(Vec<fts::MemoryFtsHit>, u8)> {
    if fts::term_count(query) >= 2 {
        let lex = fts::search_memory_relaxed(conn, query, k, repo, kind, weights)?;
        if !lex.is_empty() {
            return Ok((lex, 2));
        }
    }
    let lex = fts::search_memory_subtokens(conn, query, k, repo, kind, weights)?;
    if !lex.is_empty() {
        return Ok((lex, 3));
    }
    let lex = fts::search_memory_expanded(conn, query, k, repo, kind, weights)?;
    if !lex.is_empty() {
        return Ok((lex, TIER_EXPANDED));
    }
    Ok((Vec::new(), 1))
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
/// `Source::Vector` at the default tier 1. Used by the pure-vector path.
fn ann_to_routed(h: vector::MemoryHit) -> RoutedHit {
    RoutedHit {
        memory_id: h.memory_id,
        score: 1.0 - h.distance,
        source: Source::Vector,
        tier: 1,
    }
}

/// Map an `fts::MemoryFtsHit` directly to a [`RoutedHit`] tagged
/// `Source::Lexical`, carrying the ladder `tier` that produced it (1 when
/// the strict query hit). Used by the pure-lexical and lex-only-hybrid
/// paths.
fn lex_to_routed(h: fts::MemoryFtsHit, tier: u8) -> RoutedHit {
    RoutedHit {
        memory_id: h.memory_id,
        score: -h.score,
        source: Source::Lexical,
        tier,
    }
}
