//! Test mirror for `src/source/lock.rs`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use comemory::source::lock::RegistryLock;
use tempfile::TempDir;

/// Acquire the lock, read `data_path`, sleep to widen the race window,
/// then append `marker` and write back. Without exclusion, two threads
/// racing this both read the empty/short starting content and clobber
/// each other's write; under the lock, the second thread's read only
/// happens after the first's write completes.
fn read_modify_write(lock_path: &std::path::Path, data_path: &std::path::Path, marker: &str) {
    let _guard = RegistryLock::acquire(lock_path).expect("acquire lock");
    let existing = fs::read_to_string(data_path).unwrap_or_default();
    thread::sleep(Duration::from_millis(20));
    fs::write(data_path, format!("{existing}{marker}\n")).expect("write data");
}

/// This is AC-9's unit-level proof: two threads perform a
/// read-modify-write cycle under the same `RegistryLock`; both markers
/// must survive, proving the lock actually excludes rather than merely
/// existing as an unused file.
#[test]
fn concurrent_read_modify_write_both_survive() {
    let tmp = TempDir::new().expect("tempdir");
    let lock_path = Arc::new(tmp.path().join("sources.toml.lock"));
    let data_path = Arc::new(tmp.path().join("data.txt"));
    fs::write(&*data_path, "").expect("seed data file");

    let (lp1, dp1) = (Arc::clone(&lock_path), Arc::clone(&data_path));
    let t1 = thread::spawn(move || read_modify_write(&lp1, &dp1, "source-a"));
    let (lp2, dp2) = (Arc::clone(&lock_path), Arc::clone(&data_path));
    let t2 = thread::spawn(move || read_modify_write(&lp2, &dp2, "source-b"));
    t1.join().expect("thread a");
    t2.join().expect("thread b");

    let contents = fs::read_to_string(&*data_path).expect("read result");
    assert!(contents.contains("source-a"), "lost write a: {contents}");
    assert!(contents.contains("source-b"), "lost write b: {contents}");
    assert_eq!(
        contents.lines().count(),
        2,
        "one write must have clobbered the other under a broken lock: {contents}"
    );
}

#[test]
fn acquire_creates_missing_lock_file_and_releases_on_drop() {
    let tmp = TempDir::new().expect("tempdir");
    let lock_path: PathBuf = tmp.path().join("sources.toml.lock");
    assert!(!lock_path.exists());

    {
        let _guard = RegistryLock::acquire(&lock_path).expect("acquire");
        assert!(lock_path.exists(), "acquire must create the lock file");
    }

    // A second acquire after the guard dropped must not block forever.
    let _guard2 = RegistryLock::acquire(&lock_path).expect("re-acquire after drop");
}
