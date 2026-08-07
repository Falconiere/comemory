#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/serve/jobs/worker.rs`, driven directly (no HTTP —
//! no job-creating route exists yet; see the plan's `## Deviations`).
//!
//! Every "real work" job here runs `api::gc::run` through a `Ctx::lazy`
//! over a real temp data-dir, so the worker is exercised against the actual
//! command core, a real SQLite file and a real filesystem — no mocks. The
//! permit tests use the same `Arc<Semaphore>` type `AppState` holds.

use std::sync::Arc;
use std::time::Duration;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::serve::jobs::{JobStatus, Registry, spawn_job};
use serde_json::Value;
use tokio::sync::Semaphore;

/// A real `gc` run over `dir`, shaped as a job body.
fn gc_body(dir: std::path::PathBuf) -> impl FnOnce() -> comemory::prelude::Result<Value> {
    move || {
        let paths = Paths::new(&dir);
        paths.ensure_dirs()?;
        let cfg = Config::defaults();
        let mut ctx = Ctx::lazy(&paths, &cfg);
        let report = api::gc::run(&mut ctx, api::gc::Request {})?;
        serde_json::to_value(report).map_err(Error::Json)
    }
}

/// Poll the registry until `id` reaches a terminal status, or fail after
/// `timeout`.
async fn await_terminal(registry: &Registry, id: &str, timeout: Duration) -> String {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let view = registry
            .get(id)
            .expect("lock")
            .expect("job present in registry");
        if view.status == "done" || view.status == "error" {
            return view.status.to_string();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "job {id} stuck in status {}",
            view.status
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The job's current status slug.
fn status_of(registry: &Registry, id: &str) -> String {
    registry
        .get(id)
        .expect("lock")
        .expect("job present")
        .status
        .to_string()
}

/// Poll until `id` reaches `want`, or fail after `timeout`.
async fn await_status(registry: &Registry, id: &str, want: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    while status_of(registry, id) != want {
        assert!(
            tokio::time::Instant::now() < deadline,
            "job {id} never reached {want} (stuck in {})",
            status_of(registry, id)
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A job body that blocks on `blocker` until the test releases it, so the
/// write permit is provably held for the whole run.
fn blocking_body(
    blocker: std::sync::mpsc::Receiver<()>,
) -> impl FnOnce() -> comemory::prelude::Result<Value> {
    move || {
        blocker
            .recv_timeout(Duration::from_secs(20))
            .map_err(|e| Error::Other(format!("blocker: {e}")))?;
        Ok(serde_json::json!({"first": true}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mutating_job_runs_real_work_to_done() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));

    let id = spawn_job(
        &registry,
        Arc::clone(&permit),
        "gc",
        true,
        gc_body(home.path().to_path_buf()),
    )
    .expect("spawn");

    assert_eq!(
        await_terminal(&registry, &id, Duration::from_secs(20)).await,
        "done"
    );
    let view = registry.get(&id).expect("lock").expect("job");
    assert_eq!(view.command, "gc");
    assert!(view.error.is_none(), "unexpected error: {:?}", view.error);
    let result = view.result.expect("gc payload");
    assert_eq!(result["removed"], serde_json::json!(0));
    assert!(result["log_rows"].is_number(), "payload: {result}");
    assert!(view.finished_at.is_some());
    assert_eq!(
        permit.available_permits(),
        1,
        "the write permit must be released once the job finished"
    );
}

/// The same watch-replay mechanism AC-8's SSE stream uses, observed at the
/// worker level: subscribing only after the job finished still yields the
/// terminal status immediately.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_subscriber_reads_the_finished_jobs_terminal_status() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));

    let id = spawn_job(
        &registry,
        permit,
        "gc",
        true,
        gc_body(home.path().to_path_buf()),
    )
    .expect("spawn");
    assert_eq!(
        await_terminal(&registry, &id, Duration::from_secs(20)).await,
        "done"
    );

    let mut late = registry
        .subscribe(&id)
        .expect("lock")
        .expect("sender retained");
    let status = late.borrow_and_update().clone();

    assert!(
        status.is_terminal(),
        "late subscriber saw {}",
        status.slug()
    );
    assert_eq!(status.slug(), "done");
    assert!(status.result().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_job_lands_in_error_with_the_mapped_code() {
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));

    let id = spawn_job(&registry, Arc::clone(&permit), "index-code", true, || {
        Err(Error::NotFound("no such repo: ghost".into()))
    })
    .expect("spawn");

    assert_eq!(
        await_terminal(&registry, &id, Duration::from_secs(10)).await,
        "error"
    );
    let view = registry.get(&id).expect("lock").expect("job");
    assert!(view.result.is_none(), "a failed job carries no result");
    let error = view.error.expect("error object");
    assert_eq!(error.code, "not_found");
    assert!(
        error.message.contains("ghost"),
        "message: {}",
        error.message
    );
    assert_eq!(permit.available_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mutating_job_waits_for_an_externally_held_write_permit() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));
    let held = Arc::clone(&permit)
        .acquire_owned()
        .await
        .expect("hold the permit");

    let id = spawn_job(
        &registry,
        Arc::clone(&permit),
        "gc",
        true,
        gc_body(home.path().to_path_buf()),
    )
    .expect("spawn");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        status_of(&registry, &id),
        "queued",
        "a mutating job must not start while the permit is held"
    );

    drop(held);
    assert_eq!(
        await_terminal(&registry, &id, Duration::from_secs(20)).await,
        "done"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_class_job_ignores_the_write_permit() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));
    let held = Arc::clone(&permit)
        .acquire_owned()
        .await
        .expect("hold the permit");

    let id = spawn_job(
        &registry,
        Arc::clone(&permit),
        "eval",
        false,
        gc_body(home.path().to_path_buf()),
    )
    .expect("spawn");

    assert_eq!(
        await_terminal(&registry, &id, Duration::from_secs(20)).await,
        "done",
        "a read-class job must run even while the write permit is held"
    );
    drop(held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_mutating_job_queues_behind_the_first() {
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));
    let (release, blocker) = std::sync::mpsc::channel::<()>();

    let first = spawn_job(
        &registry,
        Arc::clone(&permit),
        "index-code",
        true,
        blocking_body(blocker),
    )
    .expect("spawn first");
    // Wait for the first job to actually hold the permit.
    await_status(&registry, &first, "running", Duration::from_secs(10)).await;

    let second = spawn_job(&registry, Arc::clone(&permit), "rebuild", true, || {
        Ok(serde_json::json!({"second": true}))
    })
    .expect("spawn second");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        status_of(&registry, &second),
        "queued",
        "the second mutating job must queue behind the first"
    );
    assert_eq!(status_of(&registry, &first), "running");

    release.send(()).expect("release the first job");
    assert_eq!(
        await_terminal(&registry, &first, Duration::from_secs(20)).await,
        "done"
    );
    assert_eq!(
        await_terminal(&registry, &second, Duration::from_secs(20)).await,
        "done"
    );
    assert_eq!(permit.available_permits(), 1);
}

/// A job body panic (only unwinds in a dev/test build — the shipped
/// release/dist binary sets `panic = "abort"`, see `worker::run`'s doc
/// comment) must still land the job in `JobStatus::Error` with a
/// panic-indicating message, and the write permit must genuinely be
/// released afterward — proven by a subsequent job acquiring it and
/// running to completion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_job_lands_in_error_and_releases_the_permit() {
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));

    let id = spawn_job(&registry, Arc::clone(&permit), "rebuild", true, || {
        panic!("boom: simulated job panic");
    })
    .expect("spawn");

    assert_eq!(
        await_terminal(&registry, &id, Duration::from_secs(10)).await,
        "error"
    );
    let view = registry.get(&id).expect("lock").expect("job");
    assert!(view.result.is_none(), "a panicked job carries no result");
    let error = view.error.expect("error object");
    assert_eq!(error.code, "internal");
    assert!(
        error.message.contains("panicked"),
        "message must indicate a panic: {}",
        error.message
    );
    assert_eq!(
        permit.available_permits(),
        1,
        "the write permit must be released after a panicking job"
    );

    // Prove the permit is genuinely usable again: a second job acquires it
    // and runs to completion.
    let second = spawn_job(&registry, Arc::clone(&permit), "rebuild", true, || {
        Ok(serde_json::json!({"second": true}))
    })
    .expect("spawn second");
    assert_eq!(
        await_terminal(&registry, &second, Duration::from_secs(10)).await,
        "done",
        "a subsequent job must acquire the permit released by the panicked job"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_registers_the_job_before_returning() {
    let registry = Arc::new(Registry::new());
    let permit = Arc::new(Semaphore::new(1));
    let held = Arc::clone(&permit)
        .acquire_owned()
        .await
        .expect("hold the permit");

    let id = spawn_job(&registry, Arc::clone(&permit), "rebuild", true, || {
        Ok(Value::Null)
    })
    .expect("spawn");

    // The `202` body a job-creating route builds is exactly this pair.
    let view = registry
        .get(&id)
        .expect("lock")
        .expect("job visible immediately after spawn_job returns");
    assert_eq!(view.job_id, id);
    assert_eq!(view.status, JobStatus::Queued.slug());
    drop(held);
}
