//! Second retrieval stage: multiply the fused relevance score by bounded
//! deterministic priors (activation, feedback, quality, supersede) and
//! expose every factor as [`ScoreParts`] for explainability.
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
use crate::retrieval::score;

/// Multiplicative factors behind a final score. Serialized verbatim into
/// `--json` output — a stable contract, not debug info. The invariant is
/// `final_score == f64::from(rrf) * activation * feedback * quality *
/// supersede`; every field except `rrf` is a post-clamp multiplier.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoreParts {
    /// Fused relevance from the candidate stage (RRF / lexical / vector).
    pub rrf: f32,
    /// ACT-R activation boost (post-clamp multiplier).
    pub activation: f64,
    /// Beta-smoothed feedback boost (post-clamp multiplier).
    pub feedback: f64,
    /// Frontmatter quality boost (post-clamp multiplier).
    pub quality: f64,
    /// [`score::SUPERSEDE_PENALTY`] when superseded by a live memory, else 1.0.
    pub supersede: f64,
    /// Product of all factors.
    pub final_score: f64,
}

/// A reranked hit, ready for the diversity stage.
#[derive(Debug, Clone)]
pub struct Reranked {
    /// Identifier of the matched memory row.
    pub memory_id: String,
    /// Which retrieval branch produced the underlying candidate.
    pub source: Source,
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
pub fn rerank(conn: &Connection, cfg: &Config, hits: &[RoutedHit]) -> Result<Vec<Reranked>> {
    let now = OffsetDateTime::now_utc();
    let clamp = cfg.rank.prior_clamp;
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        let Some(row) = memory_signals(conn, &hit.memory_id)? else {
            continue;
        };
        let days = score::days_since(&row.last_accessed, now);
        let act = score::activation(row.access_count, days, cfg.rank.decay);
        let beta = score::beta_feedback(row.used, row.irrelevant);
        let superseded_by = live_superseder(conn, &hit.memory_id)?;
        let supersede = if superseded_by.is_some() {
            score::SUPERSEDE_PENALTY
        } else {
            1.0
        };
        let activation_boost = score::activation_boost(act, clamp);
        let feedback_boost = score::feedback_boost(beta, clamp);
        let quality_boost = score::quality_boost(row.quality, clamp);
        // A negative rrf (pure-vector cosine distance > 1) inverts the boost
        // direction; the clamps keep it finite — acceptable until M2
        // normalizes the candidate scores.
        let final_score =
            f64::from(hit.score) * activation_boost * feedback_boost * quality_boost * supersede;
        out.push(Reranked {
            memory_id: hit.memory_id.clone(),
            source: hit.source,
            parts: ScoreParts {
                rrf: hit.score,
                activation: activation_boost,
                feedback: feedback_boost,
                quality: quality_boost,
                supersede,
                final_score,
            },
            superseded_by,
            body: row.body,
            simhash: row.simhash,
        });
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
}

/// Fetch the ranking signals for one live memory. Returns `Ok(None)` when
/// the row does not exist or is soft-deleted. The statement is
/// `prepare_cached` so the per-hit loop in [`rerank`] reuses one prepared
/// statement instead of re-parsing the SQL for every candidate.
fn memory_signals(conn: &Connection, id: &str) -> Result<Option<Signals>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.quality, m.access_count, COALESCE(m.last_accessed, m.created_at),
                m.body, m.simhash,
                COALESCE(f.used_count, 0), COALESCE(f.irrelevant_count, 0)
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
        })
    })
    .optional()
    .map_err(Error::from)
}

/// Find a *live* memory that supersedes `id`, if any. Edges from
/// soft-deleted memories don't count: a deleted superseder must not keep
/// punishing the memory it once replaced. Self-edges (`src_id = dst_id`)
/// are ignored as defense-in-depth — the writers refuse to create them,
/// but a hand-seeded cycle must not permanently penalize its own memory.
/// `prepare_cached` for the same per-hit-loop reason as [`memory_signals`].
fn live_superseder(conn: &Connection, id: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT e.src_id FROM edges e
           JOIN memories m ON m.id = e.src_id AND m.deleted_at IS NULL
          WHERE e.rel = 'supersedes'
            AND e.src_kind = 'memory' AND e.dst_kind = 'memory' AND e.dst_id = ?1
            AND e.src_id <> e.dst_id
          LIMIT 1",
    )?;
    stmt.query_row([id], |r| r.get(0))
        .optional()
        .map_err(Error::from)
}
