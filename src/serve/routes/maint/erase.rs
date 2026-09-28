//! `POST /api/v1/erase` (`maintenance::erase`) — permanently erase one memory
//! or one document. Mutating and confirm-gated, like every route that cannot
//! be undone: read-only first (`405 read_only`), then the write permit
//! (`503 busy`), then `"confirm": true` (`400 confirmation_required`). An
//! entity this engine never held answers `404 not_found`.

use std::time::Instant;

use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::domains::maintenance;
use crate::serve::AppState;
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::{RouteEntry, guard_mutating, require_confirm, respond};
use crate::utilities::blocking::run_blocking;
use crate::utilities::context::Ctx;

/// This resource's route-table entries, appended onto [`super::super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "POST",
        path: "/erase",
        command: "erase",
        mutating: true,
    }]
}

/// This resource's routes, merged into the `maint` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/erase", post(erase))
}

/// `POST /api/v1/erase` — `{"memory": ID}` or `{"document": SHARED_ID}`,
/// plus `"confirm": true`. The body is read as a raw [`Value`] through
/// [`split_confirm`], so the HTTP-only `confirm` never joins
/// `maintenance::erase::Request` (AC-12 parity). Runs on the server's shared
/// connection, so its next request reads the erased state.
async fn erase(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("erase", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let (req, confirmed) = split_confirm::<maintenance::erase::Request>(body)?;
        require_confirm(confirmed)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        maintenance::erase::run(&mut ctx, req)
    })
    .await;
    respond("erase", result, started)
}
