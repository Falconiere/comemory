//! `[prune]` config section: the scoring floors and retention windows
//! `comemory prune` and `comemory gc` read.
//!
//! Its own file beside `observations.rs` and `rerank.rs` for the reason those
//! have one — a section owns its struct, its file overlay and its invariants.
//! `config::file` keeps a re-export so the historical
//! `config::file::PruneConfig` path still resolves.

use serde::{Deserialize, Serialize};

use super::defaults::default_superseded_grace_days;
use super::validate::{check_knob, check_unit_interval};
use crate::prelude::*;

/// Prune scoring floors and retention windows for `comemory prune` / `gc`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneConfig {
    /// Days a soft-deleted memory stays in the trash before `gc` reaps it.
    pub trash_retention_days: u32,
    /// Quality (1..=5) at or below which a memory is a low-value candidate.
    pub low_value_default_below_quality: u32,
    /// Activation floor (ACT-R scale) below which a memory is prune-eligible.
    ///
    /// Memories whose computed activation falls below this threshold are
    /// candidates for soft-deletion. Default: `-2.0`.
    pub min_activation: f64,
    /// Beta-feedback ceiling at or below which a memory is prune-eligible.
    ///
    /// Range `[0.0, 1.0]`. A memory with cumulative feedback ≤ this value
    /// is considered low-value. Default: `0.25`.
    pub min_feedback: f64,
    /// Days to retain learning telemetry (`retrieval_log` rows and
    /// `feedback_events` rows). `comemory gc` deletes older rows.
    /// Aggregated `feedback` counters are permanent — only raw event
    /// rows age out. Must be >= 1. Default: `90`.
    pub learning_retention_days: u32,
    /// Grace window (days) for the superseded-and-forgotten prune rule:
    /// only supersede edges older than this many days count. Protects
    /// freshly-rebuilt DBs, whose edges all carry rebuild-time timestamps.
    /// `0` disables the grace entirely.
    /// Default: `SUPERSEDED_GRACE_DAYS` in `config::defaults` (7). Plain
    /// backticks, not a link: the constant is `pub(crate)`, and an intra-doc
    /// link to it from a public item raises `links to private item`.
    #[serde(default = "default_superseded_grace_days")]
    pub superseded_grace_days: u32,
}

impl PruneConfig {
    /// Overlay the file's `[prune]` keys; absent keys leave `self` untouched.
    pub(crate) fn apply(&mut self, p: PartialPruneConfig) {
        if let Some(v) = p.trash_retention_days {
            self.trash_retention_days = v;
        }
        if let Some(v) = p.low_value_default_below_quality {
            self.low_value_default_below_quality = v;
        }
        if let Some(v) = p.min_activation {
            self.min_activation = v;
        }
        if let Some(v) = p.min_feedback {
            self.min_feedback = v;
        }
        if let Some(v) = p.learning_retention_days {
            self.learning_retention_days = v;
        }
        if let Some(v) = p.superseded_grace_days {
            self.superseded_grace_days = v;
        }
    }
}

impl PruneConfig {
    /// Prune scoring floors and the learning-telemetry retention window.
    pub(crate) fn validate(&self) -> Result<()> {
        let a = self.min_activation;
        if !a.is_finite() {
            return Err(Error::Config(format!(
                "invalid prune.min_activation={a} (env COMEMORY_PRUNE_MIN_ACTIVATION): must be a finite number"
            )));
        }
        let f = self.min_feedback;
        check_knob(
            "prune.min_feedback",
            "COMEMORY_PRUNE_MIN_FEEDBACK",
            f,
            check_unit_interval(f),
        )?;
        let r = self.learning_retention_days;
        if r < 1 {
            return Err(Error::Config(format!(
                "invalid prune.learning_retention_days={r} (env COMEMORY_LEARNING_RETENTION_DAYS): must be >= 1"
            )));
        }
        let trash_days = self.trash_retention_days;
        if trash_days < 1 {
            return Err(Error::Config(format!(
                "invalid prune.trash_retention_days={trash_days} (file-only [prune] key): must be >= 1"
            )));
        }
        let q = self.low_value_default_below_quality;
        if !(1..=5).contains(&q) {
            return Err(Error::Config(format!(
                "invalid prune.low_value_default_below_quality={q} (env COMEMORY_PRUNE_BELOW_QUALITY): must be in 1..=5"
            )));
        }
        // `prune.superseded_grace_days` has no range arm: any u32 is valid
        // (0 disables the grace window).
        Ok(())
    }
}

/// File-overlay partial for [`PruneConfig`]. All fields optional.
///
/// Carries every *consumed* `PruneConfig` field, not just the M1 scoring
/// extensions: `deny_unknown_fields` would otherwise hard-error on a valid
/// `[prune]` key like `trash_retention_days` once the section is
/// overlayable at all.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialPruneConfig {
    pub(crate) trash_retention_days: Option<u32>,
    pub(crate) low_value_default_below_quality: Option<u32>,
    pub(crate) min_activation: Option<f64>,
    pub(crate) min_feedback: Option<f64>,
    pub(crate) learning_retention_days: Option<u32>,
    pub(crate) superseded_grace_days: Option<u32>,
}

/// The shipped `[prune]` defaults — the base layer every overlay is applied to.
pub(crate) fn default_prune() -> PruneConfig {
    PruneConfig {
        trash_retention_days: 30,
        low_value_default_below_quality: 2,
        min_activation: -2.0,
        min_feedback: 0.25,
        learning_retention_days: 90,
        superseded_grace_days: default_superseded_grace_days(),
    }
}
