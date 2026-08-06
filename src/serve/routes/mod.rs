//! The versioned `/api/v1` REST surface. One file per resource
//! (`memories`, `code`, `sources`, `graph`, `learning`, `maint`, `jobs`,
//! `meta`, …, added by later steps); this module owns the route table and
//! the top-level `v1_router` assembly `serve::router::build_router` merges
//! in.
//!
//! The route [`table`] is the single source of truth for which routes exist
//! and whether each one mutates — consumed by the read-only gate (a later
//! step, keyed on `mutating`, **not** the HTTP method: a `POST
//! /memories/search` is a read) and by `GET /commands` (also later).

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use serde_json::json;

use crate::serve::AppState;
use crate::serve::envelope::Envelope;

/// One `/api/v1` route's static metadata.
#[derive(Debug, Clone, Copy)]
pub struct RouteEntry {
    /// HTTP method as it appears in the route map (`"GET"`, `"POST"`, …).
    pub method: &'static str,
    /// Path under `/api/v1`, e.g. `"/health"`.
    pub path: &'static str,
    /// The CLI subcommand this route maps to, or a synthetic name for a
    /// route with no CLI counterpart (e.g. `"health"`).
    pub command: &'static str,
    /// Whether the read-only gate (§Security) rejects this route with
    /// `405 read_only` on a `--read-only` server.
    pub mutating: bool,
}

/// The static `/api/v1` route table, growing one entry per route as each
/// later step mounts it.
pub fn table() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/health",
        command: "health",
        mutating: false,
    }]
}

/// Mount the `/api/v1` surface. `state` is threaded through for later steps
/// whose route construction depends on it (e.g. conditionally mounting
/// mutating routes); today only `GET /api/v1/health` is mounted, so it is
/// unused for now.
pub fn v1_router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/health", get(health))
}

/// `GET /api/v1/health` — the same capability-probe payload as legacy `GET
/// /api/health`, wrapped in the envelope's `data` field.
async fn health(State(state): State<AppState>) -> Response {
    Envelope::ok(
        "health",
        json!({
            "read_only": state.read_only(),
            "version": env!("CARGO_PKG_VERSION"),
        }),
        0,
    )
}
