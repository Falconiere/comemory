//! `POST|GET /api/v1/projects` and `GET /api/v1/projects/{id}` — the
//! `domains::projects` create, list and show cores, on the platform's REST
//! paths so a hosted cutover forwards without remapping. `POST` answers
//! `201`, as the platform does.
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

use crate::domains::projects::principal::Principal;
use crate::domains::projects::{create, list, show};
use crate::serve::AppState;
use crate::serve::routes::project_request::{body, list_query};
use crate::serve::routes::{RouteEntry, guard_mutating, query_response, respond, respond_with};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// `project.create`'s route command, shared by the table and its handler.
const CREATE: &str = "project.create";

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
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/projects", get(list_projects).post(create_project))
        .route("/api/v1/projects/{id}", get(show_project))
}

/// `POST /api/v1/projects` — charter a draft project as the local operator.
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
        create::run(&mut ctx, &Principal::local_operator(), req)
    })
    .await;
    respond_with(StatusCode::CREATED, CREATE, result, started)
}

/// `GET /api/v1/projects` — one keyset page, newest first.
async fn list_projects(
    State(state): State<AppState>,
    query: Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let req = match list_query(query.map(|Query(pairs)| pairs)) {
        Ok(req) => req,
        Err(e) => return respond::<()>("project", Err(e), Instant::now()),
    };
    query_response(state, "project", move |ctx| list::run(ctx, req)).await
}

/// `GET /api/v1/projects/{id}` — one charter.
async fn show_project(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    query_response(state, "project.show", move |ctx| {
        show::run(ctx, show::Request { id })
    })
    .await
}
