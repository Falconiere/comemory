//! `GET|POST /api/v1/find` (`api::find`) — one ranked list across memory,
//! code, and documents.
//!
//! Its own resource file rather than an entry under [`super::memories`]:
//! `find` is cross-domain, and filing it beneath the memories resource
//! would misdescribe it in the route table that `GET /commands` reports.
//!
//! `GET` carries the query in the query string; only `POST` can supply a
//! `vector`, since an embedding does not fit in a URL.

use std::time::Instant;

use axum::Router;
use axum::extract::{Json, Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking, track_for};
use crate::serve::scope::RepoScope;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/find",
            command: "find",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/find",
            command: "find",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/find", get(find_get).post(find_post))
}

/// `GET /api/v1/find` — query-string form, no vector. An `X-Comemory-Repo`
/// header is the default `repo` filter when the query omits one.
async fn find_get(
    State(state): State<AppState>,
    scope: RepoScope,
    Query(mut req): Query<api::find::Request>,
) -> Response {
    scope.fill_if_absent(&mut req.repo);
    execute(state, req).await
}

/// `POST /api/v1/find` — body form, vector-capable.
async fn find_post(
    State(state): State<AppState>,
    scope: RepoScope,
    Json(mut req): Json<api::find::Request>,
) -> Response {
    scope.fill_if_absent(&mut req.repo);
    execute(state, req).await
}

/// Shared handler body. Access tracking is suppressed on a read-only
/// server exactly as it is for `search` / `search-code` / `context`.
async fn execute(state: AppState, req: api::find::Request) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let track = track_for(&state)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        let out = api::find::run(&mut ctx, req, track)?;
        Ok(serde_json::json!({
            "hits": out.hits,
            "query_id": out.query_id,
            "limit": out.meta.limit,
            "offset": out.meta.offset,
            "has_more": out.meta.has_more,
            "total": out.meta.total,
        }))
    })
    .await;
    respond("find", result, started)
}
