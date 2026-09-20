//! The activity feed's writer: who ran a command ([`Origin`]), what it was
//! called ([`command`]), and the one best-effort [`record`] every instrumented
//! core calls when it finishes.
//!
//! Recording happens at the COMMAND CORE, not at a delivery adapter: each core
//! runs exactly once per invocation whatever surface called it, so one call
//! site serves `cli`, `serve` and `mcp` and no run can be counted twice. This
//! is the rule `retrieval::pipeline::log_retrieval` already follows for
//! `retrieval_log`, and [`record`] mirrors its shape — takes the connection
//! the caller already holds, warns on failure, never propagates.

use std::time::{Duration, Instant};

use serde_json::Value;
use time::OffsetDateTime;

use crate::config::Config;
use crate::prelude::*;
use crate::store::activity::{self, NewActivityRow};
use crate::store::{Connection, memory_row};
use crate::utilities::context::Ctx;
use crate::utilities::error_code;

/// The `activity_log.source` vocabulary: which delivery surface ran the
/// command. Writers name a const; the table's `CHECK` holds the same three
/// values, so a typo fails the insert rather than inventing a fourth surface.
pub mod source {
    /// A terminal `comemory <command>` run.
    pub const CLI: &str = "cli";
    /// A `/api/v1` request against `comemory serve`.
    pub const HTTP: &str = "http";
    /// An MCP tool call against `comemory mcp`.
    pub const MCP: &str = "mcp";
}

/// The `activity_log.command` vocabulary, one const per instrumented core.
/// The values match `serve::routes::RouteEntry::command`, so a row a CLI run
/// wrote and a row its HTTP twin wrote name the same command.
pub mod command {
    /// `domains::memories::save`.
    pub const SAVE: &str = "save";
    /// `domains::memories::delete`.
    pub const DELETE: &str = "delete";
    /// `domains::memories::update`.
    pub const UPDATE: &str = "update";
    /// `domains::memories::restore`.
    pub const RESTORE: &str = "restore";
    /// `domains::retrieval::search`.
    pub const SEARCH: &str = "search";
    /// `domains::retrieval::find`.
    pub const FIND: &str = "find";
    /// `domains::retrieval::context`.
    pub const CONTEXT: &str = "context";
    /// `domains::retrieval::search_code`.
    pub const SEARCH_CODE: &str = "search-code";
    /// `domains::learning::feedback`.
    pub const FEEDBACK: &str = "feedback";
    /// `domains::sync::exchange::import`.
    pub const SYNC_IMPORT: &str = "sync.import";
    /// `domains::code::index_code`.
    pub const INDEX_CODE: &str = "index-code";
}

/// Who is running this command, and whether their runs are recorded at all.
///
/// Carried by [`crate::utilities::context::Ctx`] so a core never learns which
/// surface called it: it hands its `Ctx`'s origin to [`record`] and that is
/// the whole of its involvement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// A [`source`] const.
    pub source: &'static str,
    /// The caller's self-declared label, `None` when it declared none —
    /// never inferred from anything else.
    pub actor: Option<String>,
    /// Whether runs under this origin are recorded. `false` for a
    /// `--read-only` server (which writes nothing to the store, telemetry
    /// included) and when `activity.enabled` is off.
    pub enabled: bool,
    /// Whether a recorded row carries its per-command summary
    /// (`activity.summaries`).
    pub summaries: bool,
}

impl Origin {
    /// A terminal run: the `actor` is whatever `COMEMORY_ACTOR` named, which
    /// only an invoking wrapper or agent can know.
    #[must_use]
    pub fn cli(cfg: &Config) -> Self {
        Self::new(source::CLI, cfg.activity.actor.clone(), cfg)
    }

    /// An HTTP request: the `actor` is its `User-Agent`, bounded so a long
    /// header cannot widen a row without limit.
    #[must_use]
    pub fn http(cfg: &Config, user_agent: Option<&str>) -> Self {
        Self::new(source::HTTP, user_agent.map(bounded_actor), cfg)
    }

    /// An MCP tool call: the `actor` is the `clientInfo` the host sent at
    /// `initialize`, `None` when it sent none.
    #[must_use]
    pub fn mcp(cfg: &Config, client: Option<&str>) -> Self {
        Self::new(source::MCP, client.map(bounded_actor), cfg)
    }

    /// Stop recording under this origin, whatever the config says — what a
    /// `--read-only` server sets, since read-only means no store write at
    /// all, telemetry included.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.enabled = false;
        self
    }

    fn new(source: &'static str, actor: Option<String>, cfg: &Config) -> Self {
        Self {
            source,
            actor,
            enabled: cfg.activity.enabled,
            summaries: cfg.activity.summaries,
        }
    }
}

/// Longest `actor` label a row stores. A `User-Agent` can run to kilobytes;
/// the feed only needs enough of one to tell two callers apart.
const ACTOR_MAX_CHARS: usize = 120;

/// Trim and bound a declared label, on char boundaries so a multi-byte name
/// cannot be cut mid-character.
fn bounded_actor(raw: &str) -> String {
    raw.trim().chars().take(ACTOR_MAX_CHARS).collect()
}

/// Longest free text a summary carries — a query, a memory title. These are
/// the most useful things the feed can show and the most open-ended, so the
/// bound lives here rather than at each call site.
const TEXT_MAX_CHARS: usize = 200;

/// Bound caller-supplied text for a summary, on char boundaries.
#[must_use]
pub fn bounded_text(raw: &str) -> String {
    raw.chars().take(TEXT_MAX_CHARS).collect()
}

/// What a core finished with: its summary, or the error it failed with.
pub enum Outcome<'a> {
    /// The core returned `Ok`; the value is its bounded summary.
    Ok(&'a Value),
    /// The core returned `Err`.
    Failed(&'a Error),
}

/// Record one command run. Best-effort in every direction: a disabled feed,
/// an unformattable timestamp, an unserializable summary or a failing insert
/// each warn (or return quietly) and leave the caller's own result untouched —
/// a command must never fail because its telemetry did.
pub fn record(
    conn: &Connection,
    origin: &Origin,
    command: &'static str,
    elapsed: Duration,
    outcome: &Outcome<'_>,
    repo: Option<&str>,
) {
    if !origin.enabled {
        return;
    }
    let at = match memory_row::iso_format(OffsetDateTime::now_utc()) {
        Ok(at) => at,
        Err(e) => {
            tracing::warn!(error = %e, command, "activity row skipped: timestamp format failed");
            return;
        }
    };
    let (ok, error_code, summary) = match outcome {
        Outcome::Ok(summary) => (true, None, origin.summaries.then_some(*summary)),
        Outcome::Failed(e) => {
            let (slug, _class) = error_code::classify(e);
            (false, Some(slug), None)
        }
    };
    let summary = match summary.map(serde_json::to_string).transpose() {
        Ok(text) => text,
        Err(e) => {
            tracing::warn!(error = %e, command, "activity summary dropped: serialization failed");
            None
        }
    };
    let row = NewActivityRow {
        at: &at,
        command,
        source: origin.source,
        actor: origin.actor.as_deref(),
        repo,
        duration_ms: i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        ok,
        error_code,
        summary: summary.as_deref(),
    };
    if let Err(e) = activity::insert(conn, &row) {
        tracing::warn!(error = %e, command, "activity row not written");
    }
}

/// [`record`] for a core that has a [`Ctx`] rather than a bare connection —
/// what every instrumented core actually calls, once, as its last act.
///
/// Takes the origin by clone before borrowing the connection: `Ctx` owns
/// both, and the borrow checker will not lend them out at once. The clone is
/// two `bool`s, a `&'static str` and at most a 120-char label.
///
/// Recording never creates a database: a disabled origin returns first, and
/// so does a data dir with no `comemory.db` yet. That second guard is what
/// keeps the must-not-create-the-db invariant intact for a command that
/// failed before it ever opened the store — `index-code` against a
/// non-git path is the case the repo already tests for.
pub fn record_in(
    ctx: &mut Ctx<'_>,
    command: &'static str,
    started: Instant,
    outcome: &Outcome<'_>,
    repo: Option<&str>,
) {
    if !ctx.origin.enabled || !ctx.paths.db_path().exists() {
        return;
    }
    let origin = ctx.origin.clone();
    let elapsed = started.elapsed();
    match ctx.conn() {
        Ok(conn) => record(conn, &origin, command, elapsed, outcome, repo),
        Err(e) => tracing::warn!(error = %e, command, "activity row skipped: no connection"),
    }
}

#[cfg(test)]
#[path = "tests/activity.rs"]
mod tests;
