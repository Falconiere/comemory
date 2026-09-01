//! The in-process job table: one [`Job`] record per accepted job, each with
//! a retained `watch::Sender<JobStatus>` (and a second, progress-only
//! `watch::Sender<Option<Progress>>`) so a status stream can attach (or
//! re-attach) at any time and immediately read the current value.
//!
//! Memory is bounded two ways: finished jobs beyond [`MAX_FINISHED`] are
//! evicted on every insertion (queued and running jobs are never evicted),
//! and each job's own [`crate::serve::jobs::LOG_TAIL_CAP`]-bounded log tail
//! is capped independent of the job's own runtime. The `Mutex` is held
//! only for the short synchronous map mutations — never across an
//! `.await`.
//!
//! Two transitions are check-and-act pairs that MUST happen under one lock
//! hold, and do: [`Registry::try_start`] (still `Queued` and not cancelled
//! → `Running`) and [`Registry::insert_for_unless_active`] (no live job for
//! this repo → insert). Terminal statuses are sticky — [`Registry::set_status`]
//! never moves a job out of `Done`/`Error`/`Cancelled` — so a late
//! transition from a worker can never overwrite a cancel that beat it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use time::OffsetDateTime;
use tokio::sync::{broadcast, watch};

use crate::prelude::*;
use crate::serve::jobs::{Job, JobId, JobStatus, JobView, Progress};
use crate::serve::security;
use crate::store::memory_row;

/// What [`Registry::cancel`] did for a job that had not yet finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The job was still queued: it is now `Cancelled` and its body will
    /// never run.
    Cancelled,
    /// The job was running: the cooperative flag is set, and the job lands
    /// in `Cancelled` at its next boundary (or `Done` if it finishes
    /// first).
    Requested,
}

/// A freshly registered job, as [`Registry::insert`] and
/// [`Registry::insert_for_unless_active`] hand it back.
pub struct Accepted {
    /// The new job's id (the `202` body's `job_id`).
    pub id: JobId,
    /// A status receiver already positioned on [`JobStatus::Queued`].
    pub status: watch::Receiver<JobStatus>,
    /// The job's own cancel flag — the same `Arc` its record holds. The
    /// worker keeps this clone so its pre-body cancel check never depends
    /// on the record still being in the table: a job cancelled while
    /// queued is terminal, and [`evict_finished`] may drop it before its
    /// worker wakes (see that function's doc).
    pub cancel: Arc<AtomicBool>,
}

/// Random bytes behind a job id — 8 bytes, rendered as 16 lowercase-hex
/// chars (the session token's source, at a shorter width).
const JOB_ID_BYTES: usize = 8;

/// How many finished jobs are retained. Older ones (and their senders) are
/// dropped on the next insertion, so a long-lived server's job table stays
/// bounded while a late SSE subscriber still finds recent terminal states.
pub const MAX_FINISHED: usize = 100;

/// The job table. Constructed once in `AppState`, shared as an `Arc`.
#[derive(Default)]
pub struct Registry {
    jobs: Mutex<HashMap<JobId, Job>>,
    next_seq: AtomicU64,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new job for `command` as [`JobStatus::Queued`], returning
    /// its [`Accepted`] handle. Also runs the finished-job eviction sweep.
    pub fn insert(&self, command: &str) -> Result<Accepted> {
        let (job, accepted) = self.new_job(command, None)?;
        let mut jobs = self.lock()?;
        jobs.insert(accepted.id.clone(), job);
        evict_finished(&mut jobs);
        Ok(accepted)
    }

    /// [`Registry::insert`] with a repo label, refusing — under the SAME
    /// lock hold the insertion takes — when `(command, repo)` already has a
    /// queued or running job: `Err(Error::IndexRunning { repo, job_id })`,
    /// the `409 index_running` the routes report, naming the live job. A
    /// separate [`Registry::refuse_if_active`] pre-check followed by an
    /// insert would let two concurrent requests both pass the check and
    /// both queue; this is the check that counts.
    pub fn insert_for_unless_active(&self, command: &str, repo: &str) -> Result<Accepted> {
        let (job, accepted) = self.new_job(command, Some(repo))?;
        let mut jobs = self.lock()?;
        refuse_active(&jobs, command, repo)?;
        jobs.insert(accepted.id.clone(), job);
        evict_finished(&mut jobs);
        Ok(accepted)
    }

    /// Mint a fresh `Queued` record plus its [`Accepted`] handle; the caller
    /// inserts the record under the lock.
    fn new_job(&self, command: &str, repo: Option<&str>) -> Result<(Job, Accepted)> {
        let id = security::random_hex(JOB_ID_BYTES)?;
        let started_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
        let (tx, status) = watch::channel(JobStatus::Queued);
        let (progress_tx, _progress_rx) = watch::channel(None);
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let job = Job::new(
            id.clone(),
            command.to_string(),
            repo.map(str::to_string),
            started_at,
            seq,
            tx,
            progress_tx,
        );
        let cancel = Arc::clone(&job.cancel);
        Ok((job, Accepted { id, status, cancel }))
    }

    /// The id of a queued or running job for `(command, repo)`, if any —
    /// what `GET /repos` overlays as `indexing`. Lowest `seq` wins when
    /// several are live (they cannot be, in practice:
    /// [`Registry::insert_for_unless_active`] refuses the second).
    pub fn active_for(&self, command: &str, repo: &str) -> Result<Option<JobId>> {
        let jobs = self.lock()?;
        Ok(active_in(&jobs, command, repo))
    }

    /// `Err(Error::IndexRunning)` when `(command, repo)` already has a
    /// queued or running job — the fast-path pre-check a route runs before
    /// the rest of its planning, so a doomed request fails before any
    /// containment work. Not the gate itself: that is
    /// [`Registry::insert_for_unless_active`], which repeats this check
    /// atomically at insertion.
    pub fn refuse_if_active(&self, command: &str, repo: &str) -> Result<()> {
        let jobs = self.lock()?;
        refuse_active(&jobs, command, repo)
    }

    /// Request cancellation of job `id`. A queued job becomes `Cancelled`
    /// right away (its body never runs — [`Registry::try_start`] refuses
    /// it); a running job only has its flag set and stops at its next
    /// cooperative boundary. `NotFound` for an unknown id; `BadRequest` for
    /// a job that already reached a terminal status.
    pub fn cancel(&self, id: &str) -> Result<CancelOutcome> {
        let mut jobs = self.lock()?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("job not found: {id}")))?;
        if job.status.is_terminal() {
            return Err(Error::BadRequest(format!(
                "job {id} already finished ({})",
                job.status.slug()
            )));
        }
        job.cancel.store(true, Ordering::Relaxed);
        if matches!(job.status, JobStatus::Queued) {
            job.finished_at = Some(memory_row::iso_format(OffsetDateTime::now_utc())?);
            job.status = JobStatus::Cancelled;
            job.tx.send_replace(JobStatus::Cancelled);
            return Ok(CancelOutcome::Cancelled);
        }
        Ok(CancelOutcome::Requested)
    }

    /// Atomically take job `id` from `Queued` to `Running`, returning
    /// whether that happened. `false` — and no change — when the job is
    /// no longer `Queued` (cancelled while queued, so already terminal),
    /// carries the cancel flag, or is unknown (never registered, or
    /// evicted); in every one of those cases the worker must not run the
    /// body. One lock hold, so a [`Registry::cancel`] either lands before
    /// this (and is honored) or after it (and only sets the flag on a job
    /// that really is running) — never in between.
    pub fn try_start(&self, id: &str) -> bool {
        let Ok(mut jobs) = self.lock() else {
            return false;
        };
        let Some(job) = jobs.get_mut(id) else {
            return false;
        };
        if !matches!(job.status, JobStatus::Queued) || job.cancel_requested() {
            return false;
        }
        job.status = JobStatus::Running;
        job.tx.send_replace(JobStatus::Running);
        true
    }

    /// Whether a cancel has been requested for job `id`. An unknown id reads
    /// as `false` — the flag is advisory, and a missing record is already a
    /// stronger signal the worker handles on its own.
    pub fn is_cancelled(&self, id: &str) -> bool {
        self.lock()
            .ok()
            .and_then(|jobs| jobs.get(id).map(Job::cancel_requested))
            .unwrap_or(false)
    }

    /// A fresh receiver on job `id`'s log-line channel — the SSE `log`
    /// event's source. Lines pushed before subscribing are not replayed
    /// (`JobView.log_tail` carries those).
    pub fn subscribe_log(&self, id: &str) -> Result<Option<broadcast::Receiver<String>>> {
        Ok(self.lock()?.get(id).map(|job| job.log_tx.subscribe()))
    }

    /// Record a progress report for job `id`: update the stored snapshot
    /// (`JobView::progress`) and publish it on the job's progress `watch`
    /// channel (the SSE `progress` event). `NotFound` when the id is
    /// unknown — a [`crate::api::index_code::ProgressSink`] implementation
    /// treats that as best-effort and only warns; see that trait's doc.
    pub fn set_progress(&self, id: &str, progress: Progress) -> Result<()> {
        let mut jobs = self.lock()?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("job not found: {id}")))?;
        job.progress = Some(progress.clone());
        job.progress_tx.send_replace(Some(progress));
        Ok(())
    }

    /// Append one log line to job `id`'s bounded tail (`JobView::log_tail`).
    /// `NotFound` when the id is unknown, handled the same best-effort way
    /// as [`Registry::set_progress`].
    pub fn push_log(&self, id: &str, line: String) -> Result<()> {
        let mut jobs = self.lock()?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("job not found: {id}")))?;
        job.push_log(line);
        Ok(())
    }

    /// Record a job's transition: update the stored record (stamping
    /// `finished_at` on a terminal status) and publish the new value on the
    /// job's `watch` channel. `NotFound` when the id is unknown.
    ///
    /// Terminal statuses are sticky: once a job is `Done`, `Error`, or
    /// `Cancelled`, every further transition — terminal or not — is
    /// ignored (logged at debug, `Ok(())`). That is what keeps a cancel that
    /// landed while the job was queued from being overwritten by the
    /// worker's own `Running`/`Done` afterwards.
    pub fn set_status(&self, id: &str, status: JobStatus) -> Result<()> {
        let finished_at = if status.is_terminal() {
            Some(memory_row::iso_format(OffsetDateTime::now_utc())?)
        } else {
            None
        };
        let mut jobs = self.lock()?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("job not found: {id}")))?;
        if job.status.is_terminal() {
            tracing::debug!(
                job_id = id,
                from = job.status.slug(),
                to = status.slug(),
                "job status transition out of a terminal state ignored",
            );
            return Ok(());
        }
        job.finished_at = finished_at;
        job.status = status.clone();
        // `send_replace` (not `send`) because a job with no subscriber is
        // normal: the value must still be stored for a later subscriber.
        job.tx.send_replace(status);
        Ok(())
    }

    /// One job's snapshot, or `None` when the id is unknown (never
    /// registered, or evicted).
    pub fn get(&self, id: &str) -> Result<Option<JobView>> {
        Ok(self.lock()?.get(id).map(Job::view))
    }

    /// A fresh receiver on a job's status channel. Because the sender is
    /// retained, `borrow_and_update` on the returned receiver immediately
    /// yields the current status — including a terminal one for a job that
    /// finished before the subscriber attached (AC-8).
    pub fn subscribe(&self, id: &str) -> Result<Option<watch::Receiver<JobStatus>>> {
        Ok(self.lock()?.get(id).map(|job| job.tx.subscribe()))
    }

    /// A fresh receiver on job `id`'s progress channel — the SSE
    /// `progress` event's source, mirroring [`Registry::subscribe`].
    pub fn subscribe_progress(
        &self,
        id: &str,
    ) -> Result<Option<watch::Receiver<Option<Progress>>>> {
        Ok(self.lock()?.get(id).map(|job| job.progress_tx.subscribe()))
    }

    /// Every retained job, newest first.
    pub fn list(&self) -> Result<Vec<JobView>> {
        let jobs = self.lock()?;
        let mut records: Vec<&Job> = jobs.values().collect();
        records.sort_by_key(|job| std::cmp::Reverse(job.seq));
        Ok(records.into_iter().map(Job::view).collect())
    }

    /// Lock the job map, mapping poisoning (a panic elsewhere while holding
    /// the guard) to an internal error rather than propagating the panic.
    fn lock(&self) -> Result<MutexGuard<'_, HashMap<JobId, Job>>> {
        self.jobs
            .lock()
            .map_err(|_| Error::Other("serve: job registry lock poisoned".into()))
    }
}

/// The lowest-`seq` queued or running job for `(command, repo)` in an
/// already-locked table — the one query behind [`Registry::active_for`],
/// [`Registry::refuse_if_active`], and [`Registry::insert_for_unless_active`].
fn active_in(jobs: &HashMap<JobId, Job>, command: &str, repo: &str) -> Option<JobId> {
    jobs.values()
        .filter(|job| {
            job.command == command && job.repo.as_deref() == Some(repo) && !job.status.is_terminal()
        })
        .min_by_key(|job| job.seq)
        .map(|job| job.id.clone())
}

/// [`active_in`] as the `409 index_running` refusal — the one place that
/// error is built, so the route pre-check and the atomic insert gate
/// report the identical `{repo, job_id}`.
fn refuse_active(jobs: &HashMap<JobId, Job>, command: &str, repo: &str) -> Result<()> {
    match active_in(jobs, command, repo) {
        Some(job_id) => Err(Error::IndexRunning {
            repo: repo.to_string(),
            job_id,
        }),
        None => Ok(()),
    }
}

/// Drop the oldest finished jobs (by insertion order) until at most
/// [`MAX_FINISHED`] remain. Queued and running jobs are never counted and
/// never evicted — losing one would strand its worker's status updates.
///
/// A job cancelled while queued is terminal and so IS evictable, possibly
/// before its worker task wakes from the write-permit wait. That is safe
/// without tracking the worker here: the worker holds its own clone of the
/// job's cancel flag ([`Accepted::cancel`]), and [`Registry::try_start`]
/// answers `false` for an unknown id — so an evicted, cancelled job's body
/// never runs, whether the worker consults the flag or the table. Keeping
/// such a record until its worker acknowledged the cancel would need a
/// second worker→registry call on every exit path for no observable gain.
fn evict_finished(jobs: &mut HashMap<JobId, Job>) {
    let mut finished: Vec<(u64, JobId)> = jobs
        .values()
        .filter(|job| job.status.is_terminal())
        .map(|job| (job.seq, job.id.clone()))
        .collect();
    if finished.len() <= MAX_FINISHED {
        return;
    }
    finished.sort_by_key(|(seq, _)| *seq);
    let excess = finished.len() - MAX_FINISHED;
    for (_seq, id) in finished.into_iter().take(excess) {
        jobs.remove(&id);
    }
}

#[cfg(test)]
#[path = "tests/registry.rs"]
mod tests;
