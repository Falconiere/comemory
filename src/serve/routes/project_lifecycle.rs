//! `POST /api/v1/projects/{id}/{archive,restore,pause,resume}` (#328): the
//! `domains::projects::lifecycle` core under the caller's envelope. In local
//! mode that is the local agent, which holds no human verb, so every route
//! answers `403 project_agent_scope` after the body parses and writes
//! nothing. The route-table rows live with their siblings in
//! [`super::projects::table_entries`].

use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{MethodRouter, post};

use crate::domains::projects::authority;
use crate::domains::projects::lifecycle::{Body, Kind, Request};
use crate::serve::AppState;
use crate::serve::routes::project_request::body;
use crate::serve::routes::projects::caller;
use crate::serve::routes::{guard_mutating, respond};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// `kind`'s path under `/api/v1`, `{id}` left for axum.
#[must_use]
pub fn path(kind: Kind) -> String {
    format!("/api/v1/projects/{{id}}/{}", kind.as_str())
}

/// `kind`'s `POST` handler.
pub fn route(kind: Kind) -> MethodRouter<AppState> {
    post(
        move |State(state): State<AppState>,
              headers: HeaderMap,
              Path(id): Path<String>,
              raw: Bytes| { transition(state, headers, kind, id, raw) },
    )
}

/// Run `kind` on project `id` with the parsed body, behind the read-only
/// gate: `200 {project}` when [`caller`] may, else its refusal.
async fn transition(
    state: AppState,
    headers: HeaderMap,
    kind: Kind,
    id: String,
    raw: Bytes,
) -> Response {
    let started = Instant::now();
    let command = kind.command();
    let origin = state.http_origin(&headers);
    let permit = match guard_mutating(command, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let body: Body = body(&raw)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn).with_origin(origin);
        authority::run(&mut ctx, &caller(), Request { kind, id, body })
    })
    .await;
    respond(command, result, started)
}

#[cfg(test)]
#[path = "tests/project_lifecycle.rs"]
mod tests;
