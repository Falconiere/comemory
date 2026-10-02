//! `POST|GET /api/v1/projects/{id}/proposals` and
//! `GET /api/v1/projects/{id}/proposals/{proposalId}` — the
//! `domains::projects` propose and proposal-read cores on the platform's
//! REST paths, under the same local-mode envelope as every project route
//! ([`super::projects::caller`]). An admitted submission answers `201`.
//!
//! The submit route carries its own 32 MiB body limit: a 2,001-operation
//! proposal of large work-item creates must reach the engine's own `400
//! invalid_request` (the 2,000-operation schema bound) rather than the
//! server's 5 MiB `413`.

use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Map, Value};

use crate::domains::projects::authority;
use crate::domains::projects::proposals::{ListRequest, ShowRequest};
use crate::domains::projects::propose;
use crate::serve::AppState;
use crate::serve::routes::project_request::query;
use crate::serve::routes::projects::{caller, created};
use crate::serve::routes::{RouteEntry, guard_mutating, query_response, respond};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;
use crate::utilities::project_body::{body, from_value};

/// The submit route's body limit.
pub const SUBMIT_BODY_LIMIT: usize = 32 * 1024 * 1024;

const SUBMIT: &str = "project.proposal.submit";
const LIST: &str = "project.proposal.list";
const SHOW: &str = "project.proposal.show";

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "POST",
            path: "/projects/{id}/proposals",
            command: SUBMIT,
            mutating: true,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/{id}/proposals",
            command: LIST,
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/projects/{id}/proposals/{proposalId}",
            command: SHOW,
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/projects/{id}/proposals",
            // The raised limit is the submission's alone; the read keeps
            // the server-wide default.
            get(list).merge(post(submit).layer(DefaultBodyLimit::max(SUBMIT_BODY_LIMIT))),
        )
        .route("/api/v1/projects/{id}/proposals/{proposalId}", get(show))
}

/// `POST …/proposals` — submit a proposal to the project the path names.
async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Response {
    let started = Instant::now();
    let origin = state.http_origin(&headers);
    let permit = match guard_mutating(SUBMIT, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let mut fields: Map<String, Value> = body(&raw)?;
        fields.insert("projectId".to_string(), Value::String(id));
        let req: propose::Request = from_value(Value::Object(fields))?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn).with_origin(origin);
        authority::run(&mut ctx, &caller(), req)
    })
    .await;
    created(respond(SUBMIT, result, started))
}

/// `GET …/proposals` — one keyset page; the query is parsed before any
/// store access.
async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    raw: std::result::Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    match query(raw.map(|Query(pairs)| pairs), list_field) {
        Ok(mut req) => {
            req.project_id = id;
            query_response(state, LIST, move |ctx| authority::run(ctx, &caller(), req)).await
        }
        Err(e) => respond::<()>(LIST, Err(e), Instant::now()),
    }
}

/// `GET …/proposals/{proposalId}` — one proposal of the project.
async fn show(
    State(state): State<AppState>,
    Path((project_id, proposal_id)): Path<(String, String)>,
) -> Response {
    let req = ShowRequest {
        project_id,
        proposal_id,
    };
    query_response(state, SHOW, move |ctx| authority::run(ctx, &caller(), req)).await
}

/// `GET …/proposals`'s pair `key=value` stored on `req`; `None` refuses an
/// unknown key or a non-numeric `limit`.
pub fn list_field(req: &mut ListRequest, key: &str, value: String) -> Option<()> {
    match key {
        "limit" => req.limit = Some(value.parse().ok()?),
        "cursor" => req.cursor = Some(value),
        "state" => req.state = Some(value),
        _ => return None,
    }
    Some(())
}
