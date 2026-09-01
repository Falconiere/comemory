//! `GET /api/v1/repos` (`api::repos`) — the indexed code-repository
//! inventory. Own resource file rather than folded into [`super::stats`]:
//! `stats` reports the corpus, this reports the per-repo inventory the
//! console's Repositories screen and Code graph legend need. Not folded
//! into [`super::sources`] either — a `repo_marker` row (code index) and a
//! registered document source are unrelated concepts that happen to share
//! no schema.

use std::time::Instant;

use axum::Router;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/repos",
        command: "repos",
        mutating: false,
    }]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/repos", get(repos))
}

/// `GET /api/v1/repos` — the code-repository inventory (`api::repos`). Uses
/// `Ctx::lazy` rather than the shared connection so the must-not-create-the-
/// db invariant holds here exactly as it does on the CLI: a server pointed
/// at an empty data dir answers with an empty inventory instead of
/// materializing a database.
async fn repos(State(state): State<AppState>, Query(req): Query<api::repos::Request>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::repos::run(&mut ctx, req)
    })
    .await;
    respond("repos", result, started)
}
