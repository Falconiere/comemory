//! `project changes` / `GET /api/v1/projects/changes` (#324): the body-free
//! change feed a connected platform polls to keep its invalidate-and-refetch
//! nudge once the engine owns project data. A frame names a position, a
//! project, an event and an op — never a charter, proposal, work item,
//! evidence or activity body.
//!
//! Rows are written by [`record_change`] (called from
//! [`crate::domains::projects::activity::record`] for every mutation, and
//! once by a transfer import, #342) and [`record_deletion`], all on the
//! mutation's own transaction. Retention is unbounded; a cursor
//! the feed cannot continue contiguously — below the oldest retained row or
//! past the head — is refused, never answered with a page that skips.

use serde::{Deserialize, Serialize};

use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::limits;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_changes::{self, NewChange};
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;

/// `op` of a committed mutation.
pub const CHANGED: &str = "changed";
/// `op` of a hard deletion.
pub const DELETED: &str = "deleted";
/// Frames per page when the caller names no limit.
const DEFAULT_LIMIT: i64 = 100;
/// Largest page a caller may ask for; the hosted relay asks for 500.
const MAX_LIMIT: i64 = 1000;

/// `project changes` / `GET /api/v1/projects/changes` request.
#[derive(Deserialize, Debug, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// Frames after this `seq`; `0` (the start) when absent.
    #[serde(default)]
    pub after: Option<i64>,
    /// Page size, 1–1000; 100 when absent.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// One body-free notification.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ChangeFrame {
    /// Feed position; the next read's `after`.
    pub seq: i64,
    /// Always `project`.
    pub entity: &'static str,
    /// The project that changed.
    pub project_id: String,
    /// The activity event recorded with the change; the project id for a
    /// deletion and for a transfer import (#342), neither of which leaves an
    /// activity event to name.
    pub event_id: String,
    /// `changed` or `deleted`.
    pub op: String,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Vec<ChangeFrame>;

    fn verb(&self) -> Verb {
        Verb::ProjectChanges
    }

    /// Frames after `self.after`, ascending by `seq`.
    fn execute(self, ctx: &mut Ctx<'_>, _actor: &Actor) -> Result<Vec<ChangeFrame>> {
        let after = self.after.unwrap_or(0);
        if after < 0 {
            return Err(ProjectError::invalid_field("after", "invalid").into());
        }
        let limit = limits::page_within(self.limit, DEFAULT_LIMIT, MAX_LIMIT)?;
        let conn = ctx.conn()?;
        // Valid cursors run from just below the oldest retained row to the head.
        let (floor, head) =
            project_changes::bounds(conn)?.map_or((0, 0), |(oldest, head)| (oldest - 1, head));
        if after > head {
            return Err(limits::invariant("after", "cursor_ahead"));
        }
        if after < floor {
            return Err(limits::invariant("after", "cursor_expired"));
        }
        Ok(project_changes::page(conn, after, limit)?
            .into_iter()
            .map(|row| ChangeFrame {
                seq: row.seq,
                entity: "project",
                project_id: row.project_id,
                event_id: row.event_id,
                op: row.op,
            })
            .collect())
    }
}

/// Append the `changed` row for the activity event `event_id` on `conn`, the
/// mutation's own transaction. The activity writer calls it for every
/// mutation; an import (#342) calls it once with the project's own id as
/// `event_id`, because an import writes no activity event (its rows are
/// transferred content), yet a connected console must still learn the
/// project arrived.
pub(crate) fn record_change(
    conn: &Connection,
    project_id: &str,
    event_id: &str,
    entity_type: &str,
    at_ms: i64,
) -> Result<()> {
    project_changes::append(
        conn,
        &NewChange {
            project_id,
            event_id,
            entity_type,
            op: CHANGED,
            at_ms,
        },
    )
}

/// Append the `deleted` row for `project_id` on `conn`, inside the hard
/// deletion's transaction (#320). Its `event_id` is the project's own id, as
/// on the platform, because the deletion erases the audit trail; the row has
/// no foreign key, so it outlives the project.
pub fn record_deletion(conn: &Connection, project_id: &str, at_ms: i64) -> Result<()> {
    project_changes::append(
        conn,
        &NewChange {
            project_id,
            event_id: project_id,
            entity_type: "project",
            op: DELETED,
            at_ms,
        },
    )
}

#[cfg(test)]
#[path = "tests/changes.rs"]
mod tests;
