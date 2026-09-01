//! The `data` payloads of the three SSE event types
//! `GET /api/v1/jobs/{id}/events` streams: [`JobEvent`] (the lifecycle
//! `queued`/`running`/`done`/`error`/`cancelled` events, named after the
//! status slug), [`ProgressEvent`] (`progress`), and [`LogEvent`] (`log`).
//! Every payload borrows from the registry value the stream just read
//! rather than cloning it. Split out of `jobs.rs` for the size ceiling.

use serde::Serialize;
use serde_json::Value;

use crate::serve::jobs::{JobError, JobStatus, Progress};

/// The `data` of one SSE lifecycle event. Timestamps are omitted; poll
/// `GET /jobs/{id}` for the full record.
#[derive(Serialize)]
pub struct JobEvent<'a> {
    /// The job's 16-hex id.
    pub job_id: &'a str,
    /// `"queued" | "running" | "done" | "error" | "cancelled"` (also the
    /// SSE event name).
    pub status: &'static str,
    /// The success payload on a `done` event, else `null`.
    pub result: Option<&'a Value>,
    /// The error object on an `error` event, else `null`.
    pub error: Option<&'a JobError>,
}

impl<'a> JobEvent<'a> {
    /// Build the event payload for `status` on job `job_id`.
    pub fn new(job_id: &'a str, status: &'a JobStatus) -> Self {
        Self {
            job_id,
            status: status.slug(),
            result: status.result(),
            error: status.error(),
        }
    }
}

/// The `data` of one SSE `progress` event — a second, additive event type
/// alongside the lifecycle events: a client that only handles those event
/// names sees byte-identical behavior to before this type existed.
#[derive(Serialize)]
pub struct ProgressEvent<'a> {
    /// The job's 16-hex id.
    pub job_id: &'a str,
    /// Units completed so far.
    pub done: u64,
    /// Total units this run will process.
    pub total: u64,
    /// What `done`/`total` count.
    pub unit: &'a str,
}

impl<'a> ProgressEvent<'a> {
    /// Build the event payload for `progress` on job `job_id`.
    pub fn new(job_id: &'a str, progress: &'a Progress) -> Self {
        Self {
            job_id,
            done: progress.done,
            total: progress.total,
            unit: &progress.unit,
        }
    }
}

/// The `data` of one SSE `log` event — one per `ProgressSink::on_log` line,
/// best-effort (a subscriber more than `LOG_CHANNEL_CAP` lines behind
/// lags and loses the oldest; `JobView.log_tail` is the durable record).
#[derive(Serialize)]
pub struct LogEvent<'a> {
    /// The job's 16-hex id.
    pub job_id: &'a str,
    /// The log line, verbatim.
    pub line: &'a str,
}

impl<'a> LogEvent<'a> {
    /// Build the event payload for one log `line` on job `job_id`.
    pub fn new(job_id: &'a str, line: &'a str) -> Self {
        Self { job_id, line }
    }
}
