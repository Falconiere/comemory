//! `project activity` / `GET /api/v1/projects/{id}/activity` (#331): a
//! keyset page over one project's append-only `project_activity_events`,
//! ported from the platform's `listProjectActivity`. `order` is `desc`
//! (newest first, the default) or `asc`; both walk `(created_at, id)`, ties
//! broken by id, with the same `<epochMillis>:<uuid>` cursor as
//! `project list`, so a walk under concurrent writes neither repeats nor
//! skips a row that existed when it began.
//!
//! [`EventView`] and [`split`] are the wire view and page arithmetic the
//! cross-project feed (#332) reuses, as the platform shares
//! `project-activity-view.ts` and `project-activity-keyset.ts`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::keyset::{self, Positioned};
use crate::domains::projects::limits;
use crate::domains::projects::list::vocabulary;
use crate::domains::projects::timestamp::iso;
use crate::prelude::*;
use crate::store::project_activity::{self, ActivityPage, ActivityRow};
use crate::store::project_read;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// The two walk directions; the first is the default.
pub const ORDERS: &[&str] = &["desc", "asc"];
/// Events per page when the caller names no limit (the platform's
/// `ACTIVITY_PAGE_DEFAULT`).
const DEFAULT_LIMIT: i64 = 50;
/// Largest page a caller may ask for.
const MAX_LIMIT: i64 = 200;

/// `project activity` / `GET /api/v1/projects/{id}/activity` request.
#[derive(Deserialize, Debug, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// The project's UUID.
    pub id: String,
    /// Page size, 1–200; 50 when absent.
    #[serde(default)]
    pub limit: Option<i64>,
    /// The previous page's `nextCursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    #[serde(default)]
    pub order: Option<String>,
}

/// One page of a project's activity.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// In the page's order.
    pub events: Vec<EventView>,
    /// The cursor for the next page; `null` on the last one.
    pub next_cursor: Option<String>,
}

/// One activity event on the wire, the platform's
/// `ProjectActivityEventViewSchema` key for key.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    /// Event UUID.
    pub id: String,
    /// Owning project.
    pub project_id: String,
    /// `user` or `project_agent`.
    pub actor_principal_type: String,
    /// The actor's principal id.
    pub actor_principal_id: String,
    /// What happened (`project.created`, …).
    pub event_type: String,
    /// Kind of entity it happened to.
    pub entity_type: String,
    /// Id of that entity.
    pub entity_id: String,
    /// The event's JSON object.
    pub payload: Map<String, Value>,
    /// ISO-8601.
    pub created_at: String,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        Verb::ActivityRead
    }

    /// Read one page of `self.id`'s events.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Response> {
        let id = uuid::canonical(&self.id)
            .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
        let limit = limits::page_within(self.limit, DEFAULT_LIMIT, MAX_LIMIT)?;
        let cursor = self.cursor.as_deref().map(keyset::decode).transpose()?;
        let ascending = vocabulary("order", self.order.as_deref(), ORDERS)? == Some("asc");
        let conn = ctx.conn()?;
        if project_read::project(conn, &id)?.is_none() {
            return Err(ProjectError::ProjectNotFound {
                project_id: self.id,
            }
            .into());
        }
        let rows = project_activity::page(
            conn,
            &ActivityPage {
                project_id: &id,
                ascending,
                after: cursor.as_ref().map(|c| (c.at_ms, c.id.as_str())),
                limit: limit + 1,
            },
        )?;
        let (rows, next_cursor) = split(rows, limit);
        Ok(Response {
            events: rows.into_iter().map(view).collect::<Result<_>>()?,
            next_cursor,
        })
    }
}

/// A `limit + 1` read as the page a caller asked for and the cursor that
/// continues it: `None` when the extra row was not there. A negative `limit`
/// yields an empty page with no cursor.
#[must_use]
pub fn split<R: Positioned>(mut rows: Vec<R>, limit: i64) -> (Vec<R>, Option<String>) {
    let has_more = rows.len() as i64 > limit;
    // A non-positive limit is refused upstream; here it reads as an empty page.
    rows.truncate(usize::try_from(limit).unwrap_or(0));
    let next_cursor = rows.last().filter(|_| has_more).map(|last| {
        let (at_ms, id) = last.position();
        keyset::encode(at_ms, id)
    });
    (rows, next_cursor)
}

impl Positioned for ActivityRow {
    fn position(&self) -> (i64, &str) {
        (self.created_at, &self.id)
    }
}

/// One stored event as its view. A payload that is not a JSON object reads
/// as `{}` with a warning naming the row, never the value, as the platform's
/// `parseStoredJson` does; a `created_at` outside the representable range is
/// a corrupt row, refused as the charter view refuses one.
pub fn view(row: ActivityRow) -> Result<EventView> {
    let payload = if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&row.payload) {
        map
    } else {
        tracing::warn!(
            row_id = %row.id,
            column = "project_activity_events.payload",
            "stored payload is not a JSON object; reading as {{}}"
        );
        Map::new()
    };
    let created_at = iso(row.created_at).ok_or_else(|| {
        Error::from(ProjectError::Invariant {
            invariant: "project_timestamp_range".to_string(),
            message: format!(
                "project_activity_events.created_at of {} is outside the representable range",
                row.id
            ),
        })
    })?;
    Ok(EventView {
        id: row.id,
        project_id: row.project_id,
        actor_principal_type: row.actor_principal_type,
        actor_principal_id: row.actor_principal_id,
        event_type: row.event_type,
        entity_type: row.entity_type,
        entity_id: row.entity_id,
        payload,
        created_at,
    })
}

#[cfg(test)]
#[path = "tests/activity_page.rs"]
mod tests;
