//! `[observations]` config section: opt-in, bounded capture of a real query's
//! candidate pool (#209).
//!
//! Its own file beside `retrieval.rs` and `sync.rs` for the reason those have
//! one — a section owns its struct, its file overlay and its invariants, and
//! `file.rs` / `validate.rs` stay at one line each.

use serde::{Deserialize, Serialize};

use super::env::{env_parse, parse_bool_env};
use super::validate::{check_knob, check_positive_count};
use crate::prelude::*;

/// Opt-in, bounded capture of candidate observations at query time.
///
/// Off by default: capture writes a passage snapshot per candidate, which is
/// training data a user opts into rather than telemetry they inherit. The two
/// bounds are what keep an enabled capture predictable in size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationsConfig {
    /// Whether `comemory find` captures its candidate pool. Env:
    /// `COMEMORY_OBSERVATIONS_ENABLED`. Capture additionally requires the run
    /// to be one that may write telemetry at all, so a read-only
    /// `comemory serve` captures nothing whatever this says.
    pub enabled: bool,
    /// Per-candidate passage bound, in bytes, handed to `BoundedText::bound`.
    /// Validated `> 0`. Env: `COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES`.
    pub max_text_bytes: usize,
    /// Ceiling on candidates persisted per captured query. Validated `> 0`.
    /// The pool is cut at this many, never below the returned page. Env:
    /// `COMEMORY_OBSERVATIONS_MAX_CANDIDATES`.
    pub max_candidates: usize,
}

impl Default for ObservationsConfig {
    /// Off, at the contract's own default text bound and a candidate ceiling
    /// above the pool a default page builds, so enabling capture does not by
    /// itself truncate anything.
    fn default() -> Self {
        Self {
            enabled: false,
            max_text_bytes: 4096,
            max_candidates: 100,
        }
    }
}

impl ObservationsConfig {
    /// Overlay the file's `[observations]` keys; absent keys leave `self`.
    pub(crate) fn apply(&mut self, p: PartialObservationsConfig) {
        if let Some(v) = p.enabled {
            self.enabled = v;
        }
        if let Some(v) = p.max_text_bytes {
            self.max_text_bytes = v;
        }
        if let Some(v) = p.max_candidates {
            self.max_candidates = v;
        }
    }

    /// Apply the `COMEMORY_OBSERVATIONS_*` overrides, the outermost layer.
    ///
    /// Here rather than in `config::env` because that module already holds six
    /// structurally identical `apply_*_env` methods, and a seventh would be a
    /// near-duplicate of every one of them. The section owns its own reads for
    /// the same reason it owns its own overlay and its own invariants.
    pub(crate) fn apply_env(&mut self) -> Result<()> {
        if let Ok(raw) = std::env::var("COMEMORY_OBSERVATIONS_ENABLED") {
            self.enabled = parse_bool_env("COMEMORY_OBSERVATIONS_ENABLED", &raw)?;
        }
        if let Some(v) = env_parse::<usize>("COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES")? {
            self.max_text_bytes = v;
        }
        if let Some(v) = env_parse::<usize>("COMEMORY_OBSERVATIONS_MAX_CANDIDATES")? {
            self.max_candidates = v;
        }
        Ok(())
    }

    /// Both knobs are ceilings a capture is cut at, so a zero would silently
    /// record an observation with no text or no candidates rather than
    /// disabling capture — `enabled` is what disables it.
    pub(crate) fn validate(&self) -> Result<()> {
        for (field, env, value) in [
            (
                "observations.max_text_bytes",
                "COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES",
                self.max_text_bytes,
            ),
            (
                "observations.max_candidates",
                "COMEMORY_OBSERVATIONS_MAX_CANDIDATES",
                self.max_candidates,
            ),
        ] {
            check_knob(field, env, value, check_positive_count(value))?;
        }
        Ok(())
    }
}

/// File-overlay partial for [`ObservationsConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialObservationsConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) max_text_bytes: Option<usize>,
    pub(crate) max_candidates: Option<usize>,
}
