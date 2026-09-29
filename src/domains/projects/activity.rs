//! The one `project_activity_events` writer every project mutation shares:
//! called inside the mutation's own transaction, so the event and the state
//! change commit together or not at all. Never `activity_log` — that is
//! command telemetry, recorded by the core instrumentation.

use serde::Serialize;

use crate::domains::projects::principal::Principal;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_activity::{self, NewProjectEvent};
use crate::utilities::uuid;

/// What happened, to what, with which platform payload.
pub struct Event<'a, P: Serialize> {
    /// Owning project.
    pub project_id: &'a str,
    /// The platform's event name (`project.created`, …).
    pub event_type: &'a str,
    /// Kind of entity it happened to (`project`, …).
    pub entity_type: &'a str,
    /// Id of that entity.
    pub entity_id: &'a str,
    /// The payload; a struct, so its keys keep the platform's order.
    pub payload: &'a P,
}

/// Append `event` by `actor` at `at_ms` and return the event's new id.
pub fn record<P: Serialize>(
    conn: &Connection,
    actor: &Principal,
    event: &Event<'_, P>,
    at_ms: i64,
) -> Result<String> {
    let id = uuid::new_v4()?;
    let payload = serde_json::to_string(event.payload)?;
    project_activity::insert(
        conn,
        &NewProjectEvent {
            id: &id,
            project_id: event.project_id,
            actor_type: actor.principal_type.as_str(),
            actor_id: &actor.id,
            event_type: event.event_type,
            entity_type: event.entity_type,
            entity_id: event.entity_id,
            payload: &payload,
            at_ms,
        },
    )?;
    Ok(id)
}
