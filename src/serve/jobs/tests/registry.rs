#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/serve/jobs/registry.rs`, driven directly (no HTTP):
//! id generation, status transitions and timestamps, newest-first listing,
//! the bounded finished-job retention, and — the mechanism AC-8's SSE
//! replay rests on — a receiver subscribed *after* a job finished reading
//! the terminal status straight out of `borrow_and_update`.

use comemory::serve::jobs::registry::MAX_FINISHED;
use comemory::serve::jobs::{JobError, JobStatus, Registry};
use serde_json::json;

#[test]
fn insert_mints_a_16_hex_id_and_a_queued_record() {
    let registry = Registry::new();
    let (id, mut rx) = registry.insert("index-code").expect("insert");

    assert_eq!(id.len(), 16, "job id is 8 urandom bytes hex-encoded: {id}");
    assert!(
        id.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "job id must be lowercase hex: {id}"
    );
    assert_eq!(rx.borrow_and_update().slug(), "queued");

    let view = registry
        .get(&id)
        .expect("lock")
        .expect("job present after insert");
    assert_eq!(view.job_id, id);
    assert_eq!(view.command, "index-code");
    assert_eq!(view.status, "queued");
    assert!(view.finished_at.is_none());
    assert!(view.result.is_none());
    assert!(view.error.is_none());
    assert!(!view.started_at.is_empty());
}

#[test]
fn ids_are_distinct_across_insertions() {
    let registry = Registry::new();
    let mut ids = std::collections::HashSet::new();
    for _ in 0..32 {
        let (id, _rx) = registry.insert("eval").expect("insert");
        assert!(ids.insert(id), "job ids must not repeat");
    }
}

#[test]
fn set_status_publishes_transitions_and_stamps_the_finish() {
    let registry = Registry::new();
    let (id, mut rx) = registry.insert("rebuild").expect("insert");

    registry
        .set_status(&id, JobStatus::Running)
        .expect("mark running");
    assert_eq!(rx.borrow_and_update().slug(), "running");
    let view = registry.get(&id).expect("lock").expect("job");
    assert_eq!(view.status, "running");
    assert!(
        view.finished_at.is_none(),
        "a running job has no finish stamp"
    );

    registry
        .set_status(&id, JobStatus::Done(json!({"removed": 2})))
        .expect("mark done");
    let view = registry.get(&id).expect("lock").expect("job");
    assert_eq!(view.status, "done");
    assert_eq!(view.result, Some(json!({"removed": 2})));
    assert!(view.error.is_none());
    let finished = view.finished_at.expect("finish stamp");
    assert!(finished >= view.started_at);
}

#[test]
fn an_error_status_carries_the_envelope_error_object() {
    let registry = Registry::new();
    let (id, _rx) = registry.insert("index-code").expect("insert");
    let err = JobError::from_error(&comemory::errors::Error::BadRequest("no repo here".into()));

    registry
        .set_status(&id, JobStatus::Error(err))
        .expect("mark error");

    let view = registry.get(&id).expect("lock").expect("job");
    assert_eq!(view.status, "error");
    assert!(view.result.is_none());
    let error = view.error.expect("error object");
    assert_eq!(error.code, "bad_request");
    assert!(error.message.contains("no repo here"));
    assert!(view.finished_at.is_some());
}

#[test]
fn set_status_on_an_unknown_id_is_not_found() {
    let registry = Registry::new();
    let err = registry
        .set_status("deadbeefdeadbeef", JobStatus::Running)
        .expect_err("unknown id must not silently succeed");
    assert!(
        matches!(err, comemory::errors::Error::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
}

/// The exact mechanism `GET /jobs/{id}/events` relies on for AC-8: the
/// registry keeps the sender alive, so a receiver created after the job
/// already finished still reads the terminal status immediately, with no
/// `changed()` await.
#[test]
fn a_receiver_subscribed_after_completion_replays_the_terminal_status() {
    let registry = Registry::new();
    let (id, first_rx) = registry.insert("eval").expect("insert");
    registry
        .set_status(&id, JobStatus::Running)
        .expect("mark running");
    registry
        .set_status(&id, JobStatus::Done(json!({"mrr": 0.5})))
        .expect("mark done");
    drop(first_rx);

    let mut late = registry
        .subscribe(&id)
        .expect("lock")
        .expect("sender retained for a finished job");
    let status = late.borrow_and_update().clone();

    assert_eq!(status.slug(), "done");
    assert!(status.is_terminal());
    assert_eq!(status.result(), Some(&json!({"mrr": 0.5})));
}

#[test]
fn subscribe_on_an_unknown_id_yields_none() {
    let registry = Registry::new();
    assert!(
        registry
            .subscribe("deadbeefdeadbeef")
            .expect("lock")
            .is_none()
    );
}

#[test]
fn list_returns_jobs_newest_first() {
    let registry = Registry::new();
    let (first, _a) = registry.insert("index-code").expect("insert");
    let (second, _b) = registry.insert("eval").expect("insert");
    let (third, _c) = registry.insert("rebuild").expect("insert");

    let ids: Vec<String> = registry
        .list()
        .expect("list")
        .into_iter()
        .map(|v| v.job_id)
        .collect();

    assert_eq!(ids, vec![third, second, first]);
}

#[test]
fn finished_jobs_are_evicted_beyond_the_retention_window() {
    let registry = Registry::new();
    let (running, _rx) = registry.insert("index-code").expect("insert");
    registry
        .set_status(&running, JobStatus::Running)
        .expect("mark running");

    let mut finished = Vec::new();
    for _ in 0..(MAX_FINISHED + 5) {
        let (id, _rx) = registry.insert("eval").expect("insert");
        registry
            .set_status(&id, JobStatus::Done(json!(null)))
            .expect("mark done");
        finished.push(id);
    }

    let oldest = &finished[0];
    assert!(
        registry.get(oldest).expect("lock").is_none(),
        "the oldest finished job must have been evicted"
    );
    let newest = finished.last().expect("at least one finished job");
    assert!(
        registry.get(newest).expect("lock").is_some(),
        "the newest finished job must be retained"
    );
    assert!(
        registry.get(&running).expect("lock").is_some(),
        "a running job is never evicted"
    );
    let live = registry.list().expect("list").len();
    assert!(
        live <= MAX_FINISHED + 2,
        "retention must stay bounded, got {live} records"
    );
}
