//! `GET /api/v1/prune` — the dry-run scan report (`api::prune`), always
//! forcing `apply: false`. `POST /api/v1/prune` — the confirm-gated apply
//! path — and `POST /api/v1/gc` (`api::gc`, also confirm-gated) live here
//! too: both are single mutating maintenance actions with no natural home
//! besides this resource.

use std::time::Instant;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::api::{self, Ctx};
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::routes::{RouteEntry, guard_mutating, require_confirm, respond, run_blocking};

/// This resource's route-table entries, appended onto [`super::super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/prune",
            command: "prune",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/prune/candidates",
            command: "prune",
            mutating: false,
        },
        RouteEntry {
            method: "POST",
            path: "/prune",
            command: "prune",
            mutating: true,
        },
        RouteEntry {
            method: "POST",
            path: "/gc",
            command: "gc",
            mutating: true,
        },
    ]
}

/// This resource's routes, merged into the `maint` resource router.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/prune", get(prune).post(prune_apply))
        // Console-api spec §9: `GET /prune/candidates` is an ALIAS onto the
        // same handler, not a second query path — the two answers cannot
        // drift because there is only one of them.
        .route("/api/v1/prune/candidates", get(prune))
        .route("/api/v1/gc", post(gc))
}

/// `GET /api/v1/prune` — the dry-run report (`api::prune`). `apply` is
/// forced `false` regardless of the query string: a `GET` route carries
/// no confirm gate, so it must never trigger the soft-delete/cleanup path
/// — that is [`prune_apply`] below.
async fn prune(
    State(state): State<AppState>,
    Query(mut req): Query<api::prune::Request>,
) -> Response {
    req.apply = false;
    let started = Instant::now();
    let result = run_blocking(move || {
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::prune::run(&mut ctx, req)
    })
    .await;
    respond("prune", result, started)
}

/// `POST /api/v1/prune` — the confirm-gated apply path (`api::prune::run`
/// with `req.apply` taken as-is from the body). The body is read as a raw
/// [`Value`] rather than `Json<api::prune::Request>` so an HTTP-only
/// `confirm` field can ride alongside the real prune fields without adding
/// `confirm` to `api::prune::Request` itself — that struct's
/// `deny_unknown_fields` must keep mirroring exactly the CLI's `prune` args
/// (AC-12 parity), and `#[serde(flatten)]`ing it here would silently
/// disable that check.
async fn prune_apply(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("prune", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        let (body, dry_run) = split_dry_run(body)?;
        let (mut req, confirmed) = split_confirm::<api::prune::Request>(body)?;
        if let Some(dry_run) = dry_run {
            req.apply = !dry_run;
        }
        require_confirm(confirmed)?;
        let cfg = state.cfg();
        let mut conn = state.conn()?;
        let mut ctx = Ctx::borrowed(state.paths(), &cfg, &mut conn);
        api::prune::run(&mut ctx, req)
    })
    .await;
    respond("prune", result, started)
}

/// Extract and remove the HTTP-only `"dry_run"` field from a raw JSON body
/// (console-api spec §9). It is the *inverse* of `api::prune::Request`'s
/// `apply`, and HTTP-only for the same reason `confirm` is: the CLI already
/// spells this as the absence of `--apply`, and adding a second name for it
/// to the shared `Request` would break the clap↔HTTP field parity walk.
///
/// A present-but-non-boolean value is a `400 bad_request`: `"dry_run":
/// "yes"` almost always means the caller meant `true` and typed a string,
/// but `"dry_run": "false"` means the opposite and would silently delete —
/// or silently not — under any coercion rule we could pick. Rejecting is
/// the only reading that cannot surprise. `None` (key absent) leaves
/// `apply` exactly as the body spelled it, and when both keys are present
/// `dry_run` wins, since it is the safer of the two.
fn split_dry_run(mut body: Value) -> Result<(Value, Option<bool>)> {
    let raw = body.as_object_mut().and_then(|obj| obj.remove("dry_run"));
    let dry_run = match raw {
        None => None,
        Some(Value::Bool(b)) => Some(b),
        Some(other) => {
            return Err(Error::BadRequest(format!(
                "'dry_run' must be a boolean, got {other}"
            )));
        }
    };
    Ok((body, dry_run))
}

/// `?confirm=true`-equivalent body for `POST /api/v1/gc`: `api::gc::Request`
/// is `{}` (no CLI args), so the only body field is the HTTP-only confirm
/// flag.
#[derive(Deserialize)]
struct ConfirmBody {
    #[serde(default)]
    confirm: bool,
}

/// `POST /api/v1/gc` — confirm-gated trash + telemetry sweep (`api::gc`).
async fn gc(State(state): State<AppState>, Json(body): Json<ConfirmBody>) -> Response {
    let started = Instant::now();
    let permit = match guard_mutating("gc", &state) {
        Ok(permit) => permit,
        Err(resp) => return *resp,
    };
    let result = run_blocking(move || {
        let _permit = permit;
        require_confirm(body.confirm)?;
        let cfg = state.cfg();
        let mut ctx = Ctx::lazy(state.paths(), &cfg);
        api::gc::run(&mut ctx, api::gc::Request {})
    })
    .await;
    respond("gc", result, started)
}

/// Extract and remove the HTTP-only `"confirm"` field from a raw JSON body,
/// then deserialize the rest as `T` (an `api::<cmd>::Request`, which keeps
/// enforcing its own `deny_unknown_fields` on every field besides
/// `confirm`). Shared by every route whose confirm gate must not leak into
/// the corresponding `api::` `Request` type (see [`prune_apply`]'s doc).
pub(crate) fn split_confirm<T: serde::de::DeserializeOwned>(body: Value) -> Result<(T, bool)> {
    let Value::Object(mut obj) = body else {
        return Err(Error::BadRequest("expected a JSON object body".into()));
    };
    let confirmed = obj
        .remove("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let req = serde_json::from_value(Value::Object(obj)).map_err(Error::Json)?;
    Ok((req, confirmed))
}

#[cfg(test)]
#[path = "../tests/prune_ids.rs"]
mod tests;
