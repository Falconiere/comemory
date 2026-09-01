//! `GET /api/v1/jobs`, `GET /api/v1/jobs/{id}` and the
//! `GET /api/v1/jobs/{id}/events` SSE stream over `serve::jobs`. Reads
//! only — job-creating `POST`s live with their own resources.
//!
//! An unknown id is a `404 not_found` enveloped JSON response on all three,
//! the SSE route included (a client cannot mistake "no such job" for "a
//! job that never emits"). Auth accepts `?token=` too — `EventSource`
//! cannot send headers (AC-11).
//!
//! The stream interleaves two event types: `status`
//! (`queued`/`running`/`done`/`error`, unchanged) and an additive
//! `progress` — see [`status_stream`].

use std::convert::Infallible;
use std::time::Instant;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::Stream;
use serde::Deserialize;
use tokio::sync::watch;

use crate::output::page::Page;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs::{JobEvent, JobStatus, Progress, ProgressEvent};
use crate::serve::routes::{RouteEntry, respond};

/// Emitted (and the stream ended) when a status payload cannot be
/// serialized — a shape-stable last word instead of a silently truncated
/// stream.
const ENCODE_FAILED: &str =
    r#"{"status":"error","error":{"code":"internal","message":"job event serialization failed"}}"#;

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[
        RouteEntry {
            method: "GET",
            path: "/jobs",
            command: "jobs.list",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/jobs/{id}",
            command: "jobs.get",
            mutating: false,
        },
        RouteEntry {
            method: "GET",
            path: "/jobs/{id}/events",
            command: "jobs.events",
            mutating: false,
        },
    ]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/jobs", get(list))
        .route("/api/v1/jobs/{id}", get(get_one))
        .route("/api/v1/jobs/{id}/events", get(events))
}

/// `?limit=&offset=` on `GET /jobs` — transport-level windowing over the
/// in-memory registry; there is no CLI counterpart to mirror.
#[derive(Deserialize)]
struct ListQuery {
    /// Page size; `0` is `Page`'s "all" sentinel.
    #[serde(default = "default_limit")]
    limit: usize,
    /// Leading jobs to skip.
    #[serde(default)]
    offset: usize,
}

/// Same default page size as the other paged routes (`api::list`).
fn default_limit() -> usize {
    50
}

/// `GET /api/v1/jobs` — every retained job, newest first, paged. Served
/// straight off the registry (a short mutex hold, no DB and no `.await`),
/// so it does not go through `run_blocking`.
async fn list(State(state): State<AppState>, Query(query): Query<ListQuery>) -> Response {
    let started = Instant::now();
    let result = state
        .jobs()
        .list()
        .map(|jobs| Page::from_slice(jobs, query.limit, query.offset));
    respond("jobs.list", result, started)
}

/// `GET /api/v1/jobs/{id}` — one job's record, `404 not_found` when the id
/// is unknown or has aged out of the retention window.
async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let result = state
        .jobs()
        .get(&id)
        .and_then(|found| found.ok_or_else(|| Error::NotFound(format!("job not found: {id}"))));
    respond("jobs.get", result, started)
}

/// `GET /api/v1/jobs/{id}/events` — the job's lifecycle as `text/event-stream`.
///
/// The first event is an explicit read of the current status, so a client
/// attaching after the job finished immediately gets the terminal event
/// (AC-8). Later events are the two `watch` channels' transitions —
/// best-effort, since `watch` keeps only the latest value and fast
/// transitions coalesce. The handler ends the stream itself once a terminal
/// status is emitted (the registry retains both senders, so neither channel
/// closes on its own).
async fn events(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let status_rx = match subscribed_or_err(state.jobs().subscribe(&id), &id) {
        Ok(rx) => rx,
        Err(resp) => return *resp,
    };
    let progress_rx = match subscribed_or_err(state.jobs().subscribe_progress(&id), &id) {
        Ok(rx) => rx,
        Err(resp) => return *resp,
    };
    Sse::new(status_stream(id, status_rx, progress_rx)).into_response()
}

/// Unwrap one `Registry::subscribe*` result into its channel, or the same
/// enveloped `404`/error [`events`] otherwise returns.
///
/// The error side is boxed because an axum `Response` is large (>=128 bytes)
/// and this `Result` is returned by value on every subscribe — `clippy::
/// result_large_err` rejects the unboxed form.
fn subscribed_or_err<T>(
    result: Result<Option<T>>,
    id: &str,
) -> std::result::Result<T, Box<Response>> {
    match result {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(Box::new(Envelope::err(
            "jobs.events",
            &Error::NotFound(format!("job not found: {id}")),
            0,
        ))),
        Err(e) => Err(Box::new(Envelope::err("jobs.events", &e, 0))),
    }
}

/// The unfold cursor: `None` state ends the stream.
struct Cursor {
    /// Job id, echoed into every event payload.
    id: String,
    /// The job's status channel.
    status_rx: watch::Receiver<JobStatus>,
    /// The job's progress channel — see the module doc's `progress` event.
    progress_rx: watch::Receiver<Option<Progress>>,
    /// Whether the next poll is the initial current-state emission.
    first: bool,
}

/// Build the SSE event stream described on [`events`]: the first item is
/// always the job's current status (AC-8, unchanged); afterward, whichever
/// of `status_rx` / `progress_rx` changes first is emitted, until a
/// terminal status ends the stream — so `progress` events can interleave
/// with `status` events but never delay or replace one (AC-34).
fn status_stream(
    id: String,
    status_rx: watch::Receiver<JobStatus>,
    progress_rx: watch::Receiver<Option<Progress>>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    let cursor = Cursor {
        id,
        status_rx,
        progress_rx,
        first: true,
    };
    futures::stream::unfold(Some(cursor), |state| async move {
        let mut cursor = state?;
        if cursor.first {
            cursor.first = false;
            let status = cursor.status_rx.borrow_and_update().clone();
            return Some(finish_on_status(cursor, &status));
        }
        loop {
            tokio::select! {
                changed = cursor.status_rx.changed() => {
                    if changed.is_err() { return None; }
                    let status = cursor.status_rx.borrow_and_update().clone();
                    return Some(finish_on_status(cursor, &status));
                }
                changed = cursor.progress_rx.changed() => {
                    if changed.is_err() { return None; }
                    let progress = cursor.progress_rx.borrow_and_update().clone();
                    if let Some(p) = progress {
                        let (event, ok) = encode_progress(&cursor.id, &p);
                        return Some((Ok(event), if ok { Some(cursor) } else { None }));
                    }
                    // A spurious `None` change cannot happen once
                    // `set_progress` has published at least once — keep
                    // polling either channel.
                }
            }
        }
    })
}

/// Encode a status transition and decide whether the stream ends: on a
/// terminal status, or when serialization itself failed.
fn finish_on_status(
    cursor: Cursor,
    status: &JobStatus,
) -> (std::result::Result<Event, Infallible>, Option<Cursor>) {
    let (event, ok) = encode_status(&cursor.id, status);
    let next = if status.is_terminal() || !ok {
        None
    } else {
        Some(cursor)
    };
    (Ok(event), next)
}

/// Render one status as an SSE event named after the status slug. Returns
/// `false` alongside the fallback event when the payload could not be
/// serialized, which ends the stream.
fn encode_status(id: &str, status: &JobStatus) -> (Event, bool) {
    match serde_json::to_string(&JobEvent::new(id, status)) {
        Ok(json) => (Event::default().event(status.slug()).data(json), true),
        Err(e) => {
            tracing::warn!(job_id = id, error = %e, "job event serialization failed");
            (Event::default().event("error").data(ENCODE_FAILED), false)
        }
    }
}

/// Render one progress report as the SSE `progress` event. Same failure
/// contract as [`encode_status`].
fn encode_progress(id: &str, progress: &Progress) -> (Event, bool) {
    match serde_json::to_string(&ProgressEvent::new(id, progress)) {
        Ok(json) => (Event::default().event("progress").data(json), true),
        Err(e) => {
            tracing::warn!(job_id = id, error = %e, "job progress event serialization failed");
            (Event::default().event("error").data(ENCODE_FAILED), false)
        }
    }
}
