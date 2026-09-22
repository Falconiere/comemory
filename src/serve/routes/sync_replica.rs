//! `GET /api/v1/sync/replica/{changes,manifest,events}` and
//! `POST /api/v1/sync/replica/{import,stage,activate}` — the `replica-v1`
//! surface.
//!
//! Reads are read-class, writes take the mutating gate, and nothing here
//! starts a daemon. Six routes, two shapes — a cursor read and a JSON write —
//! each mounted by one generic builder, so adding a route is a line rather
//! than a copy. The cores live in [`crate::domains::sync::replica`].

use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde::de::DeserializeOwned;

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
pub fn router(_state: AppState) -> Router<AppState> {
    feed_route(
        "/api/v1/sync/replica/changes",
        "sync.replica.changes",
        |ctx, q| changes::run(ctx, q.since, q.limit, q.kind.as_deref(), q.epoch.as_deref()),
    )
    .merge(manifest_route())
    .merge(feed_route(
        "/api/v1/sync/replica/events",
        "sync.replica.events",
        |ctx, q| events::frames(ctx, q.since, q.limit, q.epoch.as_deref()),
    ))
    .merge(json_route::<ImportRequest, _>(
        "/api/v1/sync/replica/import",
        "sync.replica.import",
        accept::run,
    ))
    .merge(json_route::<StageRequest, _>(
        "/api/v1/sync/replica/stage",
        "sync.replica.stage",
        staging::stage,
    ))
    .merge(json_route::<ActivateRequest, _>(
        "/api/v1/sync/replica/activate",
        "sync.replica.activate",
        staging::activate,
    ))
}

/// A read-class route driven by a cursor query.
fn feed_route<T>(
    path: &'static str,
    command: &'static str,
    core: fn(&mut Ctx<'_>, FeedQuery) -> Result<T>,
) -> Router<AppState>
where
    T: serde::Serialize + Send + 'static,
{
    Router::new().route(
        path,
        get(
            move |State(state): State<AppState>, headers: HeaderMap, Query(q): Query<FeedQuery>| {
                let origin = state.http_origin(&headers);
                handle(state, command, Gate::Read, origin, move |ctx| core(ctx, q))
            },
        ),
    )
}

/// A mutating route whose body is one JSON envelope.
fn json_route<B, T>(
    path: &'static str,
    command: &'static str,
    core: fn(&mut Ctx<'_>, B) -> Result<T>,
) -> Router<AppState>
where
    B: DeserializeOwned + Send + 'static,
    T: serde::Serialize + Send + 'static,
{
    Router::new().route(
        path,
        post(
            move |State(state): State<AppState>, headers: HeaderMap, Json(req): Json<B>| {
                let origin = state.http_origin(&headers);
                handle(state, command, Gate::Write, origin, move |ctx| {
                    core(ctx, req)
                })
            },
        ),
    )
}

/// `GET /sync/replica/manifest` — the one route with no query of its own.
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
