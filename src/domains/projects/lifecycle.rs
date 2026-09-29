//! `project archive|restore|pause|resume` / `POST /api/v1/projects/{id}/
//! {archive,restore,pause,resume}` (#328): the four lead-only lifecycle
//! commands, ported from the platform's `project-lifecycle-service.ts` as its
//! one generic command with a four-way [`Kind`]. Each loads the row, checks
//! `expectedVersion`, checks the transition, writes a version-guarded patch
//! and records one `project.<verb>d` event, all under the command's receipt
//! (#327) in one immediate transaction. The verbs are human-only, so an agent
//! is refused by [`crate::domains::projects::authority::run`] before this
//! core touches the store.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domains::projects::activity::{self, Event};
use crate::domains::projects::authority::{Actor, Command, Verb, sealed};
use crate::domains::projects::limits::{self, RATIONALE_MAX};
use crate::domains::projects::receipt::{self, Applied, Keyed, Ran};
use crate::domains::projects::timestamp::now_ms;
use crate::domains::projects::view::{self, ProjectView};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::project_read::{self, ProjectRow};
use crate::store::projects::{ArchivedAt, LifecyclePatch, update_lifecycle};
use crate::utilities::activity::command;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;
use crate::utilities::uuid;

/// Which lifecycle command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Set `archived_at`: hide the project without deleting its history.
    Archive,
    /// Clear `archived_at` on an archived, non-terminal project.
    Restore,
    /// `active` → `paused`, with a reason.
    Pause,
    /// `paused` → `active`.
    Resume,
}

impl Kind {
    /// All four, in the platform's order.
    pub const ALL: [Self; 4] = [Self::Archive, Self::Restore, Self::Pause, Self::Resume];

    /// The verb the envelope must admit.
    #[must_use]
    pub fn verb(self) -> Verb {
        match self {
            Self::Archive => Verb::Archive,
            Self::Restore => Verb::Restore,
            Self::Pause => Verb::Pause,
            Self::Resume => Verb::Resume,
        }
    }

    /// The CLI verb and the route's last path segment.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Restore => "restore",
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }

    /// The receipt's command type and the telemetry command, `project.<verb>`.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::Archive => command::PROJECT_ARCHIVE,
            Self::Restore => command::PROJECT_RESTORE,
            Self::Pause => command::PROJECT_PAUSE,
            Self::Resume => command::PROJECT_RESUME,
        }
    }

    /// The platform's activity event name.
    #[must_use]
    pub fn event_type(self) -> &'static str {
        match self {
            Self::Archive => "project.archived",
            Self::Restore => "project.restored",
            Self::Pause => "project.paused",
            Self::Resume => "project.resumed",
        }
    }
}

/// The platform's lifecycle body. `workspaceId` is accepted and ignored.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Body {
    /// The platform's workspace id: accepted and ignored.
    #[serde(default)]
    pub workspace_id: Option<Value>,
    /// The caller's retry key, 1–200 UTF-16 units, scoped to the principal.
    pub idempotency_key: String,
    /// The `version` the caller last read, at least 1.
    pub expected_version: i64,
    /// Why, up to 4000 UTF-16 units; required and non-blank for `pause`.
    #[serde(default)]
    pub reason: Option<String>,
}

/// One lifecycle command against project `id`.
#[derive(Debug)]
pub struct Request {
    /// Which command.
    pub kind: Kind,
    /// The project's UUID, from the path or the CLI argument.
    pub id: String,
    /// The platform's body.
    pub body: Body,
}

/// The project after the command.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Response {
    /// Its charter view.
    pub project: ProjectView,
}

/// The platform's lifecycle event payload.
#[derive(Serialize)]
struct ReasonPayload<'a> {
    reason: Option<&'a str>,
}

impl sealed::Sealed for Request {}

impl Command for Request {
    type Response = Response;

    fn verb(&self) -> Verb {
        self.kind.verb()
    }

    /// Run the command as `actor`, or replay its receipt, then record the
    /// telemetry row.
    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<Response> {
        let started = Instant::now();
        let kind = self.kind;
        let ran = transition(ctx, actor, self);
        receipt::record(
            ctx,
            kind.command(),
            started,
            ran,
            |r| json!({"id": r.project.id, "version": r.project.version}),
        )
    }
}

/// Validate without the store, then load, check and patch under the receipt.
fn transition(ctx: &mut Ctx<'_>, actor: &Actor, req: Request) -> Result<Ran<Response>> {
    let id = uuid::canonical(&req.id)
        .ok_or_else(|| Error::from(ProjectError::invalid_field("projectId", "invalid")))?;
    validate(req.kind, &req.body)?;
    // The platform digests `{expectedVersion, reason}`; the engine adds the
    // project, because its receipts are scoped to principal and key only.
    let digested = json!({
        "projectId": id,
        "expectedVersion": req.body.expected_version,
        "reason": req.body.reason,
    });
    let keyed = Keyed::new(&req.body.idempotency_key, req.kind.command(), &digested)?;
    receipt::run(ctx.conn()?, actor, &keyed, |tx| {
        let response = apply(tx, actor, &req, &id)?;
        Ok(Applied {
            response,
            project_id: id.clone(),
        })
    })
}

/// The body's rules, checked before any store access: `expectedVersion` at
/// least 1 (`400`), a reason within [`RATIONALE_MAX`] (`422`), and for
/// `pause` a reason that is present (`400`) and not blank once trimmed
/// (`422`).
fn validate(kind: Kind, body: &Body) -> Result<()> {
    if body.expected_version < 1 {
        return Err(ProjectError::invalid_field("expectedVersion", "invalid").into());
    }
    if let Some(reason) = &body.reason {
        limits::text("reason", reason, 0, RATIONALE_MAX)?;
    }
    match (kind, body.reason.as_deref()) {
        (Kind::Pause, None) => Err(ProjectError::invalid_field("reason", "required").into()),
        (Kind::Pause, Some(reason)) if reason.trim().is_empty() => {
            Err(limits::invariant("reason", "blank"))
        }
        _ => Ok(()),
    }
}

/// Load the row, check its version and transition, patch it at that
/// version, and record the event.
fn apply(tx: &Connection, actor: &Actor, req: &Request, id: &str) -> Result<Response> {
    let row = project_read::project(tx, id)?.ok_or_else(|| {
        Error::from(ProjectError::ProjectNotFound {
            project_id: req.id.clone(),
        })
    })?;
    let conflict = || {
        Error::from(ProjectError::VersionConflict {
            current_version: row.version,
        })
    };
    if row.version != req.body.expected_version {
        return Err(conflict());
    }
    // Stamped once the writer lock is held, as create does.
    let at_ms = now_ms();
    let patch = patch(req.kind, &row, at_ms)?;
    if !update_lifecycle(tx, id, row.version, &patch)? {
        return Err(conflict());
    }
    let payload = ReasonPayload {
        reason: req.body.reason.as_deref(),
    };
    let event = Event {
        project_id: id,
        event_type: req.kind.event_type(),
        entity_type: "project",
        entity_id: id,
        payload: &payload,
    };
    activity::record(tx, actor, &event, at_ms)?;
    Ok(Response {
        project: view::written(tx, id, "lifecycle")?,
    })
}

/// The row patch `kind` makes from `row`'s state, or the platform's
/// `invalid_transition` refusal. Only `restore` reaches an archived project,
/// and a terminal project can be archived but never restored, paused or
/// resumed.
fn patch(kind: Kind, row: &ProjectRow, at_ms: i64) -> Result<LifecyclePatch<'static>> {
    let archived = row.archived_at.is_some();
    let terminal = matches!(row.status.as_str(), "completed" | "canceled");
    let (status, archived_at) = match kind {
        Kind::Restore if !archived => return refuse("This project is not archived"),
        Kind::Restore if terminal => {
            return refuse("A completed or canceled project cannot be restored");
        }
        Kind::Restore => (None, ArchivedAt::Clear),
        _ if archived => return refuse("This project is archived"),
        Kind::Archive => (None, ArchivedAt::Set(at_ms)),
        Kind::Pause if row.status != "active" => {
            return refuse("Only an active project can be paused");
        }
        Kind::Pause => (Some("paused"), ArchivedAt::Keep),
        Kind::Resume if row.status != "paused" => {
            return refuse("Only a paused project can be resumed");
        }
        Kind::Resume => (Some("active"), ArchivedAt::Keep),
    };
    Ok(LifecyclePatch {
        status,
        archived_at,
        at_ms,
    })
}

/// `409 invalid_transition` with the platform's sentence.
fn refuse<T>(reason: &str) -> Result<T> {
    Err(ProjectError::InvalidTransition {
        reason: reason.to_string(),
    }
    .into())
}

#[cfg(test)]
#[path = "tests/lifecycle.rs"]
mod tests;
