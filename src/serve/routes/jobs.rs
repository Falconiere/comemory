//! `GET /api/v1/jobs`, `GET /api/v1/jobs/{id}` and the
//! `GET /api/v1/jobs/{id}/events` SSE stream over `serve::jobs`. All three
//! are reads — they observe the job registry, they never start a job (the
//! job-creating `POST`s live with their own resources).
//!
//! An unknown id is a `404 not_found` **enveloped JSON** response on all
//! three, the SSE route included: the stream is only opened for a job that
//! exists, so a client cannot mistake "no such job" for "a job that never
//! emits". Auth is the router `guard`'s, which already accepts `?token=` —
//! the form `EventSource` needs, since it cannot send headers (AC-11).

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
use crate::serve::jobs::{JobEvent, JobStatus};
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
/// (AC-8). Later events are the `watch` channel's transitions —
/// best-effort, since `watch` keeps only the latest value and fast
/// transitions coalesce. The handler ends the stream itself once a terminal
/// status is emitted (the registry retains the sender, so the channel never
/// closes on its own).
async fn events(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let receiver = match state.jobs().subscribe(&id) {
        Ok(Some(rx)) => rx,
        Ok(None) => {
            let e = Error::NotFound(format!("job not found: {id}"));
            return Envelope::err("jobs.events", &e, 0);
        }
        Err(e) => return Envelope::err("jobs.events", &e, 0),
    };
    Sse::new(status_stream(id, receiver)).into_response()
}

/// The unfold cursor: `None` state ends the stream.
struct Cursor {
    /// Job id, echoed into every event payload.
    id: String,
    /// The job's status channel.
    receiver: watch::Receiver<JobStatus>,
    /// Whether the next poll is the initial current-state emission.
    first: bool,
}

/// Build the SSE event stream described on [`events`].
fn status_stream(
    id: String,
    receiver: watch::Receiver<JobStatus>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    let cursor = Cursor {
        id,
        receiver,
        first: true,
    };
    futures::stream::unfold(Some(cursor), |state| async move {
        let mut cursor = state?;
        if cursor.first {
            cursor.first = false;
        } else if cursor.receiver.changed().await.is_err() {
            // The registry dropped the sender (the job was evicted): there
            // is nothing further to report, so end the stream.
            return None;
        }
        let status = cursor.receiver.borrow_and_update().clone();
        let (event, encoded) = encode(&cursor.id, &status);
        let next = if status.is_terminal() || !encoded {
            None
        } else {
            Some(cursor)
        };
        Some((Ok(event), next))
    })
}

/// Render one status as an SSE event named after the status slug. Returns
/// `false` alongside the fallback event when the payload could not be
/// serialized, which ends the stream.
fn encode(id: &str, status: &JobStatus) -> (Event, bool) {
    match serde_json::to_string(&JobEvent::new(id, status)) {
        Ok(json) => (Event::default().event(status.slug()).data(json), true),
        Err(e) => {
            tracing::warn!(job_id = id, error = %e, "job event serialization failed");
            (Event::default().event("error").data(ENCODE_FAILED), false)
        }
    }
}
