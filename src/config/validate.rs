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
use super::sync::parse_duration;
use super::validate_knobs::{
    check_bm25_weights, check_decay, check_document_leg_weight, check_graph_hops, check_grid,
    check_max_file_bytes, check_rrf_k, check_unit_interval,
};
use crate::prelude::*;

pub(super) use super::validate_knobs::check_positive_count;

/// Attach the field, its env override, and the original display value to a failed bound.
pub(super) fn check_knob(
    field: &str,
    env: &str,
    value: impl std::fmt::Display,
    check: std::result::Result<(), &'static str>,
) -> Result<()> {
    check.map_err(|why| Error::Config(format!("invalid {field}={value} (env {env}): {why}")))
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
        self.check_retrieval_weights()?;
        self.check_retrieval_knobs()?;
        self.check_rank_knobs()?;
        self.check_prune_knobs()?;
        self.check_tune_grids()?;
        self.check_reinforce_knobs()?;
        self.check_indexing_knobs()?;
        self.check_sync_knobs()?;
        self.observations.validate()?;
        self.activity.validate().map(|()| self)
    }

    /// Weighted-BM25 column-weight sets for `memory_fts` and `code_fts`.
    fn check_retrieval_weights(&self) -> Result<()> {
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
        Ok(())
    }

    /// Scalar retrieval knobs: fusion constant, graph-walk bounds, page
    /// size and window, and the two ANN similarity floors.
    fn check_retrieval_knobs(&self) -> Result<()> {
        let k = self.retrieval.rrf_k;
        check_knob(
            "retrieval.rrf_k",
            "COMEMORY_RETRIEVAL_RRF_K",
            k,
            check_rrf_k(k),
        )?;
        let tk = self.retrieval.top_k;
        check_knob(
            "retrieval.top_k",
            "COMEMORY_RETRIEVAL_TOP_K",
            tk,
            check_positive_count(tk),
        )?;
        let gh = self.retrieval.graph_hops;
        check_knob(
            "retrieval.graph_hops",
            "COMEMORY_RETRIEVAL_GRAPH_HOPS",
            gh,
            check_graph_hops(gh),
        )?;
        let gs = self.retrieval.graph_seeds;
        check_knob(
            "retrieval.graph_seeds",
            "COMEMORY_RETRIEVAL_GRAPH_SEEDS",
            gs,
            check_positive_count(gs),
        )?;
        let w = self.retrieval.max_page_window;
        if w == 0 {
            return Err(Error::Config(
                "invalid retrieval.max_page_window=0 (env COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW): must be > 0".into(),
            ));
        }
        let mt = self.retrieval.memory_threshold;
        check_knob(
            "retrieval.memory_threshold",
            "COMEMORY_RETRIEVAL_MEMORY_THRESHOLD",
            mt,
            check_unit_interval(f64::from(mt)),
        )?;
        let ct = self.retrieval.code_threshold;
        check_knob(
            "retrieval.code_threshold",
            "COMEMORY_RETRIEVAL_CODE_THRESHOLD",
            ct,
            check_unit_interval(f64::from(ct)),
        )?;
        let dw = self.retrieval.document_leg_weight;
        check_knob(
            "retrieval.document_leg_weight",
            "COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT",
            dw,
            check_document_leg_weight(dw),
        )
    }

    /// Document-source indexing knobs consumed by `comemory index`.
    fn check_indexing_knobs(&self) -> Result<()> {
        let m = self.indexing.max_file_bytes;
        check_knob(
            "indexing.max_file_bytes",
            "COMEMORY_INDEXING_MAX_FILE_BYTES",
            m,
            check_max_file_bytes(m),
        )
    }

    /// Ranking knobs consumed by `retrieval::{rerank,diversify}`.
    fn check_rank_knobs(&self) -> Result<()> {
        let d = self.rank.decay;
        check_knob("rank.decay", "COMEMORY_RANK_DECAY", d, check_decay(d))?;
        let (lo, hi) = self.rank.prior_clamp;
        if !lo.is_finite() || !hi.is_finite() || lo <= 0.0 || lo > hi {
            return Err(Error::Config(format!(
                "invalid rank.prior_clamp={lo},{hi} (env COMEMORY_RANK_PRIOR_CLAMP): both values must be finite, lo > 0, and lo <= hi"
            )));
        }
        let l = self.rank.mmr_lambda;
        check_knob(
            "rank.mmr_lambda",
            "COMEMORY_RANK_MMR_LAMBDA",
            l,
            check_unit_interval(l),
        )?;
        let h = self.rank.near_dup_hamming;
        if h > 64 {
            return Err(Error::Config(format!(
                "invalid rank.near_dup_hamming={h} (env COMEMORY_RANK_NEAR_DUP_HAMMING): must be <= 64 (SimHash is 64-bit)"
            )));
        }
        Ok(())
    }

    /// Prune scoring floors and the learning-telemetry retention window.
    fn check_prune_knobs(&self) -> Result<()> {
        let a = self.prune.min_activation;
        if !a.is_finite() {
            return Err(Error::Config(format!(
                "invalid prune.min_activation={a} (env COMEMORY_PRUNE_MIN_ACTIVATION): must be a finite number"
            )));
        }
        let f = self.prune.min_feedback;
        check_knob(
            "prune.min_feedback",
            "COMEMORY_PRUNE_MIN_FEEDBACK",
            f,
            check_unit_interval(f),
        )?;
        let r = self.prune.learning_retention_days;
        if r < 1 {
            return Err(Error::Config(format!(
                "invalid prune.learning_retention_days={r} (env COMEMORY_LEARNING_RETENTION_DAYS): must be >= 1"
            )));
        }
        let trash_days = self.prune.trash_retention_days;
        if trash_days < 1 {
            return Err(Error::Config(format!(
                "invalid prune.trash_retention_days={trash_days} (file-only [prune] key): must be >= 1"
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
        Ok(())
    }

    /// The file-only `[tune]` grids, each held to its scalar knob's bounds.
    fn check_tune_grids(&self) -> Result<()> {
        check_grid("tune.rrf_k_grid", &self.tune.rrf_k_grid, check_rrf_k)?;
        check_grid("tune.decay_grid", &self.tune.decay_grid, check_decay)?;
        check_grid(
            "tune.mmr_lambda_grid",
            &self.tune.mmr_lambda_grid,
            check_unit_interval,
        )?;
        check_grid("tune.bm25_grid", &self.tune.bm25_grid, |(wb, wt)| {
            check_bm25_weights(&[wb, wt])
        })?;
        check_grid(
            "tune.graph_hops_grid",
            &self.tune.graph_hops_grid,
            check_graph_hops,
        )?;
        check_grid(
            "tune.graph_seeds_grid",
            &self.tune.graph_seeds_grid,
            check_positive_count,
        )?;
        // `tune.samples` has no range arm: any usize is valid (0 means the
        // exhaustive cartesian grid, and the sampler clamps a value above
        // the pool product down to it).
        Ok(())
    }

    /// Search→edit auto-reinforcement lookback.
    fn check_reinforce_knobs(&self) -> Result<()> {
        let sed = self.reinforce.search_edit_days;
        if sed < 1 {
            return Err(Error::Config(format!(
                "invalid reinforce.search_edit_days={sed} (env COMEMORY_REINFORCE_SEARCH_EDIT_DAYS): must be >= 1"
            )));
        }
        Ok(())
    }

    /// `[sync]` duration strings and `[embed].model` shape.
    fn check_sync_knobs(&self) -> Result<()> {
        // Empty / `"0"` means the deprecated pull-before-context hook is off.
        self.sync
            .pull_before_context_after_duration()
            .map_err(|e| {
                Error::Config(format!(
                    "invalid sync.pull_before_context_after={}: {e}",
                    self.sync.pull_before_context_after
                ))
            })?;
        parse_duration(&self.sync.verify_every).map_err(|e| {
            Error::Config(format!(
                "invalid sync.verify_every={}: {e}",
                self.sync.verify_every
            ))
        })?;
        parse_duration(&self.sync.daemon_interval).map_err(|e| {
            Error::Config(format!(
                "invalid sync.daemon_interval={}: {e}",
                self.sync.daemon_interval
            ))
        })?;
        let push_timeout = self.sync.push_on_save_timeout_duration().map_err(|e| {
            Error::Config(format!(
                "invalid sync.push_on_save_timeout={}: {e}",
                self.sync.push_on_save_timeout
            ))
        })?;
        // A zero timeout is a parseable duration that would make every inline
        // push fail instantly — indistinguishable from "offline" forever. The
        // way to turn the hook off is `push_on_save = false`, which says so.
        if push_timeout.is_zero() {
            return Err(Error::Config(
                "sync.push_on_save_timeout must be greater than zero (use sync.push_on_save = false to disable the inline push)".into(),
            ));
        }
        // `sync.allowlist_ttl` is deprecated and ignored, so its value is no
        // longer validated: rejecting a config over a key nothing reads would
        // block an upgrade for no gain.
        self.sync.skip_matcher()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/validate.rs"]
mod tests;
