//! `project list` / `GET /api/v1/projects`: a keyset page of charters, newest
//! first, ported from the platform's `listProjects`. Archived projects are
//! left out unless `includeArchived`; a page ends with `nextCursor` when more
//! rows follow it.

use serde::{Deserialize, Serialize};

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::keyset;
use crate::domains::projects::limits;
use crate::domains::projects::view::{self, ProjectView};
use crate::prelude::*;
use crate::store::project_read::{self, ProjectPage};
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;

/// The platform's project statuses.
pub const STATUSES: &[&str] = &[
    "draft",
    "planning",
    "active",
    "paused",
    "completed",
    "canceled",
];
/// The platform's project health values.
pub const HEALTH: &[&str] = &["unknown", "on_track", "at_risk", "off_track"];

/// `project list` / `GET /api/v1/projects` request.
#[derive(Deserialize, Debug, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// Page size, 1–100; 20 when absent.
    #[serde(default)]
    pub limit: Option<i64>,
    /// The previous page's `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Only this status: draft, planning, active, paused, completed or canceled.
    #[serde(default)]
    pub status: Option<String>,
    /// Only this health: unknown, on_track, at_risk or off_track.
    #[serde(default)]
    pub health: Option<String>,
    /// Include archived projects.
    #[serde(default)]
    pub include_archived: Option<bool>,
}

/// One page.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// Newest first.
    pub projects: Vec<ProjectView>,
    /// The cursor for the next page; `null` on the last one.
    pub next_cursor: Option<String>,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::ProjectList
    }

    /// Read one page.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Response> {
        let limit = limits::page(self.limit)?;
        let cursor = self.cursor.as_deref().map(keyset::decode).transpose()?;
        let status = vocabulary("status", self.status.as_deref(), STATUSES)?;
        let health = vocabulary("health", self.health.as_deref(), HEALTH)?;
        let conn = ctx.conn()?;
        let mut rows = project_read::project_page(
            conn,
            &ProjectPage {
                status,
                health,
                include_archived: self.include_archived.unwrap_or(false),
                after: cursor.as_ref().map(|c| (c.at_ms, c.id.as_str())),
                limit: limit + 1,
            },
        )?;
        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let next_cursor = rows
            .last()
            .filter(|_| has_more)
            .map(|last| keyset::encode(last.created_at, &last.id));
        Ok(Response {
            projects: view::load(conn, rows)?,
            next_cursor,
        })
    }
}

/// `value` when it is one of `allowed`; otherwise the platform's schema-edge
/// enum refusal (`400`).
fn vocabulary<'a>(
    field: &str,
    value: Option<&'a str>,
    allowed: &[&str],
) -> Result<Option<&'a str>> {
    match value {
        Some(v) if !allowed.contains(&v) => {
            Err(ProjectError::invalid_field(field, "invalid").into())
        }
        other => Ok(other),
    }
}

#[cfg(test)]
#[path = "tests/list.rs"]
mod tests;
