//! `output::explain` — the console's explain strip: each hit's `score_parts`
//! rendered as `{name, value, share, note}` rows (console-api spec §3).
//!
//! **Derived, never recomputed** (spec Non-Goal 9). Every number here is
//! read straight off the domain's own [`HitParts`]; this module reshapes it
//! for display and adds one thing the raw parts do not carry — the `share`
//! each multiplicative prior had in moving the score.
//!
//! **The share partition.** A rerank multiplies the normalized relevance by
//! bounded priors, so "how much did this prior matter" is a question about
//! log-magnitude, not about the factor's raw distance from zero:
//!
//! ```text
//! share_i = |ln(value_i)| / Σ_j |ln(value_j)|      over the PRIOR parts only
//! ```
//!
//! A factor of exactly `1.0` therefore gets `share == 0` — it did not move
//! the score, and reporting it as a fraction of the outcome would be a lie.
//! When every prior is `1.0` the denominator is zero and each of the `n`
//! priors gets `1/n` instead, so the strip stays a partition rather than
//! collapsing to all-zeros.
//!
//! **Invariant:** over a non-empty set of prior parts the `share`s sum to
//! `1.0` (±1e-9). Leg signals (`bm25`, `ann`, `bm25_rank`) are
//! informational — they are inputs to fusion, not multiplicative factors —
//! and always carry `share == 0`, so they never enter the partition. A
//! document hit consists only of a leg signal (the document leg has no
//! rerank stage), so its strip has no prior parts and no partition at all.

use serde::Serialize;

use crate::retrieval::score::LegScores;
use crate::retrieval::unified::fuse_domains::{DocParts, HitParts};

/// One row of the console's explain strip.
#[derive(Debug, Clone, Serialize)]
pub struct ExplainPart {
    /// Signal name: `bm25`, `ann`, `activation`, `feedback`, `quality`,
    /// `supersede`, `pagerank`, `affinity`, or `bm25_rank`.
    pub name: String,
    /// The signal's own value, verbatim from the domain's `score_parts`.
    pub value: f64,
    /// This prior's log-magnitude share of the rerank (see the module doc);
    /// `0.0` for a leg signal.
    pub share: f64,
    /// Short human-readable gloss (`"score 3.1"`, `"cosine 0.82"`,
    /// `"×1.14"`, `"rank 3"`).
    pub note: String,
}

/// Render one hit's `score_parts` as the console's explain strip.
///
/// The emitted order is per-domain and stable: leg signals first (only the
/// ones the hit actually has), then the multiplicative priors in the order
/// that domain's rerank applies them.
pub fn parts_of(parts: &HitParts) -> Vec<ExplainPart> {
    match parts {
        HitParts::Memory(p) => partition(
            legs(&p.legs),
            vec![
                prior("activation", p.activation),
                prior("feedback", p.feedback),
                prior("quality", p.quality),
                prior("supersede", p.supersede),
                prior("pagerank", p.rank),
            ],
        ),
        HitParts::Code(p) => partition(
            legs(&p.legs),
            vec![
                prior("pagerank", p.rank),
                prior("activation", p.activation),
                prior("affinity", p.affinity),
                prior("feedback", p.feedback),
            ],
        ),
        HitParts::Document(p) => partition(document_legs(*p), Vec::new()),
    }
}

/// The two router leg signals, each emitted only when that leg actually
/// produced the hit (`None` means "this leg did not fire", which is not the
/// same as a score of zero).
fn legs(legs: &LegScores) -> Vec<ExplainPart> {
    let mut out = Vec::with_capacity(2);
    if let Some(bm25) = legs.bm25 {
        out.push(leg("bm25", f64::from(bm25), format!("score {bm25:.4}")));
    }
    if let Some(ann) = legs.ann {
        out.push(leg("ann", f64::from(ann), format!("cosine {ann:.4}")));
    }
    out
}

/// The document leg's single signal: its 1-based BM25 position. Reported as
/// a leg signal rather than a prior — the document route has no rerank
/// stage, so there is nothing multiplicative to partition.
fn document_legs(p: DocParts) -> Vec<ExplainPart> {
    let rank = p.bm25_rank;
    vec![leg("bm25_rank", rank as f64, format!("rank {rank}"))]
}

/// A leg signal: informational, never part of the share partition.
fn leg(name: &str, value: f64, note: String) -> ExplainPart {
    ExplainPart {
        name: name.to_string(),
        value,
        share: 0.0,
        note,
    }
}

/// A multiplicative prior, with `share` left at zero for [`partition`] to
/// fill in.
fn prior(name: &str, value: f64) -> ExplainPart {
    ExplainPart {
        name: name.to_string(),
        value,
        share: 0.0,
        note: format!("×{value:.3}"),
    }
}

/// Assign each prior its log-magnitude share (module doc) and concatenate
/// the strip: leg signals first, priors after.
fn partition(legs: Vec<ExplainPart>, mut priors: Vec<ExplainPart>) -> Vec<ExplainPart> {
    let magnitudes: Vec<f64> = priors.iter().map(|p| log_magnitude(p.value)).collect();
    let total: f64 = magnitudes.iter().sum();
    let n = priors.len();
    for (part, magnitude) in priors.iter_mut().zip(magnitudes) {
        part.share = if total > 0.0 {
            magnitude / total
        } else if n > 0 {
            1.0 / n as f64
        } else {
            0.0
        };
    }
    let mut out = legs;
    out.append(&mut priors);
    out
}

/// `|ln(value)|`, the weight one prior carries in the partition. A
/// non-positive or non-finite factor contributes nothing: `ln` would be
/// `-inf`/`NaN` there and would poison every other share, and a prior that
/// far outside the configured clamp cannot be honestly attributed anyway.
fn log_magnitude(value: f64) -> f64 {
    if value > 0.0 && value.is_finite() {
        value.ln().abs()
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "tests/explain.rs"]
mod tests;
