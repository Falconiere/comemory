//! `GET /api/v1/learning/summary`, `GET|POST /api/v1/learning/evals`,
//! `GET /api/v1/learning/golden-set`, `GET /api/v1/learning/proposals`,
//! `POST /api/v1/learning/proposals/{id}/apply|discard`,
//! `GET /api/v1/learning/expansions` (console-api spec §7).
//!
//! Every read here is synchronous (`run_blocking` + `Ctx::borrowed`) — they
//! are all `SELECT`s. The one long-running route, `POST /learning/evals`, is
//! not implemented here at all: it is a second mount of
//! `routes::learning::eval`, the existing `POST /eval` job handler, so the
//! console's "run an eval" button and the CLI-parity route cannot drift
//! (alias, never duplicate).

use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::serve::AppState;
use crate::serve::routes::learning::{contain_golden, eval};
use crate::serve::routes::maint::prune::split_confirm;
use crate::serve::routes::{RouteEntry, guard_mutating, require_confirm, respond, run_blocking};

/// Default page size for `GET /learning/expansions`.
fn default_expansions_limit() -> usize {
    20
}

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/learning/summary",
            command: "learning.summary",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/learning/evals",
            command: "learning.evals",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/learning/evals",
            command: "eval",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/learning/golden-set",
            command: "learning.golden-set",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/learning/proposals",
            command: "learning.proposals",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/learning/proposals/{id}/apply",
            command: "learning.proposals.apply",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/learning/proposals/{id}/discard",
            command: "learning.proposals.discard",
            mutating: true,
        },
        RouteEntry {
            method: "GET",
            path: "/learning/expansions",
            command: "learning.expansions",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/learning/summary", get(summary))
        .route("/api/v1/learning/evals", get(evals).post(eval))
        .route("/api/v1/learning/golden-set", get(golden_set))
        .route("/api/v1/learning/proposals", get(proposals))
        .route(
            "/api/v1/learning/proposals/{id}/apply",
            post(proposal_apply),
        )
        .route(
            "/api/v1/learning/proposals/{id}/discard",
            post(proposal_discard),
        )
        .route("/api/v1/learning/expansions", get(expansions))
}

/// `GET /api/v1/learning/summary` — feedback counters, the newest run, the
/// mined-expansion count, and the best recall gain on record.
async fn summary(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::learning::summary(&mut ctx)
    })
    .await;
    respond("learning.summary", result, started)
}

/// `?limit=` on `GET /learning/evals`, defaulted to the same value
/// `GET /eval/history` uses so the two run lists page alike.
#[derive(Deserialize)]
struct EvalsQuery {
    #[serde(default = "crate::api::eval::default_history_limit")]
    limit: u32,
}

/// `GET /api/v1/learning/evals` — the run history with the console's
/// derived `delta` / `is_baseline` / `is_best` per row.
async fn evals(State(state): State<AppState>, Query(q): Query<EvalsQuery>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::learning::evals(&mut ctx, q.limit)
    })
    .await;
    respond("learning.evals", result, started)
}

/// `?golden=` on `GET /learning/golden-set` — an optional YAML file merged
/// over the feedback harvest.
#[derive(Deserialize)]
struct GoldenQuery {
    #[serde(default)]
    golden: Option<String>,
}

/// `GET /api/v1/learning/golden-set` — the effective golden set. `?golden=`
/// is contained to an allowed root first, through the same
/// [`contain_golden`] check `POST /eval` uses (AC-7).
async fn golden_set(State(state): State<AppState>, Query(q): Query<GoldenQuery>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let golden = contain_golden(&state, q.golden.as_deref())?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::learning::golden_set(&mut ctx, golden.as_deref())
    })
    .await;
    respond("learning.golden-set", result, started)
}

/// `GET /api/v1/learning/proposals` — the unapplied, undiscarded
/// `tune`/`bandit` runs whose knobs still differ from the live config.
async fn proposals(State(state): State<AppState>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::learning_proposals::list(&mut ctx)
    })
    .await;
    respond("learning.proposals", result, started)
}

/// The body of a confirm-gated route with no request fields of its own:
/// `{"confirm": true}` and nothing else. `deny_unknown_fields` so a typo'd
/// field is a `400` rather than a silently ignored key — [`split_confirm`]
/// removes `confirm` before this type sees the object.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmOnly {}

/// `POST /api/v1/learning/proposals/{id}/apply` — write the proposal's
/// knobs into `config.toml`, stamp the run `applied`, and reload the
/// server's in-memory config so ranking picks the new knobs up without a
/// restart. Confirm-gated (it rewrites a file the operator owns), with
/// read-only checked first (AC-19).
async fn proposal_apply(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("learning.proposals.apply", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let (ConfirmOnly {}, confirmed) = split_confirm::<ConfirmOnly>(body)?;
        require_confirm(confirmed)?;
        let applied = {
            let cfg = state.cfg();
            let mut conn = state.conn()?;
            let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
            api::learning_proposals::apply(&mut ctx, &id)?
        };
        state.reload_cfg(state.paths())?;
        Ok(applied)
    })
    .await;
    respond("learning.proposals.apply", result, started)
}

/// `POST /api/v1/learning/proposals/{id}/discard` — stop offering a
/// proposal. Mutating (it writes `eval_runs.discarded`) but NOT
/// confirm-gated: nothing outside the console's own list changes, and the
/// run row itself is untouched otherwise.
async fn proposal_discard(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("learning.proposals.discard", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::learning_proposals::discard(&mut ctx, &id)
    })
    .await;
    respond("learning.proposals.discard", result, started)
}

/// `?limit=&offset=` on `GET /learning/expansions`.
#[derive(Deserialize)]
struct ExpansionsQuery {
    #[serde(default = "default_expansions_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

/// `GET /api/v1/learning/expansions` — one page of mined `query_expansions`
/// rows, strongest support first.
async fn expansions(State(state): State<AppState>, Query(q): Query<ExpansionsQuery>) -> Response {
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::learning::expansions(&mut ctx, q.limit, q.offset)
    })
    .await;
    respond("learning.expansions", result, started)
}

#[cfg(test)]
#[path = "tests/learning_console.rs"]
mod tests;
