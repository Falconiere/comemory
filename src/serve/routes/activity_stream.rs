//! `GET /api/v1/activity/events` — the activity feed as a live SSE stream.
//!
//! The handler owns a cursor and polls for rows above it every
//! `activity.stream_poll_ms`, emitting one `activity` event per row in id
//! order. No channel, no fan-out: the table is the buffer, so a slow client
//! reads further behind and nothing is dropped — unlike [`super::jobs`]'s log
//! stream, whose `broadcast` exists because job progress is unpersisted.
//!
//! Conventions cloned from [`super::jobs`]: `?token=` auth (an `EventSource`
//! cannot set headers), enveloped errors, and a shape-stable last word when a
//! payload will not serialize. The event `id` is the row id, so a reconnect
//! resumes above it via `Last-Event-ID`; `?after_id=` wins over the header,
//! and with neither the stream starts at the newest row.

use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::Stream;
use serde::Deserialize;

use crate::domains::maintenance::activity::Item;
use crate::prelude::*;
use crate::serve::AppState;
use crate::serve::envelope::Envelope;
use crate::serve::routes::RouteEntry;
use crate::serve::scope::RepoScope;
use crate::store::activity::{self, ActivityFilter};
use crate::utilities::blocking::run_blocking;

/// Emitted (and the stream ended) when a row cannot be serialized — a
/// shape-stable last word instead of a silently truncated stream, exactly as
/// `super::jobs` does.
const ENCODE_FAILED: &str = r#"{"status":"error","error":{"code":"internal","message":"activity event serialization failed"}}"#;

/// Rows read per poll. A burst larger than this is drained over the next
/// ticks rather than in one oversized read.
const BATCH: usize = 100;

/// How long the stream waits between empty polls before sending a comment
/// line, so an idle connection is not closed by an intermediary.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// This resource's route-table entries, appended onto [`super::table`].
pub fn table_entries() -> &'static [RouteEntry] {
    &[RouteEntry {
        method: "GET",
        path: "/activity/events",
        command: "activity.events",
        mutating: false,
    }]
}

/// This resource's routes, mounted under `/api/v1`.
pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/api/v1/activity/events", get(events))
}

/// The stream's filters, the same vocabulary [`super::activity`] pages with.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
struct StreamQuery {
    /// Start above this row id. Overrides `Last-Event-ID`.
    #[serde(default)]
    after_id: Option<i64>,
    /// Only runs scoped to this repo label.
    #[serde(default)]
    repo: Option<String>,
    /// Only this command.
    #[serde(default)]
    command: Option<String>,
    /// Only this delivery surface.
    #[serde(default)]
    source: Option<String>,
    /// Only this caller label.
    #[serde(default)]
    actor: Option<String>,
}

/// The cursor one unfold step carries.
struct Cursor {
    /// Handler state: connection, config, paths.
    state: AppState,
    /// Filters, resolved once at connect.
    query: StreamQuery,
    /// Rows already delivered, up to and including this id.
    last_id: i64,
    /// Rows read but not yet emitted, oldest first.
    pending: std::collections::VecDeque<Item>,
    /// Wait between polls that found nothing.
    tick: Duration,
}

/// `GET /api/v1/activity/events` — stream rows as they are recorded.
async fn events(
    State(state): State<AppState>,
    scope: RepoScope,
    headers: HeaderMap,
    Query(mut query): Query<StreamQuery>,
) -> Response {
    query.repo = scope.resolve(query.repo);
    let start = match resolve_cursor(&state, query.after_id, &headers).await {
        Ok(id) => id,
        Err(e) => return Envelope::err("activity.events", &e, 0),
    };
    let tick = Duration::from_millis(state.cfg().activity.stream_poll_ms);
    let cursor = Cursor {
        state,
        query,
        last_id: start,
        pending: std::collections::VecDeque::new(),
        tick,
    };
    Sse::new(stream(cursor))
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response()
}

/// Where to start: the caller's `after_id`, else `Last-Event-ID` from a
/// reconnecting `EventSource`, else the newest row at connect.
async fn resolve_cursor(
    state: &AppState,
    after_id: Option<i64>,
    headers: &HeaderMap,
) -> Result<i64> {
    if let Some(id) = after_id {
        return Ok(id);
    }
    if let Some(id) = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
    {
        return Ok(id);
    }
    let state = state.clone();
    run_blocking(move || {
        let conn = state.conn()?;
        activity::newest_id(&conn)
    })
    .await
}

/// One `activity` event per recorded row, forever — until the client goes
/// away and the response stream is dropped with it.
fn stream(cursor: Cursor) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    futures::stream::unfold(Some(cursor), |state| async move {
        let mut cursor = state?;
        loop {
            if let Some(item) = cursor.pending.pop_front() {
                cursor.last_id = item.id;
                return Some(emit(cursor, &item));
            }
            match read_batch(&cursor).await {
                Ok(rows) if rows.is_empty() => tokio::time::sleep(cursor.tick).await,
                Ok(rows) => cursor.pending.extend(rows),
                // A failed read is transient (a rebuild swapping the
                // database underneath, a busy writer): wait one tick and
                // try again rather than ending a live stream.
                Err(e) => {
                    tracing::warn!(error = %e, "activity stream poll failed");
                    tokio::time::sleep(cursor.tick).await;
                }
            }
        }
    })
}

/// Read up to [`BATCH`] rows above the cursor, oldest first.
async fn read_batch(cursor: &Cursor) -> Result<Vec<Item>> {
    let state = cursor.state.clone();
    let after_id = cursor.last_id;
    let repo = cursor.query.repo.clone();
    let command = cursor.query.command.clone();
    let source = cursor.query.source.clone();
    let actor = cursor.query.actor.clone();
    run_blocking(move || {
        let filter = ActivityFilter {
            repo: repo.as_deref(),
            command: command.as_deref(),
            source: source.as_deref(),
            actor: actor.as_deref(),
            since: None,
            after_id: Some(after_id),
        };
        let conn = state.conn()?;
        let rows = activity::since_cursor(&conn, &filter, BATCH)?;
        Ok(rows.into_iter().map(Item::from_row).collect())
    })
    .await
}

/// Shape one row into its `activity` event, carrying the row id as the SSE
/// event id so a reconnect resumes above it.
fn emit(cursor: Cursor, item: &Item) -> (std::result::Result<Event, Infallible>, Option<Cursor>) {
    match Event::default()
        .event("activity")
        .id(item.id.to_string())
        .json_data(item)
    {
        Ok(event) => (Ok(event), Some(cursor)),
        Err(e) => {
            tracing::warn!(error = %e, id = item.id, "activity event not serialized");
            (Ok(Event::default().data(ENCODE_FAILED)), None)
        }
    }
}
