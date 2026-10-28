//! `GET|PUT /api/v1/config/retrieval` (console-api spec §7).
//!
//! The `GET` is pure in-memory introspection of `AppState`'s current config
//! (like `GET /health` and `GET /commands`) — no database, no blocking pool.
//! The `PUT` validates in memory, writes `config.toml`, then reloads
//! `AppState.cfg` and answers with the RELOADED knobs, so the response is
//! read back through the same layering (defaults → file → env) a restart
//! would use rather than echoing the request.

use std::time::Instant;

use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};

use crate::domains::retrieval;
use crate::serve::AppState;
use crate::serve::routes::mutate_response::{mutate_blocking, with_lazy_ctx};
use crate::serve::routes::{RouteEntry, respond};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/config/retrieval",
            command: "config.retrieval",
            mutating: false,
        },
        RouteEntry {
            method: "PUT",
            path: "/config/retrieval",
            command: "config.retrieval.update",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/config/retrieval", get(read).put(update))
}

/// `GET /api/v1/config/retrieval` — the live ranking knobs plus the
/// per-knob range table.
async fn read(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let knobs = retrieval::config_retrieval::get(&state.cfg());
    respond("config.retrieval", Ok(knobs), started)
}

/// `PUT /api/v1/config/retrieval` — partial knob update. Mutating (it
/// rewrites `config.toml`) but not confirm-gated: every knob is bounded,
/// validated before the write, and reversible by another `PUT`.
async fn update(
    State(state): State<AppState>,
    Json(req): Json<retrieval::config_retrieval::UpdateRequest>,
) -> Response {
    mutate_blocking(state, "config.retrieval.update", move |state| {
        with_lazy_ctx(state, |ctx| retrieval::config_retrieval::update(ctx, req))?;
        state.reload_cfg(state.paths())?;
        Ok(retrieval::config_retrieval::get(&state.cfg()))
    })
    .await
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
