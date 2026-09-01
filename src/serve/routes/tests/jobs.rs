#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Colocated coverage for `src/serve/routes/jobs.rs`'s SSE stream assembly
//! — the delivery guarantee `status_stream` documents: every `log` line and
//! the latest `progress` report published BEFORE a terminal status go out
//! ahead of it, and the terminal event is the last one. Driven over a real
//! [`Registry`] (subscribed first, then written exactly as a worker writes
//! it) through the real [`Sse`] response, with the wire-format body read
//! back off it — no hand-built streams, no mocks.

use axum::response::IntoResponse;
use axum::response::sse::Sse;
use comemory::serve::jobs::registry::CancelOutcome;
use comemory::serve::jobs::{JobStatus, Progress, Registry};
use serde_json::json;
use tokio::sync::{broadcast, watch};

use super::status_stream;

/// The three receivers `events` hands `status_stream`, taken BEFORE the
/// job is written to: a `broadcast` receiver only sees lines sent after it
/// subscribed, which is the same ordering a real SSE client gets.
struct Feeds {
    status: watch::Receiver<JobStatus>,
    progress: watch::Receiver<Option<Progress>>,
    log: broadcast::Receiver<String>,
}

/// Subscribe to all three of job `id`'s channels.
fn subscribe(registry: &Registry, id: &str) -> Feeds {
    Feeds {
        status: registry.subscribe(id).expect("lock").expect("job present"),
        progress: registry
            .subscribe_progress(id)
            .expect("lock")
            .expect("job present"),
        log: registry
            .subscribe_log(id)
            .expect("lock")
            .expect("job present"),
    }
}

/// Collect the FULL SSE body for `id` from already-taken `feeds`. The
/// stream must end on its own (terminal status) — a hang here IS the
/// failure this suite is about.
async fn collect_sse(id: &str, feeds: Feeds) -> String {
    let stream = status_stream(id.to_string(), feeds.status, feeds.progress, feeds.log);
    let response = Sse::new(stream).into_response();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("collect sse body");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Byte offset of `needle` in `haystack`, with a readable failure.
fn offset(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` missing from SSE body:\n{haystack}"))
}

#[tokio::test]
async fn a_terminal_status_flushes_pending_log_and_progress_first() {
    let registry = Registry::new();
    let accepted = registry.insert("index-code").expect("insert");
    let id = accepted.id.clone();
    let feeds = subscribe(&registry, &id);

    // Publish exactly as a worker does, from one thread, in order.
    registry
        .set_status(&id, JobStatus::Running)
        .expect("running");
    registry
        .push_log(&id, "indexing src/lib.rs".into())
        .expect("log 1");
    registry
        .push_log(&id, "indexing src/main.rs".into())
        .expect("log 2");
    registry
        .set_progress(
            &id,
            Progress {
                done: 2,
                total: 2,
                unit: "files".into(),
            },
        )
        .expect("progress");
    registry
        .set_status(&id, JobStatus::Done(json!({"indexed": 2})))
        .expect("done");

    let body = collect_sse(&id, feeds).await;

    let done_at = offset(&body, "event: done");
    assert_eq!(
        body.matches("event: done").count(),
        1,
        "exactly one terminal event:\n{body}"
    );
    assert_eq!(
        body.rfind("event: "),
        Some(done_at),
        "the terminal event is the LAST event:\n{body}"
    );
    assert!(
        offset(&body, "indexing src/lib.rs") < done_at,
        "a log line published before the terminal status must precede it:\n{body}"
    );
    assert!(
        offset(&body, "indexing src/main.rs") < done_at,
        "every pending log line is drained, not just the first:\n{body}"
    );
    assert!(
        offset(&body, "indexing src/lib.rs") < offset(&body, "indexing src/main.rs"),
        "log lines keep their publish order:\n{body}"
    );
    assert!(
        offset(&body, "event: progress") < done_at,
        "the pending progress report precedes the terminal event:\n{body}"
    );
    assert!(
        body.contains(r#""done":2"#),
        "the progress event carries the report:\n{body}"
    );
}

#[tokio::test]
async fn a_job_cancelled_while_queued_streams_one_cancelled_event() {
    let registry = Registry::new();
    let accepted = registry.insert("index-code").expect("insert");
    let id = accepted.id.clone();
    let feeds = subscribe(&registry, &id);

    assert_eq!(
        registry.cancel(&id).expect("cancel"),
        CancelOutcome::Cancelled,
        "a queued job is cancelled outright, not merely requested"
    );

    let body = collect_sse(&id, feeds).await;
    assert_eq!(
        body.matches("event: cancelled").count(),
        1,
        "one terminal cancelled event:\n{body}"
    );
    assert!(
        !body.contains("event: running") && !body.contains("event: done"),
        "a job cancelled while queued never ran:\n{body}"
    );
}
