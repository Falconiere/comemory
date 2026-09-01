//! `GET /api/v1/hooks` (read hook state, `api::hooks`) and `POST
//! /api/v1/hooks` (per-hook enable/disable, `api::hooks`) — the console's
//! readable, per-hook-controllable surface over `install-hooks`'s three git
//! hooks plus the config-backed search→edit auto-reinforcement row.
//!
//! `PUT /api/v1/hooks/{name}?repo=<path>` (console-api spec §6) is the
//! per-hook, idempotent form of the same write: `{enabled: true|false}`
//! against one hook named in the URL. It shares `api::hooks::run` with the
//! `POST` body form — the only difference is where the hook name and the
//! desired state come from.
//!
//! Not confirm-gated (spec AC-33b): installing or removing a hook file is
//! idempotent and reversible, unlike `POST /api/v1/hooks/install`
//! (`maint::admin`), which stays confirm-gated as the install-all
//! shorthand. `POST` still goes through [`guard_mutating`] — `405
//! read_only` on a `--read-only` server, `503 busy` on write-permit
//! contention.

use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/hooks",
            command: "hooks",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/hooks",
            command: "hooks",
            mutating: true,
        },
        RouteEntry {
            method: "PUT",
            path: "/hooks/{name}",
            command: "hooks.set",
            mutating: true,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/hooks", get(list_hooks).post(toggle_hooks))
        .route("/api/v1/hooks/{name}", put(set_hook))
}

/// The only query parameter `GET /api/v1/hooks` reads. Deliberately narrower
/// than `api::hooks::Request` (which also carries `enable`/`disable`) — a
/// `GET` must never be able to toggle a hook no matter what a client puts on
/// the query string, so this handler builds the request itself rather than
/// deserializing the full type straight off the query.
#[derive(Deserialize, Debug)]
struct ListQuery {
    #[serde(default)]
    repo: Option<String>,
}

/// `GET /api/v1/hooks` — report all four rows (`api::hooks`), read-only.
async fn list_hooks(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::hooks::run(
            &mut ctx,
            api::hooks::Request {
                repo: q.repo,
                enable: None,
                disable: None,
            },
        )
    })
    .await;
    respond("hooks", result, started)
}

/// `POST /api/v1/hooks` — enable/disable one hook (`api::hooks`), then
/// report all four rows. Read-only-gated via [`guard_mutating`]; not
/// confirm-gated (module doc, AC-33b).
async fn toggle_hooks(
    State(state): State<AppState>,
    Json(req): Json<api::hooks::Request>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("hooks", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::hooks::run(&mut ctx, req)
    })
    .await;
    respond("hooks", result, started)
}

/// `PUT /api/v1/hooks/{name}` body: the desired state of that one hook.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SetBody {
    /// `true` installs/enables the hook, `false` removes/disables it.
    enabled: bool,
}

/// `PUT /api/v1/hooks/{name}?repo=<path>` — set one hook's state
/// (`api::hooks`), then report all four rows. `name` is accepted in either
/// spelling the console might send: `post_commit` and `post-commit` both
/// resolve to the git hook `post-commit` (an unknown name after that
/// normalization is `api::hooks`' own `400 usage`). Read-only-gated via
/// [`guard_mutating`]; idempotent, so not confirm-gated — the same rule the
/// `POST` form follows.
async fn set_hook(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<ListQuery>,
    Json(body): Json<SetBody>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("hooks.set", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let hook = name.replace('_', "-");
        let (enable, disable) = if body.enabled {
            (Some(hook), None)
        } else {
            (None, Some(hook))
        };
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::hooks::run(
            &mut ctx,
            api::hooks::Request {
                repo: q.repo,
                enable,
                disable,
            },
        )
    })
    .await;
    respond("hooks.set", result, started)
}

#[cfg(test)]
#[path = "tests/hooks_put.rs"]
mod tests;
