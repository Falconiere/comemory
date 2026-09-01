//! [`spawn_job`] — the one place a background job is started. Job-creating
//! routes hand it a closure (typically `api::<cmd>::run` over a
//! `Ctx::lazy`, i.e. the job's **own** connection) and get back a job id to
//! put in their `202 Accepted` body; everything after that is this module's
//! bookkeeping. [`spawn_job_with_id`] is the same thing for a caller whose
//! closure needs its own job id — `POST /api/v1/code/index` uses it to
//! build a [`RegistryProgressSink`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use tokio::sync::Semaphore;

use crate::api::index_code::ProgressSink;
use crate::prelude::*;
use crate::serve::jobs::registry::Accepted;
use crate::serve::jobs::{JobError, JobId, JobStatus, Progress, Registry};

/// Spawn `body` as a background job named `command`: register it
/// [`JobStatus::Queued`], then run it to completion on the blocking pool,
/// publishing every transition through `registry`. Returns as soon as the
/// job is registered — the work continues afterwards.
///
/// A `mutating` job **awaits** the single write permit (§Concurrency) and
/// holds it until `body` has finished, so job-vs-job ordering is FIFO on
/// the semaphore's own wait queue; unlike a synchronous mutating request
/// (`routes::guard_mutating`, which `try_acquire`s and fails fast with
/// `503 busy`), a job waits its turn. A non-mutating, read-class job
/// (`eval`) never touches the permit and starts immediately.
pub fn spawn_job<F>(
    registry: &Arc<Registry>,
    write_permit: Arc<Semaphore>,
    command: &str,
    mutating: bool,
    body: F,
) -> Result<JobId>
where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let accepted = registry.insert(command)?;
    let id = accepted.id.clone();
    spawn_registered(registry, write_permit, accepted, mutating, body);
    Ok(id)
}

/// Like [`spawn_job`], but for a caller whose closure needs the job id
/// before it runs — `POST /api/v1/code/index` uses it to build a
/// [`RegistryProgressSink`] so `api::index_code::run_with_progress` can
/// report progress into the very job it is running as. Otherwise
/// identical: the job is registered [`JobStatus::Queued`] first, then
/// `body` (handed its own id) runs exactly as [`spawn_job`]'s would.
pub fn spawn_job_with_id<F>(
    registry: &Arc<Registry>,
    write_permit: Arc<Semaphore>,
    command: &str,
    mutating: bool,
    body: F,
) -> Result<JobId>
where
    F: FnOnce(JobId) -> Result<Value> + Send + 'static,
{
    spawn_job_for(registry, write_permit, command, None, mutating, body)
}

/// [`spawn_job_with_id`] with a repo label recorded on the job. With
/// `Some(repo)` the registration is `Registry::insert_for_unless_active`:
/// a second `index-code` for a repo that already has a queued or running
/// one is refused with `Error::IndexRunning` (→ `409 index_running`)
/// atomically at insertion, so two concurrent requests for one repo can
/// never both queue — whatever pre-check the route ran before calling
/// this is only the fast path.
pub fn spawn_job_for<F>(
    registry: &Arc<Registry>,
    write_permit: Arc<Semaphore>,
    command: &str,
    repo: Option<&str>,
    mutating: bool,
    body: F,
) -> Result<JobId>
where
    F: FnOnce(JobId) -> Result<Value> + Send + 'static,
{
    let accepted = match repo {
        Some(repo) => registry.insert_for_unless_active(command, repo)?,
        None => registry.insert(command)?,
    };
    let id = accepted.id.clone();
    let body_id = id.clone();
    spawn_registered(registry, write_permit, accepted, mutating, move || {
        body(body_id)
    });
    Ok(id)
}

/// Shared tail of [`spawn_job`] / [`spawn_job_for`]: schedule the
/// already-registered job to run on its own `tokio::spawn`ed task, handing
/// it the job's own cancel flag alongside the id.
fn spawn_registered<F>(
    registry: &Arc<Registry>,
    write_permit: Arc<Semaphore>,
    accepted: Accepted,
    mutating: bool,
    body: F,
) where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let registry = Arc::clone(registry);
    let Accepted { id, cancel, .. } = accepted;
    tokio::spawn(async move { run(registry, write_permit, id, cancel, mutating, body).await });
}

/// The spawned task: acquire the permit (mutating only), flip `Queued` →
/// `Running` atomically, run `body` on the blocking pool, then record the
/// terminal status. The permit is dropped when this function returns —
/// after the blocking work and after the terminal status is recorded, so
/// the next queued writer cannot start early.
///
/// The pre-body cancel check is two-fold and registry-independent: the
/// task's own `cancel` clone catches a cancel that landed while the job was
/// queued even if the record has since been evicted (`registry::evict_finished`),
/// and `Registry::try_start` is the atomic check-and-flip for a record
/// that is still there — a cancel cannot slip in between a separate
/// "is it cancelled?" read and a "mark running" write, because there is no
/// such gap.
///
/// The `job task panicked` branch below only fires under `panic = "unwind"`
/// (dev/test default). This crate's `[profile.release]`/`[profile.dist]` —
/// every shipped binary — set `panic = "abort"`, under which a panic in
/// `body` aborts the whole process for every connected client instead of
/// landing this job in `JobStatus::Error`.
async fn run<F>(
    registry: Arc<Registry>,
    write_permit: Arc<Semaphore>,
    id: JobId,
    cancel: Arc<AtomicBool>,
    mutating: bool,
    body: F,
) where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let _permit = if mutating {
        match write_permit.acquire_owned().await {
            Ok(permit) => Some(permit),
            Err(e) => {
                let err = JobError::internal(format!("write permit unavailable: {e}"));
                set_status(&registry, &id, JobStatus::Error(err));
                return;
            }
        }
    } else {
        None
    };
    if cancel.load(Ordering::Relaxed) || !registry.try_start(&id) {
        // Already `Cancelled` (recorded by `Registry::cancel`) or gone;
        // there is no status left to publish and the body must not run.
        tracing::debug!(job_id = %id, "job cancelled or evicted before it started; body skipped");
        return;
    }
    let status = match tokio::task::spawn_blocking(body).await {
        Ok(Ok(value)) => JobStatus::Done(value),
        Ok(Err(Error::Cancelled)) => JobStatus::Cancelled,
        Ok(Err(e)) => JobStatus::Error(JobError::from_error(&e)),
        Err(e) => JobStatus::Error(JobError::internal(format!("job task panicked: {e}"))),
    };
    set_status(&registry, &id, status);
}

/// Publish `status` for job `id`. A failed update (only possible if the
/// record vanished or the registry lock was poisoned) is logged rather than
/// propagated — there is no caller left to return it to, and the job's work
/// has already happened.
fn set_status(registry: &Registry, id: &str, status: JobStatus) {
    if let Err(e) = registry.set_status(id, status) {
        tracing::warn!(job_id = id, error = %e, "job status update failed");
    }
}

/// Concrete [`ProgressSink`] that writes into the job [`Registry`] — what
/// `POST /api/v1/code/index` hands `api::index_code::run_with_progress` via
/// [`spawn_job_with_id`], so a real index-code job's progress and log
/// lines land in `JobView.progress`/`log_tail` and stream out as the SSE
/// `progress` event. A failed registry write only warns (§Error handling
/// "Progress sink" in the console-compat plan): this sink can never fail
/// the job it instruments.
pub struct RegistryProgressSink {
    registry: Arc<Registry>,
    id: JobId,
}

impl RegistryProgressSink {
    /// A sink reporting into `registry` for job `id`.
    pub fn new(registry: Arc<Registry>, id: JobId) -> Self {
        Self { registry, id }
    }
}

impl ProgressSink for RegistryProgressSink {
    fn on_progress(&self, done: u64, total: u64) {
        let progress = Progress {
            done,
            total,
            unit: "files".to_string(),
        };
        if let Err(e) = self.registry.set_progress(&self.id, progress) {
            tracing::warn!(job_id = %self.id, error = %e, "job progress update failed");
        }
    }

    fn on_log(&self, line: &str) {
        if let Err(e) = self.registry.push_log(&self.id, line.to_string()) {
            tracing::warn!(job_id = %self.id, error = %e, "job log update failed");
        }
    }

    fn is_cancelled(&self) -> bool {
        self.registry.is_cancelled(&self.id)
    }
}

#[cfg(test)]
#[path = "tests/worker.rs"]
mod tests;
