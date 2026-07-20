//! Learning-loop config: tune grids, search→edit reinforce lookback, bandit gate.
//!
//! Split out of `file.rs` so Binding Rule 3 (≤300 code lines) stays green.

use serde::{Deserialize, Serialize};

/// Grid lists for `comemory tune`'s deterministic search — the cartesian
/// product of the four lists is the candidate grid.
///
/// File-only (`[tune]` in `config.toml`): no `COMEMORY_TUNE_*` env vars.
/// Each list must be non-empty; every value passes the same bounds as its
/// scalar knob (`Config::validate`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneConfig {
    /// RRF fusion constants; same finite-positive invariant as `retrieval.rrf_k`.
    pub rrf_k_grid: Vec<f32>,
    /// ACT-R decay exponents; same finite, >= 0 invariant as `rank.decay`.
    pub decay_grid: Vec<f64>,
    /// MMR lambdas; same `[0.0, 1.0]` invariant as `rank.mmr_lambda`.
    pub mmr_lambda_grid: Vec<f64>,
    /// `(body, tags)` BM25 pairs; same invariants as `retrieval.bm25_weights`.
    pub bm25_grid: Vec<(f32, f32)>,
}

impl Default for TuneConfig {
    /// The M1 3×3×3×3 grid (81 points), bracketing the shipped defaults.
    fn default() -> Self {
        Self {
            rrf_k_grid: vec![20.0, 60.0, 100.0],
            decay_grid: vec![0.3, 0.5, 0.8],
            mmr_lambda_grid: vec![0.5, 0.7, 0.9],
            bm25_grid: vec![(1.0, 3.0), (1.0, 1.0), (2.0, 1.0)],
        }
    }
}

/// Search→edit auto-reinforcement knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReinforceConfig {
    /// Days of `retrieval_log` lookback for search→edit provenance.
    /// Validated `≥ 1`. Env: `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS`.
    pub search_edit_days: u32,
}

impl Default for ReinforceConfig {
    fn default() -> Self {
        Self {
            search_edit_days: 7,
        }
    }
}

/// Online bandit knobs for `comemory bandit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanditConfig {
    /// When false, `comemory bandit --apply` refuses; report still works.
    #[serde(default = "default_bandit_enabled")]
    pub enabled: bool,
}

fn default_bandit_enabled() -> bool {
    true
}

impl Default for BanditConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// File-overlay partial for [`TuneConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialTuneConfig {
    pub(crate) rrf_k_grid: Option<Vec<f32>>,
    pub(crate) decay_grid: Option<Vec<f64>>,
    pub(crate) mmr_lambda_grid: Option<Vec<f64>>,
    pub(crate) bm25_grid: Option<Vec<(f32, f32)>>,
}

/// File-overlay partial for [`ReinforceConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialReinforceConfig {
    pub(crate) search_edit_days: Option<u32>,
}

/// File-overlay partial for [`BanditConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialBanditConfig {
    pub(crate) enabled: Option<bool>,
}
