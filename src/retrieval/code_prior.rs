//! The four bounded code priors — PageRank, ACT-R activation, working-set
//! co-change affinity, Beta feedback — computed in exactly one place.
//!
//! Two consumers share this math: [`crate::retrieval::code_rerank`]
//! multiplies the prior product into a max-normalized relevance score for
//! `comemory search-code`, and [`crate::retrieval::bundle`] ranks the code
//! refs of `comemory context` by the prior product alone — refs are
//! address-resolved by the graph walk, not query-matched, so they carry no
//! relevance term. Both follow the same pooled discipline: fetch
//! [`signals`] once per candidate, derive the median via
//! [`median_file_rank`], then score with [`priors`] under one shared clock
//! and one shared affinity cache.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::retrieval::code_rerank::WorkingSet;
use crate::retrieval::score;
use crate::store::Connection;

/// Per-symbol ranking signals, re-exported from [`crate::store::code_signals`]
/// (which owns the `code_symbols` + `code_feedback` join SQL) so this
/// module's own callers (`bundle`, `code_rerank`) need not know the query
/// moved.
pub use crate::store::code_signals::{Signals, signals, signals_batch};

/// Scale for the PageRank boost: `1 + RANK_SCALE·ln(1 + raw/median)`.
/// A file at the pool median maps to `1 + 0.2·ln 2 ≈ 1.14`; the clamp
/// from `cfg.rank.prior_clamp` bounds the extremes.
pub const RANK_SCALE: f64 = 0.2;

/// Scale for the working-set co-change affinity boost:
/// `1 + AFFINITY_SCALE·ln(1 + w_sum)`. Zero co-change weight maps to
/// exactly 1.0 (neutral).
pub const AFFINITY_SCALE: f64 = 0.2;

/// The four multiplicative graph priors behind a code ranking, plus their
/// product. Serialized verbatim into `--json` output (the `rank_parts`
/// object on `comemory context` code refs) — a stable contract, not debug
/// info. Unlike [`crate::retrieval::code_rerank::CodeScoreParts`] there is
/// no relevance term: the invariant is
/// `final_score == rank * activation * affinity * feedback`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodePriorParts {
    /// PageRank boost (post-clamp multiplier), pool-median-relative.
    pub rank: f64,
    /// ACT-R activation boost (post-clamp multiplier).
    pub activation: f64,
    /// Working-set co-change affinity boost (post-clamp multiplier).
    pub affinity: f64,
    /// Beta feedback boost (post-clamp multiplier).
    pub feedback: f64,
    /// Product of the four priors.
    pub final_score: f64,
}

/// Compute the four bounded priors for one signals row — the single home
/// of the prior math. Pool-scoring callers (`rerank_code`, the context
/// bundle) pass a shared `now` so one pool is judged against one clock,
/// and a shared `affinity_cache` so many symbols from one file run one
/// edge query. Derive `pool_median_rank` for the caller's candidate set
/// via [`median_file_rank`] over the pool's fetched [`signals`] rows.
pub fn priors(
    conn: &Connection,
    cfg: &Config,
    now: OffsetDateTime,
    sig: &Signals,
    working_set: &WorkingSet,
    pool_median_rank: f64,
    affinity_cache: &mut BTreeMap<String, f64>,
) -> Result<CodePriorParts> {
    let clamp = cfg.rank.prior_clamp;
    let rank = score::rank_boost(sig.rank_score, pool_median_rank, RANK_SCALE, clamp);
    let days = score::days_since(&sig.last_accessed, now);
    let activation = score::activation_boost(
        score::activation(sig.access_count, days, cfg.rank.decay),
        clamp,
    );
    let affinity = file_affinity(
        conn,
        working_set,
        &sig.repo,
        &sig.path,
        clamp,
        affinity_cache,
    )?;
    let feedback = score::feedback_boost(score::beta_feedback(sig.used, sig.irrelevant), clamp);
    Ok(CodePriorParts {
        rank,
        activation,
        affinity,
        feedback,
        final_score: rank * activation * affinity * feedback,
    })
}

/// Median of the candidate pool's DISTINCT per-file `rank_score`s — chunk
/// rows share their file's projected score, so dedup is by `(repo, path)`
/// before the shared [`score::median_rank`] does the median math.
pub fn median_file_rank<'a>(files: impl IntoIterator<Item = ((&'a str, &'a str), f64)>) -> f64 {
    let by_file: BTreeMap<(&str, &str), f64> = files.into_iter().collect();
    let mut ranks: Vec<f64> = by_file.into_values().collect();
    score::median_rank(&mut ranks)
}

/// Working-set affinity prior for one candidate file:
/// `bounded(1 + AFFINITY_SCALE·ln(1 + w_sum), clamp)` where `w_sum` is
/// the total `co_changed` edge weight between the candidate's file and
/// the working-set files. Cached per distinct candidate file in
/// `cache` so a pool with many symbols from one file runs one edge
/// query. An empty working set short-circuits to neutral 1.0 with no
/// query at all.
fn file_affinity(
    conn: &Connection,
    ws: &WorkingSet,
    repo: &str,
    path: &str,
    clamp: (f64, f64),
    cache: &mut BTreeMap<String, f64>,
) -> Result<f64> {
    if ws.files().is_empty() {
        return Ok(1.0);
    }
    let fid = crate::store::edges::file_node_id(repo, path);
    if let Some(boost) = cache.get(&fid) {
        return Ok(*boost);
    }
    let w_sum = crate::store::edges_retrieval::co_change_weight(conn, &fid, ws.files())?;
    let boost = score::bounded_boost(1.0 + AFFINITY_SCALE * (1.0 + w_sum).ln(), clamp);
    cache.insert(fid, boost);
    Ok(boost)
}

#[cfg(test)]
#[path = "tests/code_prior.rs"]
mod tests;
