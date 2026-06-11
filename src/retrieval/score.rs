//! Deterministic scoring primitives: ACT-R activation (Petrov
//! approximation), Beta-smoothed feedback, the bounded multiplier
//! mappings used by the rerank stage, and the shared timestamp→days
//! helper. Pure functions — time and counts come in as arguments so
//! tests stay clock-free.

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// ACT-R base-level activation, Petrov approximation:
/// `ln(max(n,1)) − d·ln(max(days,0) + 1)`. Time is measured in days; the
/// `+ 1` keeps the value finite for same-day access.
///
/// `decay` must be ≥ 0 (validated by `RankConfig`); negative decay would
/// invert time behavior, making older memories score higher.
pub fn activation(access_count: u64, days_since_access: f64, decay: f64) -> f64 {
    let n = access_count.max(1) as f64;
    let days = if days_since_access.is_finite() {
        days_since_access.max(0.0)
    } else {
        0.0
    };
    n.ln() - decay * (days + 1.0).ln()
}

/// Posterior mean of Beta(1, 3) prior over used/irrelevant feedback:
/// `(used + 1) / (used + irrelevant + 4)`. Zero feedback →
/// [`FEEDBACK_NEUTRAL`].
///
/// Uses `saturating_add` before the cast to avoid wrapping on very large
/// combined counts, keeping the function total for all `u64` inputs.
pub fn beta_feedback(used: u64, irrelevant: u64) -> f64 {
    (used as f64 + 1.0) / (used.saturating_add(irrelevant) as f64 + 4.0)
}

/// Beta(1, 3) posterior mean at zero feedback — the neutral point that
/// [`feedback_boost`] maps to a 1.0 multiplier.
pub const FEEDBACK_NEUTRAL: f64 = 0.25;

/// Map activation to a bounded multiplier; activation 0 → 1.0.
///
/// The 0.2 scale keeps `exp(0.2·a)` gentle: one decade of activation
/// (±5) spans roughly 0.37x..2.7x before clamping.
///
/// `clamp` must satisfy `lo <= hi` (validated by `RankConfig`).
pub fn activation_boost(activation: f64, clamp: (f64, f64)) -> f64 {
    bounded((0.2 * activation).exp(), clamp)
}

/// Map Beta feedback to a bounded multiplier; the [`FEEDBACK_NEUTRAL`]
/// point → 1.0.
///
/// `clamp` must satisfy `lo <= hi` (validated by `RankConfig`).
pub fn feedback_boost(beta: f64, clamp: (f64, f64)) -> f64 {
    bounded(beta / FEEDBACK_NEUTRAL, clamp)
}

/// Map quality 1..=5 to a bounded multiplier; quality 3 → 1.0.
///
/// The 0.075 slope spans 0.85x..1.15x across quality 1..5, a deliberate
/// nudge rather than a dominant factor.
///
/// `clamp` must satisfy `lo <= hi` (validated by `RankConfig`).
pub fn quality_boost(quality: u8, clamp: (f64, f64)) -> f64 {
    bounded(1.0 + 0.075 * (f64::from(quality) - 3.0), clamp)
}

/// Fixed multiplier applied to results superseded by a live memory.
///
/// Intentionally bypasses `prior_clamp`: a supersede is a penalty stronger
/// than any prior and must NOT be run through `bounded()`.
pub const SUPERSEDE_PENALTY: f64 = 0.2;

/// Whole days elapsed between an RFC 3339 timestamp and `now`, floored at
/// zero. All timestamp writers (`memory_row::iso_format` — shared by save,
/// rebuild, and `pipeline::record_access` — plus the SQLite
/// `strftime('%Y-%m-%dT%H:%M:%fZ', ...)` upsert arm) emit RFC 3339-parseable
/// strings. An unparsable timestamp is treated as fresh — never punish a
/// memory for a malformed clock value — but it is logged: a value that
/// fails to parse means a writer bug or row corruption, and silently
/// scoring it as fresh would mask that. Shared by `retrieval::rerank` and
/// `prune::low_value` so the two consumers cannot drift on day math.
pub fn days_since(rfc3339: &str, now: OffsetDateTime) -> f64 {
    match OffsetDateTime::parse(rfc3339, &Rfc3339) {
        Ok(then) => ((now - then).whole_seconds() as f64 / 86_400.0).max(0.0),
        Err(e) => {
            tracing::warn!(
                timestamp = rfc3339,
                error = %e,
                "score: malformed last_accessed/created_at; scoring as fresh"
            );
            0.0
        }
    }
}

/// Min-max normalize a score pool into `[0, 1]`.
///
/// Used by MMR selection in `diversify`: the relevance term must be
/// commensurate with the `[0, 1]` Jaccard diversity term, and the pool
/// mixes wildly different scales per routing branch — RRF fusion yields
/// scores around `1/k ≈ 0.016` while the pure-lexical branch carries
/// `-bm25` values anywhere from `1e-6` to `10+`. Min-max is
/// affine-invariant, so normalization is insensitive to the absolute
/// score scale. Degenerate pools — empty, all-equal, or containing
/// non-finite values — normalize to all `1.0` so downstream ordering
/// falls back to the respective tie-breaks.
///
/// NOT used by the rerank stage: min-max zeroes the pool minimum
/// (making it immune to priors) and stretches near-tie gaps to the full
/// range, which would let tiny bm25 differences drown feedback. Rerank
/// uses [`max_normalize`] instead.
pub fn min_max_normalize(scores: &[f64]) -> Vec<f64> {
    let min = scores.iter().copied().fold(f64::INFINITY, f64::min);
    let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    if !range.is_finite() || range <= 0.0 {
        return vec![1.0; scores.len()];
    }
    scores.iter().map(|s| (s - min) / range).collect()
}

/// Normalize a relevance pool by its maximum, preserving within-pool
/// ratios: the best candidate maps to 1.0 and a candidate half as
/// relevant maps to 0.5, so near-ties stay near-ties (a bounded prior
/// can reorder them) while large relevance gaps stay large (priors
/// cannot drown real relevance). Used by the rerank stage; MMR uses
/// [`min_max_normalize`] instead because its relevance term must share
/// the diversity term's [0, 1] range.
///
/// Real candidate scores are positive (lexical `-bm25` with FTS5's
/// bm25 <= 0, RRF sums, threshold-filtered cosine similarity), so the
/// degenerate arms are defense-in-depth: a pool containing a non-finite
/// score, or whose max is zero or negative, normalizes to all 1.0
/// (`f64::max` ignores NaN, so non-finiteness is checked per element,
/// not on the fold result), and a stray negative score inside a
/// positive pool clamps to 0.0 (noise stays noise).
pub fn max_normalize(scores: &[f64]) -> Vec<f64> {
    let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !max.is_finite() || max <= 0.0 || scores.iter().any(|s| !s.is_finite()) {
        return vec![1.0; scores.len()];
    }
    scores.iter().map(|s| (s / max).clamp(0.0, 1.0)).collect()
}

fn bounded(v: f64, (lo, hi): (f64, f64)) -> f64 {
    if !v.is_finite() {
        return 1.0f64.max(lo).min(hi);
    }
    v.max(lo).min(hi)
}
