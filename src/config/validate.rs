//! Shared invariant pass for the layered config.
//!
//! Split out of `file.rs` to keep each config file narrow: `file.rs` owns
//! the struct definitions, defaults, and the file overlay; `env.rs` owns
//! env parsing; this module owns the invariants both layers funnel through.
//!
//! The per-knob bounds live in tiny check fns so the scalar arms and the
//! `[tune]` grid loops can never drift on what a valid value is: a grid
//! containing `rrf_k = 0.0` fails exactly like `retrieval.rrf_k = 0.0`.

use super::file::Config;
use crate::prelude::*;

/// Bounds for `retrieval.rrf_k` and every `tune.rrf_k_grid` entry.
fn check_rrf_k(v: f32) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || v <= 0.0 {
        return Err("must be a finite positive number");
    }
    Ok(())
}

/// Bounds for `rank.decay` and every `tune.decay_grid` entry.
fn check_decay(v: f64) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || v < 0.0 {
        return Err("must be a finite non-negative number");
    }
    Ok(())
}

/// Bounds for `rank.mmr_lambda` and every `tune.mmr_lambda_grid` entry.
fn check_mmr_lambda(v: f64) -> std::result::Result<(), &'static str> {
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err("must be a finite value in [0.0, 1.0]");
    }
    Ok(())
}

/// Bounds shared by every weighted-BM25 column-weight set: the memory pair
/// (`retrieval.bm25_weights`, `tune.bm25_grid` entries) and the code triple
/// (`retrieval.code_bm25_weights`).
fn check_bm25_weights(ws: &[f32]) -> std::result::Result<(), &'static str> {
    if ws.iter().any(|w| !w.is_finite() || *w < 0.0) || ws.iter().all(|w| *w == 0.0) {
        return Err("every weight must be finite and >= 0, and at least one > 0");
    }
    Ok(())
}

/// Validate one `[tune]` grid: non-empty, and every value passes the same
/// `check` its scalar knob uses. The field is named in every message;
/// grids are file-only, so no env var is cited.
fn check_grid<T: Copy + std::fmt::Debug>(
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

impl Config {
    /// Enforce the documented retrieval/rank/prune/tune invariants.
    ///
    /// Runs at the end of both [`Config::with_file`] and
    /// [`Config::with_env`] so the file overlay and env overrides are
    /// validated identically — `[rank] decay = -1.0` in config.toml fails
    /// exactly like `COMEMORY_RANK_DECAY=-1.0`. Each message names both
    /// the config field and its env var (when one exists) so the offending
    /// knob is identifiable from either entry point.
    pub(crate) fn validate(self) -> Result<Self> {
        let (b, t) = self.retrieval.bm25_weights;
        if let Err(why) = check_bm25_weights(&[b, t]) {
            return Err(Error::Config(format!(
                "invalid retrieval.bm25_weights={b},{t} (env COMEMORY_RETRIEVAL_BM25_WEIGHTS): {why}"
            )));
        }
        let (cs, cn, cp) = self.retrieval.code_bm25_weights;
        if let Err(why) = check_bm25_weights(&[cs, cn, cp]) {
            return Err(Error::Config(format!(
                "invalid retrieval.code_bm25_weights={cs},{cn},{cp} (env COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS): {why}"
            )));
        }
        let k = self.retrieval.rrf_k;
        if let Err(why) = check_rrf_k(k) {
            return Err(Error::Config(format!(
                "invalid retrieval.rrf_k={k} (env COMEMORY_RETRIEVAL_RRF_K): {why}"
            )));
        }
        let ct = self.retrieval.code_threshold;
        if !ct.is_finite() || !(0.0..=1.0).contains(&ct) {
            return Err(Error::Config(format!(
                "invalid retrieval.code_threshold={ct} (env COMEMORY_RETRIEVAL_CODE_THRESHOLD): must be a finite value in [0.0, 1.0]"
            )));
        }
        let d = self.rank.decay;
        if let Err(why) = check_decay(d) {
            return Err(Error::Config(format!(
                "invalid rank.decay={d} (env COMEMORY_RANK_DECAY): {why}"
            )));
        }
        let (lo, hi) = self.rank.prior_clamp;
        if !lo.is_finite() || !hi.is_finite() || lo <= 0.0 || lo > hi {
            return Err(Error::Config(format!(
                "invalid rank.prior_clamp={lo},{hi} (env COMEMORY_RANK_PRIOR_CLAMP): both values must be finite, lo > 0, and lo <= hi"
            )));
        }
        let l = self.rank.mmr_lambda;
        if let Err(why) = check_mmr_lambda(l) {
            return Err(Error::Config(format!(
                "invalid rank.mmr_lambda={l} (env COMEMORY_RANK_MMR_LAMBDA): {why}"
            )));
        }
        let h = self.rank.near_dup_hamming;
        if h > 64 {
            return Err(Error::Config(format!(
                "invalid rank.near_dup_hamming={h} (env COMEMORY_RANK_NEAR_DUP_HAMMING): must be <= 64 (SimHash is 64-bit)"
            )));
        }
        let a = self.prune.min_activation;
        if !a.is_finite() {
            return Err(Error::Config(format!(
                "invalid prune.min_activation={a} (env COMEMORY_PRUNE_MIN_ACTIVATION): must be a finite number"
            )));
        }
        let f = self.prune.min_feedback;
        if !f.is_finite() || !(0.0..=1.0).contains(&f) {
            return Err(Error::Config(format!(
                "invalid prune.min_feedback={f} (env COMEMORY_PRUNE_MIN_FEEDBACK): must be a finite value in [0.0, 1.0]"
            )));
        }
        let r = self.prune.learning_retention_days;
        if r < 1 {
            return Err(Error::Config(format!(
                "invalid prune.learning_retention_days={r} (env COMEMORY_LEARNING_RETENTION_DAYS): must be >= 1"
            )));
        }
        let q = self.prune.low_value_default_below_quality;
        if !(1..=5).contains(&q) {
            return Err(Error::Config(format!(
                "invalid prune.low_value_default_below_quality={q} (env COMEMORY_PRUNE_BELOW_QUALITY): must be in 1..=5"
            )));
        }
        // `prune.superseded_grace_days` has no range arm: any u32 is valid
        // (0 disables the grace window).
        check_grid("tune.rrf_k_grid", &self.tune.rrf_k_grid, check_rrf_k)?;
        check_grid("tune.decay_grid", &self.tune.decay_grid, check_decay)?;
        check_grid(
            "tune.mmr_lambda_grid",
            &self.tune.mmr_lambda_grid,
            check_mmr_lambda,
        )?;
        check_grid("tune.bm25_grid", &self.tune.bm25_grid, |(wb, wt)| {
            check_bm25_weights(&[wb, wt])
        })?;
        Ok(self)
    }
}
