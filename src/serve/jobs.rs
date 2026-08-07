//! In-process background jobs for `/api/v1`: the long commands answer
//! `202 Accepted` + a job id and finish on a blocking-pool thread while the
//! request path stays free. Lifecycle `Queued → Running → Done | Error`; a
//! mutating job waits FIFO for the single write permit and holds it for its
//! whole run, a read-class job (`eval`) never takes it.
//!
//! These lifecycle types plus [`Registry`] (job records + their retained
//! `watch::Sender`s) and [`worker::spawn_job`] (run-this-closure-as-a-job).
//! Nothing here knows about `api::` or `Ctx` — the caller builds the
//! closure, this layer only tracks it. Not persisted (Non-Goal 4): a
//! restart forgets every job, and there is no cancellation.

use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;

use crate::prelude::*;
use crate::serve::envelope;

/// The job table: records, `watch` senders, and the finished-job eviction.
pub mod registry;
/// [`worker::spawn_job`] — the generic job-execution entry point.
pub mod worker;

pub use registry::Registry;
pub use worker::spawn_job;

/// A job's identifier: 8 random bytes hex-encoded (16 lowercase-hex chars),
/// drawn from the same `/dev/urandom` source as the session token. A bare
/// `String` rather than a newtype, matching how the crate already carries
/// id-shaped strings (`memory::id`).
pub type JobId = String;

/// Where a job is in its lifecycle, and what it produced. `Clone` because
/// it is the `watch` channel's value type; serializes as its
/// [`JobStatus::slug`] string (`"queued"`, `"running"`, `"done"`,
/// `"error"`) so the `status` field of a job payload is a plain slug and the
/// payload/error ride alongside it in their own fields.
#[derive(Clone, Debug)]
pub enum JobStatus {
    /// Registered, not started — waiting for the write permit (mutating
    /// jobs) or for the blocking pool.
    Queued,
    /// The command's closure is running on the blocking pool.
    Running,
    /// Finished successfully, carrying the payload the synchronous form of
    /// the command would have returned.
    Done(Value),
    /// Finished with an error, carrying the envelope's `{code, message}`.
    Error(JobError),
}

impl JobStatus {
    /// The machine-readable status slug used in JSON and as the SSE event
    /// name.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done(_) => "done",
            Self::Error(_) => "error",
        }
    }

    /// Whether this is a final status — no further transition follows, so
    /// the SSE handler ends its stream after emitting it.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done(_) | Self::Error(_))
    }

    /// The success payload, when finished successfully.
    pub fn result(&self) -> Option<&Value> {
        match self {
            Self::Done(v) => Some(v),
            _ => None,
        }
    }

    /// The failure object, when finished with an error.
    pub fn error(&self) -> Option<&JobError> {
        match self {
            Self::Error(e) => Some(e),
            _ => None,
        }
    }
}

impl Serialize for JobStatus {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.slug())
    }
}

/// The `{code, message}` error object a failed job carries — the same shape
/// (and the same `code` slugs) the error envelope uses, so a job failure and
/// a synchronous failure describe themselves identically.
#[derive(Clone, Debug, Serialize)]
pub struct JobError {
    /// Machine-readable slug from [`envelope::status_and_code`].
    pub code: String,
    /// Human-readable message (the `Error`'s `Display`).
    pub message: String,
}

impl JobError {
    /// Derive the job error from a crate [`Error`], reusing the envelope's
    /// one `Error → code` mapping (Binding Rule 1) — the HTTP status is
    /// irrelevant here since the job payload itself is served with `200`.
    pub fn from_error(e: &Error) -> Self {
        let (_status, code) = envelope::status_and_code(e);
        Self {
            code: code.to_string(),
            message: e.to_string(),
        }
    }

    /// An infrastructure-level failure with no crate `Error` behind it (a
    /// panicked blocking task, a closed semaphore).
    pub fn internal(message: String) -> Self {
        Self {
            code: "internal".to_string(),
            message,
        }
    }
}

/// One job's record in the [`Registry`]. Holds the retained
/// `watch::Sender`, which is why the channel never closes on its own and a
/// late SSE subscriber can still replay the terminal status.
pub struct Job {
    /// This job's 16-hex id.
    pub id: JobId,
    /// The CLI subcommand name this job runs (`"index-code"`, …).
    pub command: String,
    /// Current lifecycle status, mirrored into the `watch` channel.
    pub status: JobStatus,
    /// When the job was accepted (ISO-8601 UTC, `memory_row::iso_format`).
    pub started_at: String,
    /// When the job reached a terminal status; `None` while it is
    /// queued/running.
    pub finished_at: Option<String>,
    /// Monotonic insertion counter: the ordering key for "newest first"
    /// listing and oldest-first eviction (ISO timestamps can tie).
    pub(crate) seq: u64,
    /// Retained sender — see the struct doc.
    pub(crate) tx: watch::Sender<JobStatus>,
}

impl Job {
    /// A freshly accepted job in [`JobStatus::Queued`].
    pub(crate) fn new(
        id: JobId,
        command: String,
        started_at: String,
        seq: u64,
        tx: watch::Sender<JobStatus>,
    ) -> Self {
        Self {
            id,
            command,
            status: JobStatus::Queued,
            started_at,
            finished_at: None,
            seq,
            tx,
        }
    }

    /// The serializable snapshot handed to `GET /jobs` / `GET /jobs/{id}`.
    pub fn view(&self) -> JobView {
        JobView {
            job_id: self.id.clone(),
            command: self.command.clone(),
            status: self.status.slug(),
            started_at: self.started_at.clone(),
            finished_at: self.finished_at.clone(),
            result: self.status.result().cloned(),
            error: self.status.error().cloned(),
        }
    }
}

/// The JSON body of `GET /api/v1/jobs/{id}` (and one item of the
/// `GET /api/v1/jobs` page).
#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    /// The job's 16-hex id.
    pub job_id: String,
    /// The CLI subcommand this job runs.
    pub command: String,
    /// `"queued" | "running" | "done" | "error"`.
    pub status: &'static str,
    /// Acceptance timestamp (ISO-8601 UTC).
    pub started_at: String,
    /// Terminal-status timestamp, or `null`.
    pub finished_at: Option<String>,
    /// The payload the synchronous form would have returned, or `null`.
    pub result: Option<Value>,
    /// The envelope-shaped error object, or `null`.
    pub error: Option<JobError>,
}

/// The `data` of one SSE lifecycle event on
/// `GET /api/v1/jobs/{id}/events`. Borrowed (not cloned) from the status
/// the stream just read — timestamps are omitted; poll `GET /jobs/{id}` for
/// the full record.
#[derive(Serialize)]
pub struct JobEvent<'a> {
    /// The job's 16-hex id.
    pub job_id: &'a str,
    /// `"queued" | "running" | "done" | "error"` (also the SSE event name).
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

#[cfg(test)]
#[path = "tests/jobs.rs"]
mod tests;
