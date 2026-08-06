//! `GET /api/v1/sources` (`api::sources`). The `reconcile` side effect is
//! computed server-side from the read-only flag — never taken from the
//! query string, so a client cannot force a mirror write on a read-only
//! server (§Security "Read-only side-effect degradation").

use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/sources",
        command: "sources",
        mutating: false,
    }]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/sources", get(sources))
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
