//! `maintenance::activity::{Request, run}` — the shared middle of
//! `GET /api/v1/activity` and its SSE twin's replay: one filtered,
//! newest-first page of recorded command runs plus the per-command rollups
//! over the same filter.
//!
//! A synthetic console read with no CLI counterpart, like
//! [`super::overview`]: the feed is an HTTP surface in this iteration.
//!
//! **Must-not-create-the-db invariant** (the rule [`super::stats`] and
//! [`super::gc`] keep): being asked what has happened must not materialize a
//! database. On a data dir with no `comemory.db`, [`run`] never calls
//! [`Ctx::conn`] and answers with an empty page and no rollups.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::prelude::*;
use crate::store::activity::{self, ActivityFilter, ActivityRow};
use crate::store::activity_rollups::{self, ActivityRollup};
use crate::utilities::context::Ctx;

/// Page size when the caller names none.
const DEFAULT_LIMIT: usize = 50;
/// Largest page the route will serve, whatever the caller asks for.
pub const MAX_LIMIT: usize = 200;

/// `GET /api/v1/activity` request. Every filter is optional; an empty
/// request is "everything, newest first".
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Only runs scoped to this repo label.
    #[serde(default)]
    pub repo: Option<String>,
    /// Only this command (`save`, `find`, `sync.import`, …).
    #[serde(default)]
    pub command: Option<String>,
    /// Only this delivery surface (`cli`, `http`, `mcp`).
    #[serde(default)]
    pub source: Option<String>,
    /// Only this caller label.
    #[serde(default)]
    pub actor: Option<String>,
    /// Only runs at or after this RFC3339 UTC timestamp.
    #[serde(default)]
    pub since: Option<String>,
    /// Page size; clamped to [`MAX_LIMIT`], defaulting to
    /// [`DEFAULT_LIMIT`]. `0` means "all", also clamped.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Rows to skip before the page starts.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// One recorded run as the feed reports it: the stored row with its summary
/// parsed back into JSON.
#[derive(Serialize, Debug)]
pub struct Item {
    /// Monotonic row id; also the SSE stream's cursor.
    pub id: i64,
    /// RFC3339 UTC time the run finished.
    pub at: String,
    /// The command that ran.
    pub command: String,
    /// `cli` | `http` | `mcp`.
    pub source: String,
    /// The caller's self-declared label, when it declared one.
    pub actor: Option<String>,
    /// Repo label the run was scoped to.
    pub repo: Option<String>,
    /// Wall-clock duration of the core call.
    pub duration_ms: i64,
    /// Whether the core returned `Ok`.
    pub ok: bool,
    /// Error slug when `ok` is false.
    pub error_code: Option<String>,
    /// The bounded per-command summary; `null` when summaries were off (or
    /// when the stored text could not be parsed — see [`Item::from_row`]).
    pub summary: Option<Value>,
}

impl Item {
    /// Parse one stored row. A summary that will not parse is dropped with a
    /// warn rather than failing the whole page: the feed's job is to report
    /// what happened, and one malformed row must not hide the rest.
    pub(crate) fn from_row(row: ActivityRow) -> Self {
        let summary = row
            .summary
            .and_then(|text| match serde_json::from_str(&text) {
                Ok(value) => Some(value),
                Err(e) => {
                    tracing::warn!(error = %e, id = row.id, "activity summary dropped: unparsable");
                    None
                }
            });
        Self {
            id: row.id,
            at: row.at,
            command: row.command,
            source: row.source,
            actor: row.actor,
            repo: row.repo,
            duration_ms: row.duration_ms,
            ok: row.ok,
            error_code: row.error_code,
            summary,
        }
    }
}

/// `GET /api/v1/activity` response.
#[derive(Serialize, Debug, Default)]
pub struct Response {
    /// The requested page, newest first.
    pub items: Vec<Item>,
    /// Rows matching the filter, across every page.
    pub total: usize,
    /// Per-command counts and durations over the same filter, busiest first.
    pub rollups: Vec<ActivityRollup>,
    /// The newest row id in the table, or `0` when there is none — what
    /// a client hands `GET /activity/events?after_id=` to stream only what
    /// happens next.
    pub cursor: i64,
}

/// Read one filtered page plus its rollups.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    // The invariant: no database, no read — and no database created either.
    if !ctx.paths.db_path().exists() {
        return Ok(Response::default());
    }
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = req.offset.unwrap_or(0);
    let filter = ActivityFilter {
        repo: req.repo.as_deref(),
        command: req.command.as_deref(),
        source: req.source.as_deref(),
        actor: req.actor.as_deref(),
        since: req.since.as_deref(),
        after_id: None,
    };
    let conn = ctx.conn()?;
    let (rows, total) = activity::list(conn, &filter, limit, offset)?;
    let rollups = activity_rollups::rollups(conn, &filter)?;
    // The newest id in the table, not in this page: it is the "start from
    // now" marker a client hands the stream, and a paged or filtered read
    // must not hand back an older one and replay what it already saw.
    let cursor = activity::newest_id(conn)?;
    Ok(Response {
        items: rows.into_iter().map(Item::from_row).collect(),
        total,
        rollups,
        cursor,
    })
}

#[cfg(test)]
#[path = "tests/activity.rs"]
mod tests;
