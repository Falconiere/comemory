//! `project_activity_events` (#326): the append-only row every project
//! mutation adds inside its own transaction, and the keyset page over
//! `(created_at, id)` that reads one project's rows back in either direction
//! (#331). Never `activity_log`, which is command telemetry with its own
//! writer.

use rusqlite::Connection;
use toolu_orm::core::expr::Scalar;
use toolu_orm::core::query_column::{CommonOps, NumericOps};

use super::orm;
use super::schema_project_record::{ProjectActivityEvents, project_activity_events as col};
use crate::prelude::*;

/// One event row.
pub struct NewProjectEvent<'a> {
    /// Event UUID.
    pub id: &'a str,
    /// Owning project.
    pub project_id: &'a str,
    /// Actor principal kind.
    pub actor_type: &'a str,
    /// Actor principal id.
    pub actor_id: &'a str,
    /// What happened (`project.created`, …).
    pub event_type: &'a str,
    /// Kind of entity it happened to.
    pub entity_type: &'a str,
    /// Id of that entity.
    pub entity_id: &'a str,
    /// JSON object.
    pub payload: &'a str,
    /// Epoch milliseconds.
    pub at_ms: i64,
}

/// Append `event`.
pub fn insert(conn: &Connection, event: &NewProjectEvent<'_>) -> Result<()> {
    orm::execute(
        conn,
        ProjectActivityEvents::insert()
            .set(&col::id, event.id)
            .set(&col::project_id, event.project_id)
            .set(&col::actor_principal_type, event.actor_type)
            .set(&col::actor_principal_id, event.actor_id)
            .set(&col::event_type, event.event_type)
            .set(&col::entity_type, event.entity_type)
            .set(&col::entity_id, event.entity_id)
            .set(&col::payload, event.payload)
            .set(&col::created_at, event.at_ms)
            .to_sql(),
    )?;
    Ok(())
}

/// One stored event, column for column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    /// Event UUID.
    pub id: String,
    /// Owning project.
    pub project_id: String,
    /// Actor principal kind.
    pub actor_principal_type: String,
    /// Actor principal id.
    pub actor_principal_id: String,
    /// What happened.
    pub event_type: String,
    /// Kind of entity it happened to.
    pub entity_type: String,
    /// Id of that entity.
    pub entity_id: String,
    /// Stored JSON text.
    pub payload: String,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// One page of one project's events. `after` is the decoded keyset cursor.
#[derive(Debug, Clone, Copy)]
pub struct ActivityPage<'a> {
    /// The project whose events are read.
    pub project_id: &'a str,
    /// Oldest first when `true`, newest first otherwise.
    pub ascending: bool,
    /// Rows strictly past this `(created_at, id)` in the page's direction.
    pub after: Option<(i64, &'a str)>,
    /// Rows to return.
    pub limit: i64,
}

/// Up to `page.limit` of the project's events ordered by `(created_at, id)`
/// in the page's direction, ties broken by `id` the same way.
pub fn page(conn: &Connection, page: &ActivityPage<'_>) -> Result<Vec<ActivityRow>> {
    let mut query = ProjectActivityEvents::select()
        .columns_typed(&[
            &col::id,
            &col::project_id,
            &col::actor_principal_type,
            &col::actor_principal_id,
            &col::event_type,
            &col::entity_type,
            &col::entity_id,
            &col::payload,
            &col::created_at,
        ])
        .filter(col::project_id.eq(page.project_id));
    if let Some((at_ms, id)) = page.after {
        let (id_col, id) = (Scalar::col(&col::id), Scalar::bind(id));
        query = if page.ascending {
            query.filter(
                col::created_at
                    .gt(at_ms)
                    .or(col::created_at.eq(at_ms).and(id_col.gt(id))),
            )
        } else {
            query.filter(
                col::created_at
                    .lt(at_ms)
                    .or(col::created_at.eq(at_ms).and(id_col.lt(id))),
            )
        };
    }
    query = if page.ascending {
        query
            .order_by(col::created_at.asc())
            .order_by(col::id.asc())
    } else {
        query
            .order_by(col::created_at.desc())
            .order_by(col::id.desc())
    };
    orm::query_all(conn, query.limit(page.limit).to_sql(), |r| {
        Ok(ActivityRow {
            id: r.get(0)?,
            project_id: r.get(1)?,
            actor_principal_type: r.get(2)?,
            actor_principal_id: r.get(3)?,
            event_type: r.get(4)?,
            entity_type: r.get(5)?,
            entity_id: r.get(6)?,
            payload: r.get(7)?,
            created_at: r.get(8)?,
        })
    })
}

#[cfg(test)]
#[path = "tests/project_activity.rs"]
mod tests;
