//! `POST|GET /api/v1/projects/{id}/evidence` (#346): attach typed evidence
//! and page a project's evidence, the `domains::projects::evidence_add` and
//! `evidence_page` cores under the caller's envelope. The path names the
//! project; an attach's body is the platform's, and a page's query its
//! filters. An attach answers `200`, as the platform's procedure does. The
//! route-table rows live with their siblings in
//! [`super::projects::table_entries`].

use std::time::Instant;

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::Value;

use crate::domains::projects::authority;
use crate::domains::projects::{evidence_add, evidence_page};
use crate::serve::AppState;
use crate::serve::routes::project_request::body;
use crate::serve::routes::projects::{caller, parsed, read};
use crate::serve::routes::{guard_mutating, respond};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// `project evidence add`'s route command, shared by the table and handler.
pub const ADD: &str = "project.evidence.add";
/// `project evidence list`'s route command, shared by the table and handler.
pub const LIST: &str = "project.evidence.list";

/// `POST /api/v1/projects/{id}/evidence` — record one piece of evidence,
/// when the caller holds `evidence.create`. The path's id wins over any
/// `projectId` in the body.
pub async fn add(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Response {
    let started = Instant::now();
    let origin = state.http_origin(&headers);
    let permit = match guard_mutating(ADD, &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let req: evidence_add::Request = body(&with_project_id(&raw, id))?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn).with_origin(origin);
        authority::run(&mut ctx, &caller(), req)
    })
    .await;
    respond(ADD, result, started)
}

/// `GET /api/v1/projects/{id}/evidence` — a filtered keyset page, parsed
/// before any store access.
pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    raw: std::result::Result<Query<Vec<(String, String)>>, QueryRejection>,
) -> Response {
    let req = parsed(raw, field).map(|page| evidence_page::Request {
        project_id: id,
        ..page
    });
    read(state, LIST, req).await
}

/// `raw` with `projectId` set to the path's `id` when it is a JSON object;
/// anything else passes through for [`body`] to refuse.
pub fn with_project_id(raw: &[u8], id: String) -> Vec<u8> {
    match serde_json::from_slice::<Value>(raw) {
        Ok(Value::Object(mut map)) => {
            map.insert("projectId".to_string(), Value::String(id));
            serde_json::to_vec(&map).unwrap_or_else(|_| raw.to_vec())
        }
        _ => raw.to_vec(),
    }
}

/// One query pair stored on `req`; `None` refuses an unknown key (the id
/// comes from the path) or a non-numeric `limit`. The core judges every
/// other value.
pub fn field(req: &mut evidence_page::Request, key: &str, value: String) -> Option<()> {
    match key {
        "limit" => req.limit = Some(value.parse().ok()?),
        "cursor" => req.cursor = Some(value),
        "kind" => req.kind = Some(value),
        "trust" => req.trust = Some(value),
        "workItemId" => req.work_item_id = Some(value),
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
#[path = "tests/project_evidence.rs"]
mod tests;
