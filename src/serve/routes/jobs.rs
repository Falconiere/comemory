//! `GET /api/v1/jobs`, `GET /api/v1/jobs/{id}` and the
//! `GET /api/v1/jobs/{id}/events` SSE stream over `serve::jobs`. Reads
//! only — job-creating `POST`s live with their own resources.
//!
//! An unknown id is a `404 not_found` enveloped JSON response on all three,
//! the SSE route included (a client cannot mistake "no such job" for "a
//! job that never emits"). Auth accepts `?token=` too — `EventSource`
//! cannot send headers (AC-11).
//!
//! The stream interleaves three event types: `status`
//! (`queued`/`running`/`done`/`error`/`cancelled`), an additive
//! `progress`, and an additive `log` (one per log line) — see
//! [`status_stream`]; the terminal `status` is always the last event and
//! never overtakes a `log`/`progress` published before it.
//! `POST /api/v1/jobs/{id}/cancel` is the one write
//! here: it flips the job's cooperative cancel flag (`Registry::cancel`)
//! and is deliberately not read-only-gated — stopping a job never writes
//! to the store, and a `--read-only` server can still run read-class jobs
//! worth stopping.

use std::convert::Infallible;
use std::time::Instant;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::Stream;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::{broadcast, watch};

use crate::output::page::Page;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::jobs::registry::CancelOutcome;
use crate::serve::jobs::{JobEvent, JobStatus, LogEvent, Progress, ProgressEvent};
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
        RouteEntry {
            method: "POST",
            path: "/jobs/{id}/cancel",
            command: "jobs.cancel",
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
        .route("/api/v1/jobs/{id}/cancel", post(cancel))
}

/// `POST /api/v1/jobs/{id}/cancel` — cooperative cancel. `data` reports
/// what happened: `{job_id, outcome: "cancelled"}` for a queued job (now
/// terminal), `{job_id, outcome: "requested"}` for a running one (stops at
/// its next boundary). `404 not_found` for an unknown id, `400 bad_request`
/// for a job that already finished.
async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let started = Instant::now();
    let result = state.jobs().cancel(&id).map(|outcome| {
        let slug = match outcome {
            CancelOutcome::Cancelled => "cancelled",
            CancelOutcome::Requested => "requested",
        };
        json!({ "job_id": id, "outcome": slug })
    });
    respond("jobs.cancel", result, started)
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
    let log_rx = match subscribed_or_err(state.jobs().subscribe_log(&id), &id) {
        Ok(rx) => rx,
        Err(resp) => return *resp,
    };
    Sse::new(status_stream(id, status_rx, progress_rx, log_rx)).into_response()
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
    /// The job's log-line channel — see the module doc's `log` event.
    /// `None` once the channel closed (the arm is retired, never polled
    /// again, so a closed channel cannot spin the select loop).
    log_rx: Option<broadcast::Receiver<String>>,
    /// Whether the next poll is the initial current-state emission.
    first: bool,
    /// A terminal status already observed but not yet emitted: the stream
    /// is draining the log/progress channels ahead of it
    /// ([`drain_then_finish`]).
    terminal: Option<JobStatus>,
}

/// One unfold step: the event to yield, and the cursor to continue with
/// (`None` ends the stream).
type Step = (std::result::Result<Event, Infallible>, Option<Cursor>);

/// Build the SSE event stream described on [`events`]: the first item is
/// always the job's current status (AC-8, unchanged); afterward, whichever
/// of `log_rx` / `progress_rx` / `status_rx` is ready first is emitted — in
/// that priority when several are (`biased`) — until a terminal status
/// ends the stream, so `progress` and `log` events can interleave with
/// `status` events but never delay or replace one (AC-34).
///
/// Delivery guarantee: the terminal `status` event is the last event, and
/// every log line and the latest progress report published BEFORE that
/// transition go out ahead of it. The worker publishes them in order from
/// one thread, so they already sit in their channels when the terminal
/// status lands, and [`drain_then_finish`] empties those channels first.
/// Only `Lagged` log lines (a subscriber `LOG_CHANNEL_CAP` behind) are
/// lost, as the `log` event's contract says.
fn status_stream(
    id: String,
    status_rx: watch::Receiver<JobStatus>,
    progress_rx: watch::Receiver<Option<Progress>>,
    log_rx: broadcast::Receiver<String>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    let cursor = Cursor {
        id,
        status_rx,
        progress_rx,
        log_rx: Some(log_rx),
        first: true,
        terminal: None,
    };
    futures::stream::unfold(Some(cursor), |state| async move {
        let mut cursor = state?;
        if cursor.first {
            cursor.first = false;
            let status = cursor.status_rx.borrow_and_update().clone();
            return Some(on_status(cursor, status));
        }
        if let Some(status) = cursor.terminal.take() {
            return Some(drain_then_finish(cursor, status));
        }
        loop {
            tokio::select! {
                biased;
                line = next_log(&mut cursor.log_rx) => {
                    match line {
                        Some(line) => return Some(emit_log(cursor, &line)),
                        None => cursor.log_rx = None,
                    }
                }
                changed = cursor.progress_rx.changed() => {
                    if changed.is_err() { return None; }
                    let progress = cursor.progress_rx.borrow_and_update().clone();
                    if let Some(p) = progress {
                        return Some(emit_progress(cursor, &p));
                    }
                    // A spurious `None` change cannot happen once
                    // `set_progress` has published at least once — keep
                    // polling every channel.
                }
                changed = cursor.status_rx.changed() => {
                    if changed.is_err() { return None; }
                    let status = cursor.status_rx.borrow_and_update().clone();
                    return Some(on_status(cursor, status));
                }
            }
        }
    })
}

/// The next log line off `rx`, skipping a `Lagged` gap (the tail is the
/// durable record); `None` once the channel is closed, which retires the
/// arm. With the arm already retired (`rx` is `None`) this never resolves,
/// so the select loop only wakes for status/progress changes.
async fn next_log(rx: &mut Option<broadcast::Receiver<String>>) -> Option<String> {
    let Some(rx) = rx.as_mut() else {
        return std::future::pending().await;
    };
    loop {
        match rx.recv().await {
            Ok(line) => return Some(line),
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// One log line already sitting in `rx`, without waiting — the drain's
/// counterpart of [`next_log`]. `None` when the channel is empty, closed,
/// or the arm is retired; a `Lagged` gap is skipped the same way.
fn try_next_log(rx: &mut Option<broadcast::Receiver<String>>) -> Option<String> {
    let rx = rx.as_mut()?;
    loop {
        match rx.try_recv() {
            Ok(line) => return Some(line),
            Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty | TryRecvError::Closed) => return None,
        }
    }
}

/// Route a freshly read status: a terminal one drains the other two
/// channels ahead of itself ([`drain_then_finish`]); a non-terminal one is
/// emitted as-is.
fn on_status(cursor: Cursor, status: JobStatus) -> Step {
    if status.is_terminal() {
        drain_then_finish(cursor, status)
    } else {
        finish_on_status(cursor, &status)
    }
}

/// With terminal `status` in hand, yield ONE still-pending `log` line or
/// `progress` report and park the status on the cursor for the next poll;
/// only once both channels are empty does the terminal event itself go
/// out, ending the stream.
fn drain_then_finish(mut cursor: Cursor, status: JobStatus) -> Step {
    if let Some(line) = try_next_log(&mut cursor.log_rx) {
        cursor.terminal = Some(status);
        return emit_log(cursor, &line);
    }
    if cursor.progress_rx.has_changed().unwrap_or(false) {
        let progress = cursor.progress_rx.borrow_and_update().clone();
        if let Some(p) = progress {
            cursor.terminal = Some(status);
            return emit_progress(cursor, &p);
        }
    }
    finish_on_status(cursor, &status)
}

/// Encode a status transition and decide whether the stream ends: on a
/// terminal status, or when serialization itself failed.
fn finish_on_status(cursor: Cursor, status: &JobStatus) -> Step {
    let (event, ok) = encode_status(&cursor.id, status);
    let next = if status.is_terminal() || !ok {
        None
    } else {
        Some(cursor)
    };
    (Ok(event), next)
}

/// Yield one `log` event, keeping the stream open unless encoding failed.
fn emit_log(cursor: Cursor, line: &str) -> Step {
    let (event, ok) = encode_log(&cursor.id, line);
    (Ok(event), ok.then_some(cursor))
}

/// Yield one `progress` event, keeping the stream open unless encoding
/// failed.
fn emit_progress(cursor: Cursor, progress: &Progress) -> Step {
    let (event, ok) = encode_progress(&cursor.id, progress);
    (Ok(event), ok.then_some(cursor))
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

/// Render one log line as the SSE `log` event. Same failure contract as
/// [`encode_status`].
fn encode_log(id: &str, line: &str) -> (Event, bool) {
    match serde_json::to_string(&LogEvent::new(id, line)) {
        Ok(json) => (Event::default().event("log").data(json), true),
        Err(e) => {
            tracing::warn!(job_id = id, error = %e, "job log event serialization failed");
            (Event::default().event("error").data(ENCODE_FAILED), false)
        }
    }
}

#[cfg(test)]
#[path = "tests/jobs.rs"]
mod tests;
