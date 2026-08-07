//! [`spawn_job`] — the one place a background job is started. Job-creating
//! routes hand it a closure (typically `api::<cmd>::run` over a
//! `Ctx::lazy`, i.e. the job's **own** connection) and get back a job id to
//! put in their `202 Accepted` body; everything after that is this module's
//! bookkeeping.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Semaphore;

use crate::prelude::*;
use crate::serve::jobs::{JobError, JobId, JobStatus, Registry};

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
    let (id, _rx) = registry.insert(command)?;
    let registry = Arc::clone(registry);
    let job_id = id.clone();
    tokio::spawn(async move { run(registry, write_permit, job_id, mutating, body).await });
    Ok(id)
}

/// The spawned task: acquire the permit (mutating only), mark `Running`,
/// run `body` on the blocking pool, then record the terminal status. The
/// permit is dropped when this function returns — after the blocking work
/// and after the terminal status is recorded, so the next queued writer
/// cannot start early.
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
    set_status(&registry, &id, JobStatus::Running);
    let status = match tokio::task::spawn_blocking(body).await {
        Ok(Ok(value)) => JobStatus::Done(value),
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

#[cfg(test)]
#[path = "tests/worker.rs"]
mod tests;
