//! `[activity]` config section: the per-command activity feed behind
//! `GET /api/v1/activity` and its SSE twin.
//!
//! Its own file beside `observations.rs` and `sync.rs` for the same reason
//! those have one — a section owns its struct, its file overlay, its env
//! reads and its invariants, and `file.rs` / `validate.rs` stay at one line
//! each.

use serde::{Deserialize, Serialize};

use super::env::{env_parse, parse_bool_env};
use super::validate::check_knob;
use crate::prelude::*;

/// Recording and streaming knobs for the activity feed.
///
/// On by default, unlike `[observations]`: a row names the command, its
/// caller and its outcome — the traffic a user is already running — rather
/// than snapshotting corpus content. `summaries` is the knob that turns off
/// the per-command detail (query text, memory titles) for anyone who wants
/// the feed without it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityConfig {
    /// Whether instrumented commands record a row at all. Env:
    /// `COMEMORY_ACTIVITY_ENABLED`. Recording additionally requires a run
    /// that may write to the store, so a read-only `comemory serve` records
    /// nothing whatever this says.
    pub enabled: bool,
    /// Whether a row carries its bounded per-command JSON summary. `false`
    /// stores `NULL` there, so no query text or memory title is persisted.
    /// Env: `COMEMORY_ACTIVITY_SUMMARIES`.
    pub summaries: bool,
    /// How often the SSE handler polls for rows above its cursor, in
    /// milliseconds. Validated `> 0`. Env: `COMEMORY_ACTIVITY_STREAM_POLL_MS`.
    pub stream_poll_ms: u64,
    /// Caller label a CLI run reports as its `actor`, `None` when unset —
    /// never inferred. Env-only (`COMEMORY_ACTOR`): it identifies the wrapper
    /// or agent invoking this one process, which a config file shared across
    /// runs cannot say. HTTP and MCP callers declare themselves through
    /// `User-Agent` and `clientInfo` instead.
    #[serde(default)]
    pub actor: Option<String>,
}

impl Default for ActivityConfig {
    /// Recording on with summaries, polling twice a second — fast enough to
    /// read as live, slow enough that an idle console costs one indexed
    /// lookup per tick.
    fn default() -> Self {
        Self {
            enabled: true,
            summaries: true,
            stream_poll_ms: 500,
            actor: None,
        }
    }
}

impl ActivityConfig {
    /// Overlay the file's `[activity]` keys; absent keys leave `self`.
    pub(crate) fn apply(&mut self, p: PartialActivityConfig) {
        if let Some(v) = p.enabled {
            self.enabled = v;
        }
        if let Some(v) = p.summaries {
            self.summaries = v;
        }
        if let Some(v) = p.stream_poll_ms {
            self.stream_poll_ms = v;
        }
    }

    /// Apply the `COMEMORY_ACTIVITY_*` overrides plus `COMEMORY_ACTOR`, the
    /// outermost layer. Here rather than in `config::env` for the reason
    /// `ObservationsConfig::apply_env` is: the section owns its own reads.
    pub(crate) fn apply_env(&mut self) -> Result<()> {
        if let Ok(raw) = std::env::var("COMEMORY_ACTIVITY_ENABLED") {
            self.enabled = parse_bool_env("COMEMORY_ACTIVITY_ENABLED", &raw)?;
        }
        if let Ok(raw) = std::env::var("COMEMORY_ACTIVITY_SUMMARIES") {
            self.summaries = parse_bool_env("COMEMORY_ACTIVITY_SUMMARIES", &raw)?;
        }
        if let Some(v) = env_parse::<u64>("COMEMORY_ACTIVITY_STREAM_POLL_MS")? {
            self.stream_poll_ms = v;
        }
        if let Ok(raw) = std::env::var("COMEMORY_ACTOR") {
            let trimmed = raw.trim();
            // An empty or whitespace-only value is "no actor", not an actor
            // named "": the column stays NULL rather than recording a label
            // no reader could act on.
            self.actor = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        Ok(())
    }

    /// A zero poll interval would spin the stream task against the database
    /// with no wait at all, so it is refused rather than clamped.
    pub(crate) fn validate(&self) -> Result<()> {
        // Checked on the `u64` itself: routing it through `check_positive_count`
        // would mean a `usize` conversion that is lossy on a 32-bit target, and
        // the only invalid interval is zero.
        let bound = if self.stream_poll_ms >= 1 {
            Ok(())
        } else {
            Err("must be >= 1")
        };
        check_knob(
            "activity.stream_poll_ms",
            "COMEMORY_ACTIVITY_STREAM_POLL_MS",
            self.stream_poll_ms,
            bound,
        )
    }
}

/// File-overlay partial for [`ActivityConfig`]. `actor` is absent on purpose:
/// it is env-only (see [`ActivityConfig::actor`]), and `deny_unknown_fields`
/// makes that refusal explicit rather than silent.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialActivityConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) summaries: Option<bool>,
    pub(crate) stream_poll_ms: Option<u64>,
}

#[cfg(test)]
#[path = "tests/activity.rs"]
mod tests;
