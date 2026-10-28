//! The `data` payloads of the three SSE event types
//! `GET /api/v1/jobs/{id}/events` streams: [`JobEvent`] (the lifecycle
//! `queued`/`running`/`done`/`error`/`cancelled` events, named after the
//! status slug), [`ProgressEvent`] (`progress`), and [`LogEvent`] (`log`).
//! Every payload borrows from the registry value the stream just read
//! rather than cloning it. Split out of `jobs.rs` for the size ceiling.

use serde::Serialize;
use serde_json::Value;

use crate::serve::jobs::{JobError, JobStatus, Progress};

/// Emit each event type's `new` constructor. A table keeps the three
/// same-named methods from pairing while each type keeps its own fields.
macro_rules! event_ctors {
    ($(
        $(#[$type_meta:meta])*
        struct $Type:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $fty:ty
            ),* $(,)?
        }
        $(#[$new_meta:meta])*
        fn new($($arg:ident : $aty:ty),* $(,)?) {
            $($init_field:ident : $init:expr),* $(,)?
        }
    )+) => {
        $(
            $(#[$type_meta])*
            #[derive(Serialize)]
            pub struct $Type<'a> {
                $(
                    $(#[$field_meta])*
                    pub $field: $fty,
                )*
            }

            impl<'a> $Type<'a> {
                $(#[$new_meta])*
                pub fn new($($arg: $aty),*) -> Self {
                    Self {
                        $($init_field: $init),*
                    }
                }
            }
        )+
    };
}

event_ctors! {
    /// The `data` of one SSE lifecycle event. Timestamps are omitted; poll
    /// `GET /jobs/{id}` for the full record.
    struct JobEvent {
        /// The job's 16-hex id.
        job_id: &'a str,
        /// `"queued" | "running" | "done" | "error" | "cancelled"` (also the
        /// SSE event name).
        status: &'static str,
        /// The success payload on a `done` event, else `null`.
        result: Option<&'a Value>,
        /// The error object on an `error` event, else `null`.
        error: Option<&'a JobError>,
    }
    /// Build the event payload for `status` on job `job_id`.
    fn new(job_id: &'a str, status: &'a JobStatus) {
        job_id: job_id,
        status: status.slug(),
        result: status.result(),
        error: status.error(),
    }

    /// The `data` of one SSE `progress` event — a second, additive event type
    /// alongside the lifecycle events: a client that only handles those event
    /// names sees byte-identical behavior to before this type existed.
    struct ProgressEvent {
        /// The job's 16-hex id.
        job_id: &'a str,
        /// Units completed so far.
        done: u64,
        /// Total units this run will process.
        total: u64,
        /// What `done`/`total` count.
        unit: &'a str,
    }
    /// Build the event payload for `progress` on job `job_id`.
    fn new(job_id: &'a str, progress: &'a Progress) {
        job_id: job_id,
        done: progress.done,
        total: progress.total,
        unit: &progress.unit,
    }

    /// The `data` of one SSE `log` event — one per `ProgressSink::on_log` line,
    /// best-effort (a subscriber more than `LOG_CHANNEL_CAP` lines behind
    /// lags and loses the oldest; `JobView.log_tail` is the durable record).
    struct LogEvent {
        /// The job's 16-hex id.
        job_id: &'a str,
        /// The log line, verbatim.
        line: &'a str,
    }
    /// Build the event payload for one log `line` on job `job_id`.
    fn new(job_id: &'a str, line: &'a str) {
        job_id: job_id,
        line: line,
    }
}
