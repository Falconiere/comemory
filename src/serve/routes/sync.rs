//! `GET /api/v1/sync/{changes,manifest}` and `POST /api/v1/sync/import`
//! (memory-sync design spec), plus `GET /api/v1/sync/code/manifest` and
//! `POST /api/v1/sync/code/import` (code-graph sync design).

use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::sync::{CodeImportRequest, ImportRequest};
use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking};

const DEFAULT_CHANGES_LIMIT: usize = 100;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/sync/changes",
            command: "sync.changes",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/sync/manifest",
            command: "sync.manifest",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/sync/import",
            command: "sync.import",
            mutating: true,
        },
        RouteEntry {
            method: "GET",
            path: "/sync/code/manifest",
            command: "sync.code.manifest",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/sync/code/import",
            command: "sync.code.import",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`. Each handler is a
/// closure over [`handle`] — the five differ only in the extractor they
/// take and the `api::sync` core they call, and a named wrapper per route
/// would be five copies of the same three lines.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        // `GET /sync/changes?since=&limit=` — pull log entries above a cursor.
        .route(
            "/api/v1/sync/changes",
            get(
                |State(state): State<AppState>, Query(q): Query<ChangesQuery>| {
                    handle(state, "sync.changes", Gate::Read, move |ctx| {
                        api::sync::changes::run(ctx, q.since, q.limit)
                    })
                },
            ),
        )
        // `GET /sync/manifest` — 256-bucket content-hash digest.
        .route(
            "/api/v1/sync/manifest",
            get(|State(state): State<AppState>| {
                handle(state, "sync.manifest", Gate::Read, api::sync::manifest::run)
            }),
        )
        // `POST /sync/import` — apply a batch of wire entries, stamping the
        // author `X-Comemory-Author` names.
        .route(
            "/api/v1/sync/import",
            post(
                |State(state): State<AppState>,
                 headers: HeaderMap,
                 Json(req): Json<ImportRequest>| {
                    let author = author_from_headers(&headers);
                    handle(state, "sync.import", Gate::Write, move |ctx| {
                        api::sync::import::run(ctx, req, author.as_deref())
                    })
                },
            ),
        )
        // `GET /sync/code/manifest?repo=` — the per-file digest list.
        .route(
            "/api/v1/sync/code/manifest",
            get(
                |State(state): State<AppState>, Query(q): Query<CodeManifestQuery>| {
                    handle(state, "sync.code.manifest", Gate::Read, move |ctx| {
                        api::sync::code_manifest::run(ctx, &q.repo)
                    })
                },
            ),
        )
        // `POST /sync/code/import` — apply one code-projection batch.
        .route(
            "/api/v1/sync/code/import",
            post(
                |State(state): State<AppState>, Json(req): Json<CodeImportRequest>| {
                    handle(state, "sync.code.import", Gate::Write, move |ctx| {
                        api::sync::code_import::run(ctx, req)
                    })
                },
            ),
        )
}

/// Query params for `GET /api/v1/sync/changes`.
#[derive(Debug, Deserialize)]
struct ChangesQuery {
    #[serde(default)]
    since: i64,
    #[serde(default = "default_changes_limit")]
    limit: usize,
}

fn default_changes_limit() -> usize {
    DEFAULT_CHANGES_LIMIT
}

/// Query params for `GET /api/v1/sync/code/manifest`.
#[derive(Debug, Deserialize)]
struct CodeManifestQuery {
    repo: String,
}

/// Whether a route writes to the store — a write takes the read-only /
/// write-permit gate first and holds the permit for the whole batch.
#[derive(Clone, Copy)]
enum Gate {
    Read,
    Write,
}

/// One request: gate it, open the shared connection off the runtime, run
/// `f` over a borrowed [`Ctx`], and envelope the result under `command`.
async fn handle<T, F>(state: AppState, command: &'static str, gate: Gate, f: F) -> Response
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce(&mut Ctx<'_>) -> Result<T> + Send + 'static,
{
    let started = Instant::now();
    let permit = match gate {
        Gate::Read => None,
        Gate::Write => match guard_mutating(command, &state) {
            Ok(permit) => Some(permit),
            Err(resp) => return *resp,
        },
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        f(&mut ctx)
    })
    .await;
    respond(command, result, started)
}

/// Read optional author override from `X-Comemory-Author`.
fn author_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-comemory-author")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "tests/sync.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/sync_code.rs"]
mod tests_code;
