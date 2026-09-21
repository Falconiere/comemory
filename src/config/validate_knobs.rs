//! The per-knob bounds every config layer funnels through.
//!
//! Split out of `validate.rs` so Binding Rule 3 (<= 300 code lines) stays
//! green there. Each fn is a pure bound on one value, shared by the scalar
//! arms and the `[tune]` grid loops so a grid entry can never be accepted
//! where the scalar knob would be refused.

use crate::prelude::*;

/// Bounds for `retrieval.rrf_k` and every `tune.rrf_k_grid` entry.
pub(super) fn check_rrf_k(v: f32) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || v <= 0.0 {
        return Err("must be a finite positive number");
    }
    Ok(())
}

/// Bounds for `retrieval.graph_hops`. `0` is valid and disables the
/// graph-expansion leg; the ceiling keeps the recursive edge walk bounded.
pub(super) fn check_graph_hops(v: u32) -> std::result::Result<(), &'static str> {
    if v > 4 {
        return Err("must be <= 4 (0 disables the graph-expansion leg)");
    }
    Ok(())
}

/// The walk needs at least one graph seed, the default page at least one hit,
/// and a capture bound at least one of what it bounds. Persisted `top_k = 0`
/// would otherwise select the router's unlimited-page sentinel.
pub(super) fn check_positive_count(v: usize) -> std::result::Result<(), &'static str> {
    if v < 1 {
        return Err("must be >= 1");
    }
    Ok(())
}

/// Bounds for `rank.decay` and every `tune.decay_grid` entry.
pub(super) fn check_decay(v: f64) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || v < 0.0 {
        return Err("must be a finite non-negative number");
    }
    Ok(())
}

/// Bounds shared by every knob constrained to the unit interval:
/// `rank.mmr_lambda` (and every `tune.mmr_lambda_grid` entry),
/// `retrieval.memory_threshold`, `retrieval.code_threshold`, and
/// `prune.min_feedback`.
pub(super) fn check_unit_interval(v: f64) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err("must be a finite value in [0.0, 1.0]");
    }
    Ok(())
}

/// Bounds shared by every weighted-BM25 column-weight set: the memory pair
/// (`retrieval.bm25_weights`, `tune.bm25_grid` entries) and the code triple
/// (`retrieval.code_bm25_weights`).
pub(super) fn check_bm25_weights(ws: &[f32]) -> std::result::Result<(), &'static str> {
    if ws.iter().any(|w| !w.is_finite() || *w < 0.0) || ws.iter().all(|w| *w == 0.0) {
        return Err("every weight must be finite and >= 0, and at least one > 0");
    }
    Ok(())
}

/// Bounds for `retrieval.document_leg_weight`: the weighted-RRF
/// contribution weight for the document leg must be a finite, strictly
/// positive multiplier — `0.0` would silently disable the leg via a
/// dedicated knob instead of the documented `--only` scope filter.
pub(super) fn check_document_leg_weight(v: f32) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || v <= 0.0 || v > 10.0 {
        return Err("must be a finite value in (0.0, 10.0]");
    }
    Ok(())
}

/// Bounds for `indexing.max_file_bytes`: the document writer's
/// too-large ceiling must be a positive byte count.
pub(super) fn check_max_file_bytes(v: u64) -> std::result::Result<(), &'static str> {
    if v == 0 {
        return Err("must be > 0");
    }
    Ok(())
}

/// Validate one `[tune]` grid: non-empty, and every value passes the same
/// `check` its scalar knob uses. The field is named in every message;
/// grids are file-only, so no env var is cited.
pub(super) fn check_grid<T: Copy + std::fmt::Debug>(
    field: &str,
    values: &[T],
    check: impl Fn(T) -> std::result::Result<(), &'static str>,
) -> Result<()> {
    if values.is_empty() {
        return Err(Error::Config(format!(
            "invalid {field}=[] (file-only [tune] key): grid must be non-empty"
        )));
    }
    for &v in values {
        if let Err(why) = check(v) {
            return Err(Error::Config(format!(
                "invalid {field} value {v:?} (file-only [tune] key): {why}"
            )));
        }
    }
    Ok(())
}
