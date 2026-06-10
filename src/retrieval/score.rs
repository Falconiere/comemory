//! Deterministic scoring primitives: ACT-R activation (Petrov
//! approximation), Beta-smoothed feedback, and the bounded multiplier
//! mappings used by the rerank stage. Pure functions — time and counts
//! come in as arguments so tests stay clock-free.

/// ACT-R base-level activation, Petrov approximation:
/// `ln(max(n,1)) − d·ln(max(days,0) + 1)`. Time is measured in days; the
/// `+ 1` keeps the value finite for same-day access.
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
/// `(used + 1) / (used + irrelevant + 4)`. Zero feedback → 0.25.
///
/// Uses `saturating_add` before the cast to avoid wrapping on very large
/// combined counts, keeping the function total for all `u64` inputs.
pub fn beta_feedback(used: u64, irrelevant: u64) -> f64 {
    (used as f64 + 1.0) / (used.saturating_add(irrelevant) as f64 + 4.0)
}

/// Map activation to a bounded multiplier; activation 0 → 1.0.
pub fn activation_boost(activation: f64, clamp: (f64, f64)) -> f64 {
    bounded((0.2 * activation).exp(), clamp)
}

/// Map Beta feedback to a bounded multiplier; the 0.25 neutral point → 1.0.
pub fn feedback_boost(beta: f64, clamp: (f64, f64)) -> f64 {
    bounded(beta / 0.25, clamp)
}

/// Map quality 1..=5 to a bounded multiplier; quality 3 → 1.0.
pub fn quality_boost(quality: u8, clamp: (f64, f64)) -> f64 {
    bounded(1.0 + 0.075 * (f64::from(quality) - 3.0), clamp)
}

/// Fixed multiplier applied to results superseded by a live memory.
pub const SUPERSEDE_PENALTY: f64 = 0.2;

fn bounded(v: f64, (lo, hi): (f64, f64)) -> f64 {
    if !v.is_finite() {
        return 1.0;
    }
    v.max(lo).min(hi)
}
