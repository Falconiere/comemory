//! `POST /api/v1/erase` (`maintenance::erase`) — permanently erase one memory
//! or one document. Mutating and confirm-gated, like every route that cannot
//! be undone: read-only first (`405 read_only`), then the write permit
//! (`503 busy`), then `"confirm": true` (`400 confirmation_required`). An
//! entity this engine never held answers `404 not_found`.

use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::domains::maintenance;
use crate::serve::AppState;
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::mutate_response::{mutate_blocking, with_ctx};
use crate::serve::routes::{RouteEntry, require_confirm};

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
    mutate_blocking(state, "erase", move |state| {
        let (req, confirmed) = split_confirm::<maintenance::erase::Request>(body)?;
        require_confirm(confirmed)?;
        with_ctx(state, None, |ctx| maintenance::erase::run(ctx, req))
    })
    .await
}
