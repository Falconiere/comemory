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
//! contention — and, like `hooks/install`, contains an explicit `repo`
//! to the session's allowed roots before a hook file is written.
//!
//! The `search-edit-reinforcement` row lives in `config.toml`, so both
//! writers reload `AppState.cfg` after toggling it and answer from the
//! reloaded config — the same read the next `GET /hooks` (and the index
//! jobs that consult `[reinforce] enabled`) will make.

use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::maint::admin::contain_repo;
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
        apply_and_report(&state, req)
    })
    .await;
    respond("hooks", result, started)
}

/// The shared middle of both writers: contain an explicit `repo` (a hook
/// file is written under it — the same [`contain_repo`] gate `POST
/// /hooks/install` runs; `400` when it does not exist, `403` outside every
/// allowed root), apply the toggle through `api::hooks::run`, reload the
/// server's config when the config-backed row was written, and only then
/// report all four rows — read back through the reloaded config, so the
/// answer is exactly what the next `GET /hooks` will say rather than an
/// echo of the write. An implicit `repo` (the server's own cwd) is not
/// contained: that is the process's own directory, the same cwd
/// `save --ref-*` anchors against.
fn apply_and_report(
    state: &AppState,
    mut req: api::hooks::Request,
) -> Result<api::hooks::Response> {
    if let Some(repo) = req.repo.as_deref() {
        let canonical = contain_repo(state, repo)?;
        req.repo = Some(canonical.to_string_lossy().into_owned());
    }
    let repo = req.repo.clone();
    let wrote_config =
        [req.enable.as_deref(), req.disable.as_deref()].contains(&Some(api::hooks::REINFORCE_HOOK));
    {
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::hooks::run(&mut ctx, req)?;
    }
    if wrote_config {
        // Only after the file is written: a failed write must not swap in
        // a config the file does not back (the `config.rs` `PUT` ordering).
        state.reload_cfg(state.paths())?;
    }
    let cfg = state.cfg();
    let mut ctx = Ctx::lazy(state.paths(), &cfg);
    api::hooks::run(
        &mut ctx,
        api::hooks::Request {
            repo,
            enable: None,
            disable: None,
        },
    )
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
/// [`guard_mutating`], `repo` contained, config reloaded after a
/// `search-edit-reinforcement` write ([`apply_and_report`]); idempotent, so
/// not confirm-gated — the same rules the `POST` form follows.
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
        // Unconditional, for every name: `_` is not part of any hook's
        // spelling, so a console may send either `post_commit` or
        // `post-commit` (and either `search_edit_reinforcement` or its
        // hyphenated form). Anything that does not match a known hook
        // after this is rejected by `api::hooks` as a usage error.
        let hook = name.replace('_', "-");
        let (enable, disable) = if body.enabled {
            (Some(hook), None)
        } else {
            (None, Some(hook))
        };
        apply_and_report(
            &state,
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
