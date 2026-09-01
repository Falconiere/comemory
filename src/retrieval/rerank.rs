//! Second retrieval stage: multiply the fused relevance score by bounded
//! deterministic priors (activation, feedback, quality, supersede, rank)
//! and expose every factor as [`ScoreParts`] for explainability.
//!
//! Consumes the [`RoutedHit`] list produced by [`crate::retrieval::router`]
//! and emits [`Reranked`] entries (carrying body + simhash) for the
//! diversify stage. All priors come from [`crate::retrieval::score`]; the
//! clamp and decay knobs come from `cfg.rank`.

use rusqlite::{Connection, OptionalExtension};
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::router::{RoutedHit, Source};
use crate::retrieval::score::{self, LegScores};

/// Scale for the memory PageRank boost: `1 + MEMORY_RANK_SCALE·ln(1 +
/// raw/median)`. A memory at the pool median maps to `1 + 0.2·ln 2 ≈ 1.14`
/// — matching `code_prior::RANK_SCALE`, so both graphs nudge ranking with
/// the same slope; `cfg.rank.prior_clamp` bounds the extremes.
pub const MEMORY_RANK_SCALE: f64 = 0.2;

/// Multiplicative factors behind a final score. Serialized verbatim into
/// `--json` output — a stable contract, not debug info. The invariant is
/// `final_score == f64::from(rrf) * activation * feedback * quality *
/// supersede * rank` (up to f32 rounding of the normalized value); `rrf` is
/// the pool-normalized relevance and every other field is a post-clamp
/// multiplier.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoreParts {
    /// Max-normalized relevance in `[0, 1]`: pool max maps to 1.0,
    /// within-pool ratios preserved; degenerate pools normalize to 1.0.
    /// The raw fused score is not exposed — normalization makes priors
    /// branch-stable.
    pub rrf: f32,
    /// ACT-R activation boost (post-clamp multiplier).
    pub activation: f64,
    /// Beta-smoothed feedback boost (post-clamp multiplier).
    pub feedback: f64,
    /// Frontmatter quality boost (post-clamp multiplier).
    pub quality: f64,
    /// [`score::SUPERSEDE_PENALTY`] when superseded by a live memory, else 1.0.
    pub supersede: f64,
    /// Memory-graph PageRank boost (post-clamp multiplier), relative to the
    /// candidate pool's median `memories.rank_score`. Exactly 1.0 while no
    /// [`crate::graph::memory_rank`] pass has run (every score still at the
    /// column default), and uniform across the pool on an edge-free corpus.
    pub rank: f64,
    /// Product of all factors.
    pub final_score: f64,
    /// The raw per-leg signals fusion consumed, carried through from the
    /// router. Additive: no factor above depends on it, and `final_score`
    /// is unchanged by its presence.
    pub legs: LegScores,
}

/// A reranked hit, ready for the diversity stage.
#[derive(Debug, Clone)]
pub struct Reranked {
    /// Identifier of the matched memory row.
    pub memory_id: String,
    /// Which retrieval branch produced the underlying candidate.
    pub source: Source,
    /// Lexical ladder tier of the underlying candidate (see
    /// [`RoutedHit::tier`]): 1 strict, 2 word-OR, 3 subtoken-OR,
    /// 4 learned expansion.
    pub tier: u8,
    /// Every multiplicative factor behind `parts.final_score`.
    pub parts: ScoreParts,
    /// Live memory that supersedes this one, if any.
    pub superseded_by: Option<String>,
    /// Body text, carried for MMR/SimHash in the diversify stage.
    pub body: String,
    /// SimHash of the body, carried for near-dup collapse.
    pub simhash: u64,
}

/// Rerank candidates by multiplying relevance with bounded priors, sorted
/// by descending `final_score` (ties break on ascending `memory_id` so the
/// order is fully deterministic). Hits whose memory row vanished or was
/// soft-deleted (raced delete) are silently dropped.
///
/// `as_of_cutoff` (set only by `--as-of`) scopes the supersede penalty to
/// superseders that already existed at that instant; `None` — including
/// under a plain `--until` — penalizes by the present-day corpus.
pub fn rerank(
    conn: &Connection,
    cfg: &Config,
    hits: &[RoutedHit],
    as_of_cutoff: Option<&str>,
) -> Result<Vec<Reranked>> {
    // Normalize the whole candidate pool up front, then zip — pairing is
    // established before any hit is dropped below, so a vanished memory
    // row cannot skew which norm belongs to which hit.
    let normalized: Vec<f64> =
        score::max_normalize(&hits.iter().map(|h| f64::from(h.score)).collect::<Vec<_>>());
    // Pass 1: fetch every candidate's signals, dropping the vanished rows.
    // The PageRank prior is pool-relative, so no candidate can be scored
    // until the whole surviving pool is known.
    let mut pool: Vec<(&RoutedHit, f64, Signals)> = Vec::with_capacity(hits.len());
    for (hit, norm) in hits.iter().zip(&normalized) {
        if let Some(sig) = memory_signals(conn, &hit.memory_id)? {
            pool.push((hit, *norm, sig));
        }
    }
    let ctx = PoolCtx {
        cfg,
        now: OffsetDateTime::now_utc(),
        as_of_cutoff,
        median_rank: pool_median_rank(pool.iter().map(|(_, _, s)| s.rank_score)),
    };
    // Pass 2: score each candidate against the pool-wide context.
    let mut out = Vec::with_capacity(pool.len());
    for (hit, norm, sig) in pool {
        out.push(score_hit(conn, &ctx, hit, norm, sig)?);
    }
    // `total_cmp` keeps the comparator a total order even if an upstream
    // stage ever leaks a NaN rrf score — `sort_by` panics on detected
    // ordering violations (Rust 1.81+), so a non-total comparator would
    // turn a bad score into a crash instead of a bad rank.
    out.sort_by(|a, b| {
        b.parts
            .final_score
            .total_cmp(&a.parts.final_score)
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });
    Ok(out)
}

/// Everything one rerank call shares across its candidates: the knobs, one
/// clock so the whole pool is judged against the same instant, the supersede
/// cutoff, and the pool-derived PageRank median.
struct PoolCtx<'a> {
    cfg: &'a Config,
    now: OffsetDateTime,
    as_of_cutoff: Option<&'a str>,
    median_rank: f64,
}

/// Median of the pool's DISTINCT stored `rank_score`s: exact-equal values
/// collapse first, so a pool crowded with identically-ranked memories cannot
/// drag the reference point away from the spread of scores actually present
/// (`code_prior::median_file_rank`'s distinct semantics, keyed by value here
/// because each candidate is already its own memory). Delegates the median
/// arithmetic to [`score::median_rank`].
fn pool_median_rank(ranks: impl IntoIterator<Item = f64>) -> f64 {
    let mut distinct: Vec<f64> = ranks.into_iter().collect();
    distinct.sort_by(f64::total_cmp);
    distinct.dedup();
    score::median_rank(&mut distinct)
}

/// Score one candidate: build every bounded prior from its already-fetched
/// signals and multiply them into `norm` (the pool-normalized relevance).
fn score_hit(
    conn: &Connection,
    ctx: &PoolCtx<'_>,
    hit: &RoutedHit,
    norm: f64,
    row: Signals,
) -> Result<Reranked> {
    let clamp = ctx.cfg.rank.prior_clamp;
    let days = score::days_since(&row.last_accessed, ctx.now);
    let act = score::activation(row.access_count, days, ctx.cfg.rank.decay);
    let beta = score::beta_feedback(row.used, row.irrelevant);
    let superseded_by = live_superseder(conn, &hit.memory_id, ctx.as_of_cutoff)?;
    let supersede = if superseded_by.is_some() {
        score::SUPERSEDE_PENALTY
    } else {
        1.0
    };
    let activation = score::activation_boost(act, clamp);
    let feedback = score::feedback_boost(beta, clamp);
    let quality = score::quality_boost(row.quality, clamp);
    let rank = score::rank_boost(row.rank_score, ctx.median_rank, MEMORY_RANK_SCALE, clamp);
    let final_score = norm * activation * feedback * quality * supersede * rank;
    Ok(Reranked {
        memory_id: hit.memory_id.clone(),
        source: hit.source,
        tier: hit.tier,
        parts: ScoreParts {
            rrf: norm as f32,
            activation,
            feedback,
            quality,
            supersede,
            rank,
            final_score,
            legs: hit.legs,
        },
        superseded_by,
        body: row.body,
        simhash: row.simhash,
    })
}

/// Per-memory ranking signals pulled in one query: row metadata plus the
/// (optional) feedback counters, with `COALESCE` neutralizing absent rows.
struct Signals {
    quality: u8,
    access_count: u64,
    last_accessed: String,
    body: String,
    simhash: u64,
    used: u64,
    irrelevant: u64,
    rank_score: f64,
}

/// Fetch the ranking signals for one live memory. Returns `Ok(None)` when
/// the row does not exist or is soft-deleted. The statement is
/// `prepare_cached` so the per-hit loop in [`rerank`] reuses one prepared
/// statement instead of re-parsing the SQL for every candidate.
fn memory_signals(conn: &Connection, id: &str) -> Result<Option<Signals>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.quality, m.access_count, COALESCE(m.last_accessed, m.created_at),
                m.body, m.simhash,
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0),
                m.rank_score
           FROM memories m
           LEFT JOIN feedback f ON f.memory_id = m.id
          WHERE m.id = ?1 AND m.deleted_at IS NULL",
    )?;
    stmt.query_row([id], |r| {
        Ok(Signals {
            quality: r.get(0)?,
            access_count: r.get::<_, i64>(1)?.max(0) as u64,
            last_accessed: r.get(2)?,
            body: r.get(3)?,
            simhash: r.get::<_, i64>(4)? as u64,
            used: r.get::<_, i64>(5)?.max(0) as u64,
            irrelevant: r.get::<_, i64>(6)?.max(0) as u64,
            rank_score: r.get(7)?,
        })
    })
    .optional()
    .map_err(Error::from)
}

/// Find the *live* memory that supersedes `id`, if any — earliest by
/// `created_at` (ties on `id`) so repeated replacements report one stable
/// superseder. Soft-deleted superseders don't count; self-edges ignored as
/// defense-in-depth. `prepare_cached` per [`memory_signals`].
///
/// `as_of_cutoff` bounds the **superseder's own `created_at`**, never
/// `edges.created_at` — rebuild re-stamps edges, frontmatter `created`
/// survives. The cutoff is `memory_row::iso_format` output; that
/// formatter ↔ SQLite `datetime()` contract is pinned by the
/// mixed-precision store tests and the `--as-of` CLI e2e, so a format
/// drift fails loudly, not silently.
///
/// `pub(crate)` so `api::show` reuses the exact same join for its
/// `superseded_by` field instead of re-deriving it (Binding Rule 1).
pub(crate) fn live_superseder(
    conn: &Connection,
    id: &str,
    as_of_cutoff: Option<&str>,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT e.src_id FROM edges e
           JOIN memories m ON m.id = e.src_id AND m.deleted_at IS NULL
          WHERE e.rel = 'supersedes'
            AND e.src_kind = 'memory' AND e.dst_kind = 'memory' AND e.dst_id = ?1
            AND e.src_id <> e.dst_id
            AND (?2 IS NULL OR datetime(m.created_at) <= datetime(?2))
          ORDER BY datetime(m.created_at) ASC, m.id ASC
          LIMIT 1",
    )?;
    stmt.query_row(rusqlite::params![id, as_of_cutoff], |r| r.get(0))
        .optional()
        .map_err(Error::from)
}

#[cfg(test)]
#[path = "tests/rerank.rs"]
mod tests;
