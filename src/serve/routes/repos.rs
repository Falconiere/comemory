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
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::index_runs::INDEX_JOB_COMMAND;
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
///
/// One HTTP-only overlay on top of the shared core (spec §10): a repo with
/// a queued or running `index-code` job in this server's registry reports
/// `status: "indexing"` and carries the job's id in `indexing_job`, so the
/// console can link the row straight to `GET /jobs/{id}`. The registry is a
/// server-process concept the CLI has no access to, which is why it is an
/// overlay here rather than a field `api::repos` could fill in.
///
/// Deliberately the one repo-bearing read OUTSIDE the default repo scope
/// (`X-Comemory-Repo` / `serve --repo`, `crate::serve::scope::RepoScope`):
/// the inventory is how a client discovers which scopes exist, and the
/// scope cannot be cleared from the client side (an empty header reads as
/// absent and falls back to `--repo`), so applying it here would leave a
/// `--repo alpha` console unable to ever list — or switch to — another
/// repo. Only an explicit `?repo=` narrows it.
async fn repos(State(state): State<AppState>, Query(req): Query<api::repos::Request>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut response = {
            let mut ctx = Ctx::lazy(state.paths(), &cfg);
            api::repos::run(&mut ctx, req)?
        };
        overlay_indexing(&state, &mut response)?;
        Ok(response)
    })
    .await;
    respond("repos", result, started)
}

#[cfg(test)]
#[path = "tests/repos.rs"]
mod tests;

/// Mark every row whose repo has a live `index-code` job `"indexing"` and
/// record that job's id. An archived repo cannot have one (indexing is
/// refused for it), so the two statuses cannot collide.
fn overlay_indexing(state: &AppState, response: &mut api::repos::Response) -> Result<()> {
    for row in &mut response.repos {
        if let Some(job_id) = state.jobs().active_for(INDEX_JOB_COMMAND, &row.repo)? {
            row.status = "indexing".to_string();
            row.indexing_job = Some(job_id);
        }
    }
    Ok(())
}
