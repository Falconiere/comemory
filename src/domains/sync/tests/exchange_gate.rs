#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tests for [`comemory::domains::sync::exchange_gate`]: the pause holds the
//! same `sync.lock` every exchange pass takes, and it is bounded.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use comemory::config::paths::Paths;
use comemory::domains::sync::auto;
use comemory::domains::sync::exchange_gate::pause;
use comemory::prelude::Error;
use comemory::utilities::file_lock::FileLock;

fn data_dir() -> (tempfile::TempDir, Paths) {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    std::fs::create_dir_all(paths.data_dir()).expect("data dir");
    (home, paths)
}

#[test]
fn a_held_pause_blocks_a_pass_until_it_drops() {
    let (_home, paths) = data_dir();
    let gate = pause(&paths, Duration::from_secs(5)).expect("pause");

    let (tx, rx) = mpsc::channel();
    let pass_paths = paths.clone();
    let pass = thread::spawn(move || {
        let _pass = auto::hold_pass_lock(&pass_paths).expect("pass lock");
        tx.send(Instant::now()).expect("send");
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "a pass must not start while the exchange is paused"
    );
    let released = Instant::now();
    drop(gate);
    let started = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the pass must resume once the pause drops");
    assert!(started >= released, "the pass started before the release");
    pass.join().expect("join");
}

#[test]
fn a_running_pass_past_the_bound_reports_busy_not_a_hang() {
    let (_home, paths) = data_dir();
    let _pass = auto::hold_pass_lock(&paths).expect("a pass holds the gate");

    let started = Instant::now();
    let err = pause(&paths, Duration::from_millis(200)).expect_err("must not hang");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the bound must cap the wait: {:?}",
        started.elapsed()
    );
    assert!(matches!(err, Error::Busy(_)), "expected Busy, got {err}");
}

#[test]
fn a_pass_finishing_inside_the_bound_is_drained_then_paused() {
    let (_home, paths) = data_dir();
    let holder = FileLock::acquire(&paths.data_dir().join(auto::PASS_LOCK), "test-pass")
        .expect("a pass holds the gate");
    let finished = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        drop(holder);
        Instant::now()
    });

    let _gate = pause(&paths, Duration::from_secs(5)).expect("pause after the pass ends");
    let paused_at = Instant::now();
    assert!(
        finished.join().expect("join") <= paused_at,
        "the pause must wait for the running pass to finish"
    );
    assert!(
        FileLock::try_acquire(&paths.data_dir().join(auto::PASS_LOCK), "probe")
            .expect("probe")
            .is_none(),
        "the pause must hold sync.lock itself"
    );
}
