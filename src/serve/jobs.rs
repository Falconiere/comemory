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
//! restart forgets every job.
//!
//! [`Progress`] (`JobView.progress`/`log_tail`, and the additive SSE
//! `progress` event `serve::routes::jobs` streams) is a second, parallel
//! channel per job — deliberately NOT a [`JobStatus`] variant, so today's
//! `status` SSE payload stays byte-identical for a client that ignores the
//! new event type. Log lines are a third channel (`broadcast`, the SSE
//! `log` event), and cancellation (`POST /jobs/{id}/cancel`) is a per-job
//! flag a cooperating core polls at its next boundary — see
//! [`Registry::cancel`] and `api::index_code::ProgressSink::is_cancelled`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::{broadcast, watch};

use crate::prelude::*;
use crate::serve::envelope;

/// The SSE event payload types (`status`, `progress`, `log`).
pub mod events;
/// The job table: records, `watch` senders, and the finished-job eviction.
pub mod registry;
/// [`worker::spawn_job`] — the generic job-execution entry point.
pub mod worker;

pub use events::{JobEvent, LogEvent, ProgressEvent};
pub use registry::Registry;
pub use worker::{spawn_job, spawn_job_for, spawn_job_with_id};

/// Capacity of each job's log `broadcast` channel. A subscriber slower
/// than this many lines behind lags and loses the oldest — acceptable for
/// a live stream; `JobView.log_tail` is the durable form.
pub const LOG_CHANNEL_CAP: usize = 256;

/// A job's identifier: 8 random bytes hex-encoded (16 lowercase-hex chars),
/// drawn from the same `/dev/urandom` source as the session token. A bare
/// `String` rather than a newtype, matching how the crate already carries
/// id-shaped strings (`memory::id`).
pub type JobId = String;

/// Bound on [`Job::log_tail`] — only the last N log lines are kept, oldest
/// dropped first, so a long-running job cannot grow the registry's memory
/// use without limit.
pub const LOG_TAIL_CAP: usize = 20;

/// Where a job is in its lifecycle, and what it produced. `Clone` because
/// it is the `watch` channel's value type; serializes as its
/// [`JobStatus::slug`] string (`"queued"`, `"running"`, `"done"`,
/// `"error"`) so the `status` field of a job payload is a plain slug and the
/// payload/error ride alongside it in their own fields. Deliberately never
/// gains a payload-carrying variant for progress — see [`Progress`] and the
/// module doc.
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
    /// Stopped at the caller's request (`POST /jobs/{id}/cancel`) — either
    /// before its body ever ran (cancelled while queued) or at the first
    /// boundary a cooperating core checked (`Error::Cancelled` unwound its
    /// transaction, so nothing was half-written).
    Cancelled,
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
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether this is a final status — no further transition follows
    /// (`Registry::set_status` ignores any attempt to leave one), so the
    /// SSE handler ends its stream after emitting it.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done(_) | Self::Error(_) | Self::Cancelled)
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

/// A job's fractional progress, mirrored into [`JobView::progress`] and
/// streamed as the SSE `progress` event's payload
/// (`GET /api/v1/jobs/{id}/events`). Deliberately not a [`JobStatus`]
/// variant — see that enum's doc and the module doc — so a client that
/// ignores the `progress` event type sees today's `status` payload
/// unchanged.
#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    /// Units completed so far.
    pub done: u64,
    /// Total units this run will process.
    pub total: u64,
    /// What `done`/`total` count (e.g. `"files"`).
    pub unit: String,
}

/// One job's record in the [`Registry`]. Holds two retained
/// `watch::Sender`s (status and progress), which is why neither channel
/// closes on its own and a late SSE subscriber can still replay the
/// terminal status.
pub struct Job {
    /// This job's 16-hex id.
    pub id: JobId,
    /// The CLI subcommand name this job runs (`"index-code"`, …).
    pub command: String,
    /// The repo label this job works on, when it has one — what
    /// [`Registry::active_for`] matches so a second `index-code` for the
    /// same repo can be refused with `409 index_running`.
    pub repo: Option<String>,
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
    /// Retained sender for the SSE `progress` event — see the struct doc.
    pub(crate) progress_tx: watch::Sender<Option<Progress>>,
    /// Current progress snapshot, or `None` before the first report.
    pub(crate) progress: Option<Progress>,
    /// Bounded ring buffer of the job's most recent log lines (newest
    /// last) — see [`LOG_TAIL_CAP`].
    pub(crate) log_tail: VecDeque<String>,
    /// Live log-line fan-out for the SSE `log` event; retained like the
    /// `watch` senders so a subscriber can attach at any time.
    pub(crate) log_tx: broadcast::Sender<String>,
    /// The cooperative cancel flag — set by [`Registry::cancel`], read by
    /// the worker before the body runs and by a cooperating core's
    /// `ProgressSink::is_cancelled` at each boundary.
    pub(crate) cancel: Arc<AtomicBool>,
}

impl Job {
    /// A freshly accepted job in [`JobStatus::Queued`], with no progress
    /// reported yet.
    pub(crate) fn new(
        id: JobId,
        command: String,
        repo: Option<String>,
        started_at: String,
        seq: u64,
        tx: watch::Sender<JobStatus>,
        progress_tx: watch::Sender<Option<Progress>>,
    ) -> Self {
        let (log_tx, _log_rx) = broadcast::channel(LOG_CHANNEL_CAP);
        Self {
            id,
            command,
            repo,
            status: JobStatus::Queued,
            started_at,
            finished_at: None,
            seq,
            tx,
            progress_tx,
            progress: None,
            log_tail: VecDeque::new(),
            log_tx,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Append `line` to the bounded log tail, dropping the oldest entry
    /// once [`LOG_TAIL_CAP`] is reached — unbounded growth on a long index
    /// would leak memory for the life of the server — and fan it out to
    /// any live `log` subscriber (a send with no receiver is not an error:
    /// the tail is the durable record).
    pub(crate) fn push_log(&mut self, line: String) {
        if self.log_tail.len() >= LOG_TAIL_CAP {
            self.log_tail.pop_front();
        }
        let _ = self.log_tx.send(line.clone());
        self.log_tail.push_back(line);
    }

    /// Whether a cancel has been requested for this job.
    pub(crate) fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The serializable snapshot handed to `GET /jobs` / `GET /jobs/{id}`.
    pub fn view(&self) -> JobView {
        JobView {
            job_id: self.id.clone(),
            command: self.command.clone(),
            repo: self.repo.clone(),
            status: self.status.slug(),
            started_at: self.started_at.clone(),
            finished_at: self.finished_at.clone(),
            result: self.status.result().cloned(),
            error: self.status.error().cloned(),
            progress: self.progress.clone(),
            log_tail: self.log_tail.iter().cloned().collect(),
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
    /// The repo label this job works on, or `null`.
    pub repo: Option<String>,
    /// `"queued" | "running" | "done" | "error" | "cancelled"`.
    pub status: &'static str,
    /// Acceptance timestamp (ISO-8601 UTC).
    pub started_at: String,
    /// Terminal-status timestamp, or `null`.
    pub finished_at: Option<String>,
    /// The payload the synchronous form would have returned, or `null`.
    pub result: Option<Value>,
    /// The envelope-shaped error object, or `null`.
    pub error: Option<JobError>,
    /// The most recent progress report, or `null` before the first one.
    pub progress: Option<Progress>,
    /// The job's most recent log lines (newest last), bounded to
    /// [`LOG_TAIL_CAP`] entries.
    pub log_tail: Vec<String>,
}

#[cfg(test)]
#[path = "tests/jobs.rs"]
mod tests;
