//! The one `project_activity_events` writer every project mutation shares:
//! called inside the mutation's own transaction, so the event, its
//! `project_changes` feed row (#324) and the state change commit together or
//! not at all. Never `activity_log` — that is command telemetry, recorded by
//! the core instrumentation.
//!
//! Because every mutation passes through it, it also reports whether the
//! project is bound by a transfer (#342): a mutation core puts
//! [`Recorded::local_only`] in its response's `warnings`, so a change to a
//! transferred project says it stays local.

use serde::Serialize;

use crate::domains::projects::authority::Actor;
use crate::domains::projects::changes;
use crate::domains::projects::local_only::{LocalOnly, local_only};
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

/// What recording an event found.
#[must_use = "a mutation of a bound project must surface `local_only` as a warning"]
#[derive(Debug)]
pub struct Recorded {
    /// The new event's id.
    pub id: String,
    /// Set when the project is bound by a transfer: this change stays here.
    pub local_only: Option<LocalOnly>,
}

/// Append `event` by the admitted `actor` at `at_ms`, and its `changed` feed
/// row, and report the new event's id and whether the project is bound.
pub fn record<P: Serialize>(
    conn: &Connection,
    actor: &Actor,
    event: &Event<'_, P>,
    at_ms: i64,
) -> Result<Recorded> {
    let actor = actor.principal();
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
    changes::record_change(conn, event.project_id, &id, event.entity_type, at_ms)?;
    let local_only = local_only(conn, event.project_id)?;
    Ok(Recorded { id, local_only })
}
