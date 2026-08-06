//! The versioned `/api/v1` REST surface: one file per resource, merged by
//! this module's route [`table`] (source of truth for the read-only gate's
//! `mutating` flag and `GET /commands`, both later steps) and `v1_router`
//! assembly. Also owns the handler-layer helpers every resource reuses:
//! [`run_blocking`] (run `api::<cmd>::run` — and the connection lock it
//! takes — on a blocking-pool thread, never across an `.await`),
//! [`respond`] (envelope the result), and [`guard_mutating`] (the
//! read-only/write-permit gate every mutating route calls first).

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;
use serde_json::json;
use tokio::sync::OwnedSemaphorePermit;

use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;

/// `GET|POST /code/search`.
pub mod code;
/// `GET /graph`, `GET /edges`.
pub mod graph;
/// `GET /doctor`, `GET /consolidate`, `GET /prune`.
pub mod maint;
/// `GET /memories`, `GET /memories/{id}`, `GET|POST /memories/search`.
pub mod memories;
/// `GET /completions`, `GET /commands`.
pub mod meta;
/// `GET /sources`.
pub mod sources;

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

/// This module's own routes, prepended to every resource's `table_entries`.
const BASE: &[RouteEntry] = &[RouteEntry {
    method: "GET",
    path: "/health",
    command: "health",
    mutating: false,
}];

/// The full `/api/v1` route table: this module's own entries plus every
/// resource module's, growing one entry per route as each later step mounts
/// one. A fresh `Vec` per call (not a `'static` slice) since it is a small,
/// infrequent aggregation over per-resource `const` arrays — concatenating
/// `const` slices at compile time buys nothing here.
pub fn table() -> Vec<RouteEntry> {
    let mut entries: Vec<RouteEntry> = BASE.to_vec();
    entries.extend_from_slice(memories::table_entries());
    entries.extend_from_slice(memories::write::table_entries());
    entries.extend_from_slice(code::table_entries());
    entries.extend_from_slice(graph::table_entries());
    entries.extend_from_slice(maint::table_entries());
    entries.extend_from_slice(maint::prune::table_entries());
    entries.extend_from_slice(sources::table_entries());
    entries.extend_from_slice(meta::table_entries());
    entries
}

/// Mount the `/api/v1` surface. `state` is threaded through so per-resource
/// route construction can depend on it (e.g. conditionally mounting
/// mutating routes in a later step).
pub fn v1_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/health", get(health))
        .merge(memories::router(state.clone()))
        .merge(code::router(state.clone()))
        .merge(graph::router(state.clone()))
        .merge(maint::router(state.clone()))
        .merge(sources::router(state.clone()))
        .merge(meta::router(state))
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

/// Run `f` on the blocking-thread-pool, flattening a `JoinError` (task
/// panic) into the crate `Error` so callers can just `?` through it. Shared
/// by every `/api/v1` route: `api::<cmd>::run`'s DB work — and the
/// `MutexGuard` it takes on `AppState`'s shared connection — must never run
/// on (or cross an `.await` on) the async runtime's own worker threads;
/// running the whole closure, guard included, inside the blocking task
/// satisfies both.
pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("blocking task panicked: {e}")))?
}

/// Gate a confirm-required mutating route: `Ok(())` when `confirmed`, else
/// `Err(Error::ConfirmationRequired)` (`400 confirmation_required`).
/// Transport-agnostic — POST routes source `confirmed` from `"confirm":
/// true` on their own `Request` struct, DELETE routes from `?confirm=true`.
///
/// **Ordering (AC-19):** a `mutating` + confirm-gated route MUST check
/// `state.read_only()` first (→ `405 read_only`) and only call this when
/// not read-only — read-only outranks a missing confirm.
pub fn require_confirm(confirmed: bool) -> Result<()> {
    if confirmed {
        Ok(())
    } else {
        Err(Error::ConfirmationRequired(
            "this operation requires explicit confirmation".into(),
        ))
    }
}

/// Guard a mutating route before the write: `405 read_only` on a
/// `--read-only` server (checked FIRST — AC-19, read-only outranks a
/// missing confirm and outranks the write permit too), else a
/// `try_acquire` on the single write permit (§Concurrency), `503 busy` with
/// a `Retry-After` header on contention (AC-17). `Ok(permit)` must be held
/// for the duration of the blocking write — move it into the
/// `run_blocking` closure; dropping it releases the permit to the next
/// queued request/job.
///
/// Every mutating route calls this first, so both gates live in one place
/// (Binding Rule 1) rather than each handler re-deriving the read-only/busy
/// envelope on its own.
pub(crate) fn guard_mutating(
    command: &str,
    state: &AppState,
) -> std::result::Result<OwnedSemaphorePermit, Box<Response>> {
    if state.read_only() {
        return Err(Box::new(Envelope::read_only(command)));
    }
    Arc::clone(state.write_permit())
        .try_acquire_owned()
        .map_err(|_| Box::new(Envelope::busy(command)))
}

/// Whether a search-shaped route should record access counts /
/// `retrieval_log` rows: never on a read-only server (§Security "Read-only
/// side-effect degradation"), else the same `COMEMORY_DISABLE_ACCESS_TRACKING`
/// test hook the CLI honors — shared by `search`, `search-code`, and
/// `context` so the three cannot drift.
pub(crate) fn track_for(state: &AppState) -> Result<bool> {
    Ok(!state.read_only() && crate::cli::track_searches()?)
}

/// Envelope a blocking route's result: [`Envelope::ok`] on success,
/// [`Envelope::err`] (status/code derived from the crate `Error`)
/// otherwise. `started` feeds `meta.elapsed_ms`.
pub(crate) fn respond<T: Serialize>(
    command: &str,
    result: Result<T>,
    started: Instant,
) -> Response {
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(data) => Envelope::ok(command, data, elapsed_ms),
        Err(e) => Envelope::err(command, &e, elapsed_ms),
    }
}
