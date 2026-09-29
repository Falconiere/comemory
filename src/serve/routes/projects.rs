//! `POST|GET /api/v1/projects`, `GET /api/v1/projects/{id}` and
//! `GET /api/v1/projects/changes` — the `domains::projects` create, list, show
//! and change-feed cores, on the platform's REST paths so a hosted cutover
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
use crate::domains::projects::{changes, create, show};
use crate::serve::AppState;
use crate::serve::routes::project_request::{body, list_field, query};
use crate::serve::routes::{RouteEntry, guard_mutating, query_response, respond};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// `project.create`'s route command, shared by the table and its handler.
const CREATE: &str = "project.create";
/// `project.changes`'s route command, shared by the table and its handler.
const CHANGES: &str = "project.changes";

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
            path: "/projects/changes",
            command: CHANGES,
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/projects",
            get(|State(state), q| read(state, "project", q, list_field)).post(create_project),
        )
        // The static segment wins over `{id}`: `changes` is never a project id.
        .route(
            "/api/v1/projects/changes",
            get(|State(state), q| read(state, CHANGES, q, changes_field)),
        )
        .route("/api/v1/projects/{id}", get(show_project))
}

/// The envelope a local-mode HTTP caller runs under: the local agent, which
/// reads but holds no human verb, so `POST /projects` answers `403
/// project_agent_scope` (#315). #316 makes it configurable; #317 adds hosted
/// mode's signed stamp.
fn caller() -> Envelope {
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
fn created(mut response: Response) -> Response {
    if response.status() == StatusCode::OK {
        *response.status_mut() = StatusCode::CREATED;
    }
    response
}

/// A query-string read under [`caller`]'s envelope: `GET /projects` (a keyset
/// page, newest first) and `GET /projects/changes` (body-free frames after a
/// cursor). The query is parsed before any store access, so a malformed one
/// never opens the database.
async fn read<R>(
    state: AppState,
    command: &'static str,
    raw: std::result::Result<Query<Vec<(String, String)>>, QueryRejection>,
    field: fn(&mut R, &str, String) -> Option<()>,
) -> Response
where
    R: Command + Default + Send + 'static,
    R::Response: serde::Serialize + Send + 'static,
{
    match query(raw.map(|Query(pairs)| pairs), field) {
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

/// `GET /api/v1/projects/{id}` — one charter.
async fn show_project(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    query_response(state, "project.show", move |ctx| {
        authority::run(ctx, &caller(), show::Request { id })
    })
    .await
}

#[cfg(test)]
#[path = "tests/projects.rs"]
mod tests;
