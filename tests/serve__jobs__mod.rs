//! Mirror test for `src/serve/jobs/mod.rs` — the job lifecycle types.
//!
//! Proves the wire shapes the `/api/v1/jobs*` routes and the SSE stream
//! depend on: `JobStatus` serializes as a bare slug string, a failed job's
//! `{code, message}` comes from the envelope's one `Error → code` map, and
//! `JobView` / `JobEvent` carry `result`/`error` in their own fields.

use comemory::errors::Error;
use comemory::serve::jobs::{JobError, JobEvent, JobStatus};
use serde_json::json;

#[test]
fn status_slugs_and_terminality_cover_the_lifecycle() {
    assert_eq!(JobStatus::Queued.slug(), "queued");
    assert_eq!(JobStatus::Running.slug(), "running");
    assert_eq!(JobStatus::Done(json!({"ok": 1})).slug(), "done");
    assert_eq!(
        JobStatus::Error(JobError::internal("boom".into())).slug(),
        "error"
    );

    assert!(!JobStatus::Queued.is_terminal());
    assert!(!JobStatus::Running.is_terminal());
    assert!(JobStatus::Done(json!(null)).is_terminal());
    assert!(JobStatus::Error(JobError::internal("boom".into())).is_terminal());
}

#[test]
fn status_serializes_as_a_bare_slug_string() {
    let done = JobStatus::Done(json!({"removed": 3}));
    assert_eq!(
        serde_json::to_value(&done).expect("serialize"),
        json!("done")
    );
    assert_eq!(
        serde_json::to_value(JobStatus::Queued).expect("serialize"),
        json!("queued")
    );
}

#[test]
fn result_and_error_accessors_are_status_exclusive() {
    let done = JobStatus::Done(json!({"removed": 3}));
    assert_eq!(done.result(), Some(&json!({"removed": 3})));
    assert!(done.error().is_none());

    let failed = JobStatus::Error(JobError::internal("boom".into()));
    assert!(failed.result().is_none());
    assert_eq!(failed.error().map(|e| e.code.as_str()), Some("internal"));
}

#[test]
fn job_error_reuses_the_envelope_error_code_map() {
    let not_found = JobError::from_error(&Error::NotFound("memory not found: ab12cd34".into()));
    assert_eq!(not_found.code, "not_found");
    assert!(
        not_found.message.contains("ab12cd34"),
        "message: {}",
        not_found.message
    );

    let forbidden = JobError::from_error(&Error::Forbidden("outside every allowed root".into()));
    assert_eq!(forbidden.code, "forbidden");

    let bad_request = JobError::from_error(&Error::BadRequest("no such path".into()));
    assert_eq!(bad_request.code, "bad_request");
}

#[test]
fn job_event_carries_the_payload_beside_the_status_slug() {
    let done = JobStatus::Done(json!({"symbols": 12}));
    let event = serde_json::to_value(JobEvent::new("0011223344556677", &done)).expect("serialize");
    assert_eq!(
        event,
        json!({
            "job_id": "0011223344556677",
            "status": "done",
            "result": {"symbols": 12},
            "error": null,
        })
    );

    let failed = JobStatus::Error(JobError::from_error(&Error::NotFound("gone".into())));
    let event =
        serde_json::to_value(JobEvent::new("0011223344556677", &failed)).expect("serialize");
    assert_eq!(event["status"], json!("error"));
    assert_eq!(event["result"], json!(null));
    assert_eq!(event["error"]["code"], json!("not_found"));
}
