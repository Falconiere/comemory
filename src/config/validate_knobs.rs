//! The per-knob bounds every config layer funnels through.
//!
//! Split out of `validate.rs` so Binding Rule 3 (<= 300 code lines) stays
//! green there. Each fn is a pure bound on one value, shared by the scalar
//! arms and the `[tune]` grid loops so a grid entry can never be accepted
//! where the scalar knob would be refused.

use crate::prelude::*;

/// One `pub(super) fn name(v: ty) -> Result<(), &'static str>` per row: the
/// value is refused with `msg` when `bad` holds. The predicate and message
/// are the only things that vary between the scalar bounds, so a table
/// replaces near-identical bodies.
macro_rules! knob_checks {
    ($(
        $(#[$meta:meta])*
        $name:ident($v:ident: $ty:ty) refuses $bad:expr => $msg:literal
    ),+ $(,)?) => {
        $(
            $(#[$meta])*
            pub(super) fn $name($v: $ty) -> std::result::Result<(), &'static str> {
                if $bad {
                    return Err($msg);
                }
                Ok(())
            }
        )+
    };
}

knob_checks! {
    /// Bounds for `retrieval.rrf_k` and every `tune.rrf_k_grid` entry.
    check_rrf_k(v: f32) refuses !v.is_finite() || v <= 0.0
        => "must be a finite positive number",
    /// Bounds for `retrieval.graph_hops`. `0` is valid and disables the
    /// graph-expansion leg; the ceiling keeps the recursive edge walk bounded.
    check_graph_hops(v: u32) refuses v > 4
        => "must be <= 4 (0 disables the graph-expansion leg)",
    /// The walk needs at least one graph seed, the default page at least one
    /// hit, and a capture bound at least one of what it bounds. Persisted
    /// `top_k = 0` would otherwise select the router's unlimited-page sentinel.
    check_positive_count(v: usize) refuses v < 1
        => "must be >= 1",
    /// Bounds for `rank.decay` and every `tune.decay_grid` entry.
    check_decay(v: f64) refuses !v.is_finite() || v < 0.0
        => "must be a finite non-negative number",
    /// Bounds for `prune.min_activation`: any finite ACT-R activation floor.
    check_finite(v: f64) refuses !v.is_finite()
        => "must be a finite number",
    /// Bounds shared by every knob constrained to the unit interval:
    /// `rank.mmr_lambda` (and every `tune.mmr_lambda_grid` entry),
    /// `retrieval.memory_threshold`, `retrieval.code_threshold`, and
    /// `prune.min_feedback`.
    check_unit_interval(v: f64) refuses !v.is_finite() || !(0.0..=1.0).contains(&v)
        => "must be a finite value in [0.0, 1.0]",
    /// Bounds for `retrieval.document_leg_weight`: the weighted-RRF
    /// contribution weight for the document leg must be a finite, strictly
    /// positive multiplier — `0.0` would silently disable the leg via a
    /// dedicated knob instead of the documented `--only` scope filter.
    check_document_leg_weight(v: f32) refuses !v.is_finite() || v <= 0.0 || v > 10.0
        => "must be a finite value in (0.0, 10.0]",
    /// Bounds for `indexing.max_file_bytes`: the document writer's
    /// too-large ceiling must be a positive byte count.
    check_max_file_bytes(v: u64) refuses v == 0
        => "must be > 0",
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
