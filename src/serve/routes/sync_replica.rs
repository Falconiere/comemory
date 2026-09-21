//! `GET /api/v1/sync/replica/{changes,manifest,events}` and
//! `POST /api/v1/sync/replica/{import,stage,activate}` — the `replica-v1`
//! surface.
//!
//! The legacy routes in [`super::sync`] keep their own wire shape; both write
//! the same journal. Reads are read-class, the three write routes take the
//! mutating gate, and nothing here starts a daemon: this is the hosted
//! engine's request path, not its lifecycle. The cores themselves live in
//! [`crate::domains::sync::replica`]; this file is extractors and gating only.

use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::domains::sync::replica::contract::ImportRequest;
use crate::domains::sync::replica::contract_views::{ActivateRequest, StageRequest};
use crate::domains::sync::replica::{accept, changes, events, manifest, staging};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond};
use crate::utilities::activity::Origin;
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// Positions returned when a caller names no limit.
const DEFAULT_LIMIT: usize = 100;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/sync/replica/changes",
            command: "sync.replica.changes",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/sync/replica/manifest",
            command: "sync.replica.manifest",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/sync/replica/events",
            command: "sync.replica.events",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/sync/replica/import",
            command: "sync.replica.import",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/sync/replica/stage",
            command: "sync.replica.stage",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/sync/replica/activate",
            command: "sync.replica.activate",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
///
/// One builder per route, grouped into a read half and a write half: the six
/// differ only in extractor and core, and keeping them apart keeps every
/// function small enough to read at a glance.
pub fn router(state: AppState) -> Router<AppState> {
    read_routes().merge(write_routes(state))
}

/// The three read-class routes.
fn read_routes() -> Router<AppState> {
    changes_route()
        .merge(manifest_route())
        .merge(events_route())
}

/// `GET /sync/replica/changes` — the ordered page above a cursor.
fn changes_route() -> Router<AppState> {
    Router::new().route(
        "/api/v1/sync/replica/changes",
        get(
            |State(state): State<AppState>, headers: HeaderMap, Query(q): Query<FeedQuery>| {
                let origin = state.http_origin(&headers);
                handle(
                    state,
                    "sync.replica.changes",
                    Gate::Read,
                    origin,
                    move |ctx| {
                        changes::run(ctx, q.since, q.limit, q.kind.as_deref(), q.epoch.as_deref())
                    },
                )
            },
        ),
    )
}

/// `GET /sync/replica/manifest` — holdings, capability and seeding progress.
fn manifest_route() -> Router<AppState> {
    Router::new().route(
        "/api/v1/sync/replica/manifest",
        get(|State(state): State<AppState>, headers: HeaderMap| {
            let origin = state.http_origin(&headers);
            handle(
                state,
                "sync.replica.manifest",
                Gate::Read,
                origin,
                manifest::run,
            )
        }),
    )
}

/// `GET /sync/replica/events` — notification-only frames.
fn events_route() -> Router<AppState> {
    Router::new().route(
        "/api/v1/sync/replica/events",
        get(
            |State(state): State<AppState>, headers: HeaderMap, Query(q): Query<FeedQuery>| {
                let origin = state.http_origin(&headers);
                handle(
                    state,
                    "sync.replica.events",
                    Gate::Read,
                    origin,
                    move |ctx| events::frames(ctx, q.since, q.limit, q.epoch.as_deref()),
                )
            },
        ),
    )
}

/// The three mutating routes: import, stage and activation.
fn write_routes(_state: AppState) -> Router<AppState> {
    import_route().merge(stage_route()).merge(activate_route())
}

/// `POST /sync/replica/import` — apply an envelope.
fn import_route() -> Router<AppState> {
    Router::new().route(
        "/api/v1/sync/replica/import",
        post(
            |State(state): State<AppState>, headers: HeaderMap, Json(req): Json<ImportRequest>| {
                let origin = state.http_origin(&headers);
                handle(
                    state,
                    "sync.replica.import",
                    Gate::Write,
                    origin,
                    move |ctx| accept::run(ctx, req),
                )
            },
        ),
    )
}

/// `POST /sync/replica/stage` — store one part of an oversized revision.
fn stage_route() -> Router<AppState> {
    Router::new().route(
        "/api/v1/sync/replica/stage",
        post(
            |State(state): State<AppState>, headers: HeaderMap, Json(req): Json<StageRequest>| {
                let origin = state.http_origin(&headers);
                handle(
                    state,
                    "sync.replica.stage",
                    Gate::Write,
                    origin,
                    move |ctx| staging::stage(ctx, req),
                )
            },
        ),
    )
}

/// `POST /sync/replica/activate` — assemble a staged upload and accept it.
fn activate_route() -> Router<AppState> {
    Router::new().route(
        "/api/v1/sync/replica/activate",
        post(
            |State(state): State<AppState>,
             headers: HeaderMap,
             Json(req): Json<ActivateRequest>| {
                let origin = state.http_origin(&headers);
                handle(
                    state,
                    "sync.replica.activate",
                    Gate::Write,
                    origin,
                    move |ctx| staging::activate(ctx, req),
                )
            },
        ),
    )
}

/// Query params shared by `changes` and `events`.
#[derive(Debug, Deserialize)]
struct FeedQuery {
    /// Positions above this one.
    #[serde(default)]
    since: i64,
    /// Page size.
    #[serde(default = "default_limit")]
    limit: usize,
    /// Restrict to one entity kind.
    #[serde(default)]
    kind: Option<String>,
    /// The epoch the cursor was taken under; a mismatch is a 409.
    #[serde(default)]
    epoch: Option<String>,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

/// Whether a route writes to the store.
#[derive(Clone, Copy)]
enum Gate {
    Read,
    Write,
}

/// One request: gate it, open the shared connection off the runtime, run `f`
/// over a borrowed [`Ctx`], and envelope the result under `command`.
async fn handle<T, F>(
    state: AppState,
    command: &'static str,
    gate: Gate,
    origin: Origin,
    f: F,
) -> Response
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
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn).with_origin(origin);
        f(&mut ctx)
    })
    .await;
    respond(command, result, started)
}

#[cfg(test)]
#[path = "tests/sync_replica.rs"]
mod tests;
