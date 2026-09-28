#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::domains::memories::save_lock`]: `SaveGuard`
//! construction is the only door, and it is bounded.

use std::thread;
use std::time::Duration;

use comemory::config::paths::Paths;
use comemory::domains::memories::save_lock::acquire_within;
use comemory::utilities::file_lock::FileLock;

#[test]
fn a_free_lock_is_acquired_well_inside_the_bound() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    std::fs::create_dir_all(paths.data_dir()).expect("data dir");

    let _guard = acquire_within(&paths, Duration::from_secs(5)).expect("acquire");
}

#[test]
fn a_lock_still_held_past_the_bound_reports_busy_not_a_hang() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    std::fs::create_dir_all(paths.data_dir()).expect("data dir");
    let lock_path = paths.data_dir().join("memory-save.lock");
    let _holder = FileLock::acquire(&lock_path, "test-holder").expect("hold the lock");

    let started = std::time::Instant::now();
    let err = acquire_within(&paths, Duration::from_millis(200)).expect_err("must not hang");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the bound must actually cap the wait: {:?}",
        started.elapsed()
    );
    assert!(
        matches!(err, comemory::prelude::Error::Busy(_)),
        "a contended acquire must report Busy, not any other error: {err}"
    );
}

#[test]
fn a_release_before_the_bound_lets_the_waiter_through() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    std::fs::create_dir_all(paths.data_dir()).expect("data dir");
    let lock_path = paths.data_dir().join("memory-save.lock");
    let holder = FileLock::acquire(&lock_path, "test-holder").expect("hold the lock");

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        drop(holder);
    });

    let guard = acquire_within(&paths, Duration::from_secs(5))
        .expect("the waiter must succeed once the holder releases");
    drop(guard);
}
