//! The in-process job table: one [`Job`] record per accepted job, each with
//! a retained `watch::Sender<JobStatus>` so a status stream can attach (or
//! re-attach) at any time and immediately read the current value.
//!
//! Memory is bounded by evicting finished jobs beyond the
//! [`MAX_FINISHED`] most recent on every insertion; queued and running jobs
//! are never evicted. The `Mutex` is held only for the short synchronous
//! map mutations — never across an `.await`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use time::OffsetDateTime;
use tokio::sync::watch;

use crate::prelude::*;
use crate::serve::jobs::{Job, JobId, JobStatus, JobView};
use crate::serve::security;
use crate::store::memory_row;

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
    /// its fresh id and a receiver already positioned on that initial
    /// status. Also runs the finished-job eviction sweep.
    pub fn insert(&self, command: &str) -> Result<(JobId, watch::Receiver<JobStatus>)> {
        let id = security::random_hex(JOB_ID_BYTES)?;
        let started_at = memory_row::iso_format(OffsetDateTime::now_utc())?;
        let (tx, rx) = watch::channel(JobStatus::Queued);
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let job = Job::new(id.clone(), command.to_string(), started_at, seq, tx);
        let mut jobs = self.lock()?;
        jobs.insert(id.clone(), job);
        evict_finished(&mut jobs);
        Ok((id, rx))
    }

    /// Record a job's transition: update the stored record (stamping
    /// `finished_at` on the first terminal status) and publish the new
    /// value on the job's `watch` channel. `NotFound` when the id is
    /// unknown.
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
        if job.finished_at.is_none() {
            job.finished_at = finished_at;
        }
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

/// Drop the oldest finished jobs (by insertion order) until at most
/// [`MAX_FINISHED`] remain. Queued and running jobs are never counted and
/// never evicted — losing one would strand its worker's status updates.
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
