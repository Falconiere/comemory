//! `project create` / `POST /api/v1/projects`: a draft charter, ported from
//! the platform's `project-charter-service.ts`. One transaction writes the
//! project, its repositories, its project-level criteria and exactly one
//! `project.created` activity event; any refusal or failure rolls all of it
//! back. The ordinary `activity_log` telemetry row is written afterwards by
//! the core instrumentation, never by the transaction.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::domains::projects::activity::{self, Event};
use crate::domains::projects::charter::{self, Charter};
use crate::domains::projects::limits;
use crate::domains::projects::principal::Principal;
use crate::domains::projects::slug::{base_slug, disambiguate};
use crate::domains::projects::timestamp::now_ms;
use crate::domains::projects::view::{self, ProjectView};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::connection::write_transaction;
use crate::store::project_read;
use crate::store::projects::{NewProject, ProjectInsert, insert_project, insert_relations};
use crate::utilities::activity::{Outcome, command, record_in};
use crate::utilities::context::Ctx;
use crate::utilities::project_error::{ProjectError, RequestEdge};
use crate::utilities::uuid;

/// Bounds the slug retry loop, as the platform does.
const MAX_SLUG_ATTEMPTS: usize = 25;

/// `project create` / `POST /api/v1/projects` request: the platform's body,
/// minus `idempotencyKey` (#327). `workspaceId` is accepted and ignored.
#[derive(Deserialize, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    /// Client-generated UUID; minted when absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The platform's workspace id: accepted and ignored, because the data
    /// directory is the workspace.
    #[serde(default)]
    pub workspace_id: Option<serde_json::Value>,
    /// Display name, 1–120 characters.
    pub name: String,
    /// 2–10 characters matching `^[A-Z][A-Z0-9]*$`, unique.
    pub key_prefix: String,
    /// The finite outcome, 1–2000 characters.
    pub outcome: String,
    /// Up to 50 project-level success criteria, each 1–500 characters.
    #[serde(default)]
    pub success_criteria: Vec<String>,
    /// Up to 50 constraints, each 1–500 characters.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Up to 50 non-goals, each 1–500 characters.
    #[serde(default)]
    pub non_goals: Vec<String>,
    /// Up to 50 canonical `owner/name` repositories.
    #[serde(default)]
    pub repositories: Vec<String>,
    /// The lead's principal id; the actor's when absent.
    #[serde(default)]
    pub lead_user_id: Option<String>,
    /// `YYYY-MM-DD` (UTC midnight) or an RFC 3339 timestamp.
    #[serde(default)]
    pub target_date: Option<String>,
}

/// The created project.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Its charter view.
    pub project: ProjectView,
}

/// The platform's `project.created` payload, keys in its order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedPayload<'a> {
    name: &'a str,
    key_prefix: &'a str,
    repository_count: usize,
    criteria_count: usize,
}

/// Create a draft project as `actor`.
pub fn run(ctx: &mut Ctx<'_>, actor: &Principal, req: Request) -> Result<Response> {
    let started = Instant::now();
    let result = create(ctx, actor, req);
    let summary = result
        .as_ref()
        .map(|r| serde_json::json!({"id": r.project.id, "keyPrefix": r.project.key_prefix}));
    let outcome = match &summary {
        Ok(value) => Outcome::Ok(value),
        Err(e) => Outcome::Failed(e),
    };
    record_in(ctx, command::PROJECT_CREATE, started, &outcome, None);
    result
}

/// Validate, then write everything in one immediate transaction.
fn create(ctx: &mut Ctx<'_>, actor: &Principal, req: Request) -> Result<Response> {
    let charter = charter::validate(req, actor)?;
    let tx = write_transaction(ctx.conn()?)?;
    // Stamped once the writer lock is held, so commit order and `created_at`
    // order agree for every writer of this database.
    let at_ms = now_ms();
    let constraints = serde_json::to_string(&charter.constraints)?;
    let non_goals = serde_json::to_string(&charter.non_goals)?;
    let mut project = NewProject {
        id: &charter.id,
        slug: "",
        key_prefix: &charter.key_prefix,
        name: &charter.name,
        outcome: &charter.outcome,
        constraints: &constraints,
        non_goals: &non_goals,
        lead_type: charter.lead.principal_type.as_str(),
        lead_id: &charter.lead.id,
        target_date: charter.target_date,
        creator_type: actor.principal_type.as_str(),
        creator_id: &actor.id,
        at_ms,
    };
    let slug = insert_with_unique_slug(&tx, &project)?;
    project.slug = &slug;
    write_relations(&tx, &project, &charter)?;
    let payload = CreatedPayload {
        name: &charter.name,
        key_prefix: &charter.key_prefix,
        repository_count: charter.repositories.len(),
        criteria_count: charter.success_criteria.len(),
    };
    let event = Event {
        project_id: &charter.id,
        event_type: "project.created",
        entity_type: "project",
        entity_id: &charter.id,
        payload: &payload,
    };
    activity::record(&tx, actor, &event, at_ms)?;
    let view = created_view(&tx, &charter.id)?;
    tx.commit()?;
    Ok(Response { project: view })
}

/// Insert the row under the plain slug, then `-2`, `-3`, … on each real
/// slug collision; a taken key prefix or id refuses the whole create.
fn insert_with_unique_slug(conn: &Connection, project: &NewProject<'_>) -> Result<String> {
    let base = base_slug(project.name);
    for attempt in 0..MAX_SLUG_ATTEMPTS {
        let slug = disambiguate(&base, attempt);
        let row = NewProject {
            slug: &slug,
            ..*project
        };
        match insert_project(conn, &row)? {
            ProjectInsert::Inserted => return Ok(slug),
            ProjectInsert::SlugTaken => {}
            ProjectInsert::KeyPrefixTaken => {
                let message = "keyPrefix is already used in this workspace";
                let refusal = ProjectError::invalid_field("keyPrefix", "duplicate");
                return Err(refusal.at(RequestEdge::Invariant, Some(message)).into());
            }
            ProjectInsert::IdTaken => return Err(limits::invariant("id", "duplicate")),
        }
    }
    let message = "Could not derive a unique project slug";
    let refusal = ProjectError::invalid_field("name", "slug_exhausted");
    Err(refusal.at(RequestEdge::Invariant, Some(message)).into())
}

/// The repositories and project-level criteria (each criterion a new UUID).
fn write_relations(conn: &Connection, project: &NewProject<'_>, charter: &Charter) -> Result<()> {
    let criteria = charter
        .success_criteria
        .iter()
        .map(|description| Ok((uuid::new_v4()?, description.clone())))
        .collect::<Result<Vec<_>>>()?;
    insert_relations(conn, project, &charter.repositories, &criteria)
}

/// The view of the row this transaction just wrote.
fn created_view(conn: &Connection, id: &str) -> Result<ProjectView> {
    let row = project_read::project(conn, id)?;
    let view = row.map(|row| view::load(conn, vec![row])).transpose()?;
    view.and_then(|mut views| views.pop()).ok_or_else(|| {
        ProjectError::Invariant {
            invariant: "project_created_row".to_string(),
            message: format!("project {id} vanished inside its own creation transaction"),
        }
        .into()
    })
}

#[cfg(test)]
#[path = "tests/create.rs"]
mod tests;
