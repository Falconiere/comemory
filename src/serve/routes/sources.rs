//! `GET /api/v1/sources` (`api::sources`). The `reconcile` side effect is
//! computed server-side from the read-only flag — never taken from the
//! query string, so a client cannot force a mirror write on a read-only
//! server (§Security "Read-only side-effect degradation").
//!
//! `DELETE /api/v1/sources?target=<id|path>&confirm=true` (`api::unindex`)
//! also lives here — the query form expresses both the id and the
//! registered-path target, mirroring `comemory unindex <SOURCE_ID|PATH>`.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, require_confirm, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/sources",
            command: "sources",
            mutating: false,
        },
        RouteEntry {
            method: "DELETE",
            path: "/sources",
            command: "unindex",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/sources", get(sources).delete(unindex))
}

/// `GET /api/v1/sources` — list registered sources (`api::sources`). The
/// mirror reconcile runs only when the server is not `--read-only`.
async fn sources(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let req = api::sources::Request {
            reconcile: !state.read_only(),
        };
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::sources::run(&mut ctx, req)
    })
    .await;
    respond("sources", result, started)
}

/// `?target=&confirm=true` on `DELETE /sources` — transport-level; not part
/// of `api::unindex::Request` since the CLI has no `--confirm` concept.
#[derive(Deserialize)]
struct UnindexQuery {
    target: String,
    #[serde(default)]
    confirm: bool,
}

/// `DELETE /api/v1/sources?target=&confirm=true` — unregister a source
/// (`api::unindex`), confirm gated. `guard_mutating` (read-only/busy) runs
/// before [`require_confirm`] so a read-only server rejects with `405` even
/// without `?confirm=true` (AC-19).
async fn unindex(State(state): State<AppState>, Query(query): Query<UnindexQuery>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("unindex", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        require_confirm(query.confirm)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::unindex::run(
            &mut ctx,
            api::unindex::Request {
                target: query.target,
            },
        )
    })
    .await;
    respond("unindex", result, started)
}
