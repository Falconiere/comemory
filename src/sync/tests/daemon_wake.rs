#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Wake file + interruptible sleep (no live daemon required).

use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use comemory::config::Paths;
use comemory::sync::daemon_wake::{
    signal_wake, sleep_interruptible, sleep_interruptible_at, take_wake, wake_after_save_best_effort,
    wake_file,
};

use crate::test_common as common;

#[test]
fn wake_file_lives_under_data_dir() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    assert_eq!(wake_file(&paths), sb.data_dir().join("sync.wake"));
    assert_eq!(paths.sync_wake_file(), sb.data_dir().join("sync.wake"));
}

#[test]
fn signal_wake_creates_file_take_clears_it() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    assert!(!take_wake(&paths));
    signal_wake(&paths).expect("signal");
    assert!(wake_file(&paths).exists());
    assert!(take_wake(&paths));
    assert!(!wake_file(&paths).exists());
    assert!(!take_wake(&paths));
}

#[test]
fn wake_after_save_skips_empty_repo() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    wake_after_save_best_effort(&paths, "");
    wake_after_save_best_effort(&paths, "   ");
    assert!(!wake_file(&paths).exists());
}

#[test]
fn wake_after_save_signals_for_labelled_repo() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    wake_after_save_best_effort(&paths, "acme/cli");
    assert!(wake_file(&paths).exists());
}

#[test]
fn sleep_returns_immediately_when_wake_pending() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    signal_wake(&paths).expect("signal");
    let started = Instant::now();
    sleep_interruptible(Duration::from_secs(30), &paths);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "pending wake must not wait the full interval"
    );
    assert!(!wake_file(&paths).exists());
}

#[test]
fn sleep_wakes_when_file_appears_mid_wait() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wake_path: PathBuf = dir.path().join("sync.wake");
    let barrier = Arc::new(Barrier::new(2));
    let wake_for_sleeper = wake_path.clone();
    let barrier_sleeper = Arc::clone(&barrier);

    let sleeper = thread::spawn(move || {
        barrier_sleeper.wait();
        let started = Instant::now();
        sleep_interruptible_at(Duration::from_secs(30), &wake_for_sleeper);
        started.elapsed()
    });

    barrier.wait();
    // Let the sleeper enter its poll loop before signalling.
    thread::sleep(Duration::from_millis(50));
    std::fs::write(&wake_path, b"1").expect("write wake");
    let elapsed = sleeper.join().expect("sleeper");
    assert!(
        elapsed < Duration::from_secs(2),
        "mid-wait wake took {elapsed:?}"
    );
    assert!(!wake_path.exists());
}
