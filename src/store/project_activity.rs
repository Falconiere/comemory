//! `project_activity_events` writes (#326): the append-only row every project
//! mutation adds inside its own transaction. Never `activity_log`, which is
//! command telemetry with its own writer.

use rusqlite::Connection;

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
