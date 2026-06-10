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

fn bounded(v: f64, (lo, hi): (f64, f64)) -> f64 {
    if !v.is_finite() {
        return 1.0f64.max(lo).min(hi);
    }
    v.max(lo).min(hi)
}
