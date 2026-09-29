//! `POST|GET /api/v1/projects`, `GET /api/v1/projects/{id}`,
//! `GET /api/v1/projects/{id}/plan`, `GET /api/v1/projects/{id}/activity`,
//! `POST /api/v1/projects/{id}/{archive,restore,pause,resume}`,
//! `POST|GET /api/v1/projects/{id}/evidence` and
//! `GET /api/v1/projects/changes` — the `domains::projects` create, list,
//! show, plan, activity-page, lifecycle, evidence and change-feed cores, on the platform's REST
//! paths so a hosted cutover
//! forwards without remapping. Every core runs under [`caller`]'s envelope,
//! which no header can change: in local mode the local agent, so `POST`
//! answers `403 project_agent_scope`, and an admitted create answers `201`,
//! as the platform does.
//!
//! Bodies and queries are parsed here rather than by axum's extractors,
//! whose rejections are plain text: every malformed input answers the
//! project `invalid_request` envelope (`400`, naming the field) instead.

use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;

use crate::domains::projects::authority::{self, Command, Envelope};
use crate::domains::projects::lifecycle::Kind;
use crate::domains::projects::{changes, create, plan, show};
use crate::serve::AppState;
use crate::serve::routes::project_activity::{self, ACTIVITY};
use crate::serve::routes::project_evidence;
use crate::serve::routes::project_lifecycle;
use crate::serve::routes::project_request::{list_field, query};
use crate::serve::routes::{RouteEntry, guard_mutating, query_response, respond};
use crate::utilities::activity::command;
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;
use crate::utilities::project_body::body;

/// `project.create`'s route command, shared by the table and its handler.
const CREATE: &str = "project.create";
/// `project.changes`'s route command, shared by the table and its handler.
const CHANGES: &str = "project.changes";
/// `project plan show`'s route command, shared by the table and its handler.
const PLAN_SHOW: &str = "project.plan.show";

/// This resource's route-table entries, appended onto [`super::table`]. The
/// list route carries the bare `project` command, the top-level clap name the
/// parity inventory matches on.
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "POST",
            path: "/projects",
            command: CREATE,
            mutating: true,
        },
        RouteEntry {
            method: "GET",
            path: "/projects",
            command: "project",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/{id}",
            command: "project.show",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/{id}/plan",
            command: PLAN_SHOW,
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/{id}/activity",
            command: ACTIVITY,
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/projects/{id}/archive",
            command: command::PROJECT_ARCHIVE,
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/projects/{id}/restore",
            command: command::PROJECT_RESTORE,
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/projects/{id}/pause",
            command: command::PROJECT_PAUSE,
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/projects/{id}/resume",
            command: command::PROJECT_RESUME,
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/projects/{id}/evidence",
            command: project_evidence::ADD,
            mutating: true,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/{id}/evidence",
            command: project_evidence::LIST,
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/changes",
            command: CHANGES,
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    let lifecycle = Kind::ALL.into_iter().fold(Router::new(), |router, kind| {
        router.route(
            &project_lifecycle::path(kind),
            project_lifecycle::route(kind),
        )
    });
    lifecycle
        .route(
            "/api/v1/projects",
            get(|State(state), q| read(state, "project", parsed(q, list_field)))
                .post(create_project),
        )
        // The static segment wins over `{id}`: `changes` is never a project id.
        .route(
            "/api/v1/projects/changes",
            get(|State(state), q| read(state, CHANGES, parsed(q, changes_field))),
        )
        .route(
            "/api/v1/projects/{id}",
            get(|State(state), Path(id)| read(state, "project.show", Ok(show::Request { id }))),
        )
        .route(
            "/api/v1/projects/{id}/plan",
            get(|State(state), Path(id)| read(state, PLAN_SHOW, Ok(plan::Request { id }))),
        )
        .route(
            "/api/v1/projects/{id}/activity",
            get(project_activity::page),
        )
        .route(
            "/api/v1/projects/{id}/evidence",
            get(project_evidence::list).post(project_evidence::add),
        )
}

/// The envelope a local-mode HTTP caller runs under: the local agent, which
/// reads but holds no human verb, so `POST /projects` and the lifecycle
/// routes answer `403 project_agent_scope` (#315). #316 makes it
/// configurable; #317 adds hosted mode's signed stamp.
pub(super) fn caller() -> Envelope {
    Envelope::local_agent()
}

/// `POST /api/v1/projects` — charter a draft project, when [`caller`] may.
async fn create_project(State(state): State<AppState>, headers: HeaderMap, raw: Bytes) -> Response {
    let started = Instant::now();
    let origin = state.http_origin(&headers);
    let permit = match guard_mutating(CREATE, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let req: create::Request = body(&raw)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn).with_origin(origin);
        authority::run(&mut ctx, &caller(), req)
    })
    .await;
    created(respond(CREATE, result, started))
}

/// The platform's `201` for a created project; a refusal passes through.
pub(super) fn created(mut response: Response) -> Response {
    if response.status() == StatusCode::OK {
        *response.status_mut() = StatusCode::CREATED;
    }
    response
}

/// A query string as `R`, each pair stored by `field`.
pub(super) fn parsed<R: Default>(
    raw: std::result::Result<Query<Vec<(String, String)>>, QueryRejection>,
    field: fn(&mut R, &str, String) -> Option<()>,
) -> crate::errors::Result<R> {
    query(raw.map(|Query(pairs)| pairs), field)
}

/// A read under [`caller`]'s envelope of a request parsed before any store
/// access, so a malformed one never opens the database: the query-string
/// reads `GET /projects` (a keyset page, newest first),
/// `GET /projects/{id}/activity` (one project's events) and
/// `GET /projects/changes` (body-free frames after a cursor), and the
/// path-only reads `GET /projects/{id}` (the charter) and
/// `GET /projects/{id}/plan` (the committed plan), which cannot be malformed
/// before the core.
pub(super) async fn read<R>(
    state: AppState,
    command: &'static str,
    req: crate::errors::Result<R>,
) -> Response
where
    R: Command + Send + 'static,
    R::Response: serde::Serialize + Send + 'static,
{
    match req {
        Ok(req) => {
            query_response(state, command, move |ctx| {
                authority::run(ctx, &caller(), req)
            })
            .await
        }
        Err(e) => respond::<()>(command, Err(e), Instant::now()),
    }
}

/// `GET /projects/changes`'s integer `after` or `limit` stored on `req`;
/// `None` refuses any other key or a non-integer value.
pub fn changes_field(req: &mut changes::Request, key: &str, value: String) -> Option<()> {
    let slot = [("after", &mut req.after), ("limit", &mut req.limit)]
        .into_iter()
        .find_map(|(name, slot)| (name == key).then_some(slot))?;
    *slot = Some(value.parse().ok()?);
    Some(())
}

#[cfg(test)]
#[path = "tests/projects.rs"]
mod tests;
