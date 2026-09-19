//! `domains::learning::recall_status::{Request, Output, run}` — shared middle
//! of `comemory recall-status` / `GET /api/v1/learning/recall-status` / the
//! MCP `recall_status` tool: tracked queries, verdicts, saves and the
//! still-pending queries for a repo + lower time bound. Read-only.
//!
//! `since` parses like `--since` elsewhere then is normalised to UTC (every
//! compared column is UTC `Z` text); a bad value is [`Error::Usage`].
//! [`run`] never creates `comemory.db` (like
//! `domains::maintenance::stats::run`): with none yet, it reports zeros.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::macros::time;

use crate::prelude::*;
use crate::store::{feedback, memory_row, retrieval_log};
use crate::utilities::context::Ctx;
use crate::utilities::when::{DayEdge, parse_when};

/// `comemory recall-status` / `GET /api/v1/learning/recall-status` / the MCP
/// `recall_status` tool request.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Restrict every count to this repo. Unset reports across every repo.
    #[serde(default)]
    pub repo: Option<String>,
    /// Lower time bound: an RFC3339 timestamp or a bare `YYYY-MM-DD` date,
    /// the same shapes `--since` accepts everywhere else
    /// ([`crate::utilities::when::parse_when`]). Defaults to the start of
    /// the current UTC day when omitted.
    #[serde(default)]
    pub since: Option<String>,
}

/// `recall-status` output — the same JSON object every adapter returns
/// (spec §"`recall-status` output").
#[derive(Debug, Serialize)]
pub struct Output {
    /// The repo filter this report was scoped to, echoed verbatim.
    pub repo: Option<String>,
    /// The resolved lower time bound, `iso_format`-shaped.
    pub since: String,
    /// Tracked `find`/`search`/`context`/`search-code` rows at or after
    /// `since` — pending ∪ judged.
    pub queries: u64,
    /// `feedback_events` rows recorded in the window.
    pub feedback_events: u64,
    /// Live memories created in the window (`repo`-scoped when set).
    pub saves: u64,
    /// Tracked queries in the window with no verdict yet, oldest first.
    pub pending: Vec<retrieval_log::PendingRow>,
}

/// Report tracked queries, verdicts, saves, and pending recalls for the
/// window `[since, now)`, optionally scoped to `repo`.
///
/// Never creates `comemory.db` as a side effect of asking about it (the
/// same invariant [`crate::domains::maintenance::stats::run`] keeps): on a
/// data dir with no database yet, this returns a zero report with `since`
/// resolved rather than opening [`Ctx::conn`].
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Output> {
    let since = resolve_since(req.since.as_deref())?;
    if !ctx.paths.db_path().exists() {
        return Ok(Output {
            repo: req.repo,
            since,
            queries: 0,
            feedback_events: 0,
            saves: 0,
            pending: Vec::new(),
        });
    }
    let repo = req.repo.as_deref();
    let conn = ctx.conn()?;
    let pending = retrieval_log::pending_since(&*conn, repo, &since)?;
    let queries = retrieval_log::count_since(&*conn, repo, &since)?;
    let feedback_events = feedback::events_since(&*conn, repo, &since)?;
    let saves = memory_row::count_created_since(&*conn, repo, &since)?;
    Ok(Output {
        repo: req.repo,
        since,
        queries,
        feedback_events,
        saves,
        pending,
    })
}

/// Parse an explicit `since` the same way `--since` is parsed everywhere
/// else, or default to the start of the current UTC day when omitted — the
/// "today" a caller with no window in mind expects. Either way the result is
/// `iso_format`-shaped so the store's plain string `>=` compares
/// chronologically against `retrieval_log.at` / `feedback_events.at` /
/// `memories.created_at`.
fn resolve_since(raw: Option<&str>) -> Result<String> {
    let ts = if let Some(v) = raw {
        parse_when(v, DayEdge::Start).map_err(|e| match e {
            Error::Usage(msg) => Error::Usage(format!("since: {msg}")),
            other => other,
        })?
    } else {
        let now = OffsetDateTime::now_utc();
        now.date().with_time(time!(00:00:00)).assume_utc()
    };
    memory_row::iso_format(ts.to_offset(time::UtcOffset::UTC))
}

#[cfg(test)]
#[path = "tests/recall_status.rs"]
mod tests;
