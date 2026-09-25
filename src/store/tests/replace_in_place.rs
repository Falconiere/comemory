#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/replace_in_place.rs` over real SQLite files in
//! WAL mode: a connection opened before the replace reads the new content
//! without reopening, a writer blocked during the copy commits nothing, a
//! page-size mismatch and a held write lock both leave the destination
//! untouched, and neither the file nor its sidecars are ever renamed away.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use comemory::errors::Error;
use rusqlite::Connection;
use tempfile::tempdir;

use super::{copy_pages, open_destination, replace_in_place};

/// Rows in [`big_source`]: enough 2 KiB values for a multi-page copy.
const BIG_ROWS: usize = 64;

/// Create a WAL-mode database at `path` whose table `t` holds `rows`.
fn wal_db(path: &Path, rows: &[&str]) -> Connection {
    let conn = Connection::open(path).expect("open");
    conn.pragma_update(None, "journal_mode", "WAL")
        .expect("wal");
    conn.execute_batch("CREATE TABLE t(v TEXT NOT NULL);")
        .expect("create");
    for row in rows {
        conn.execute("INSERT INTO t(v) VALUES (?1)", [row])
            .expect("insert");
    }
    conn
}

/// A closed source database spanning many pages, so a one-page-per-step copy
/// takes several steps.
fn big_source(path: &Path) {
    let conn = wal_db(path, &[]);
    for i in 0..BIG_ROWS {
        conn.execute(
            "INSERT INTO t(v) VALUES (?1)",
            [format!("{i:04}{}", "x".repeat(2048))],
        )
        .expect("insert big row");
    }
}

/// Every value in `t`, sorted, through a cached statement — the kind a
/// long-lived connection keeps across requests.
fn values(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare_cached("SELECT v FROM t ORDER BY v")
        .expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("rows")
}

/// `path` with `suffix` appended (`-wal`, `-shm`).
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}

#[test]
fn a_connection_opened_before_the_replace_reads_the_new_content_without_reopening() {
    let dir = tempdir().expect("tempdir");
    let live = dir.path().join("live.db");
    let source = dir.path().join("source.db");
    let reader = wal_db(&live, &["old"]);
    assert_eq!(values(&reader), ["old"]);
    {
        let src = wal_db(&source, &["new one", "new two"]);
        src.execute_batch(
            "CREATE TABLE only_in_source(x INTEGER); INSERT INTO only_in_source VALUES (7);",
        )
        .expect("source-only table");
    }

    let mut dest = open_destination(&live, Duration::from_secs(5)).expect("destination");
    replace_in_place(&mut dest, &source).expect("replace");

    assert_eq!(
        values(&reader),
        ["new one", "new two"],
        "a connection open since before the replace reads the new content on its next query"
    );
    let x: i64 = reader
        .query_row("SELECT x FROM only_in_source", [], |r| r.get(0))
        .expect("the new schema is visible without reopening");
    assert_eq!(x, 7);
    assert_eq!(
        values(&dest),
        ["new one", "new two"],
        "and so does the destination"
    );
}

#[test]
fn a_writer_blocked_during_the_copy_gets_busy_and_commits_nothing_then_commits_into_the_new_content()
 {
    let dir = tempdir().expect("tempdir");
    let live = dir.path().join("live.db");
    let source = dir.path().join("source.db");
    drop(wal_db(&live, &["old"]));
    big_source(&source);
    let writer = Connection::open(&live).expect("writer");
    writer
        .busy_timeout(Duration::from_millis(50))
        .expect("writer busy timeout");

    let mut dest = open_destination(&live, Duration::from_secs(5)).expect("destination");
    let mut attempts = Vec::new();
    copy_pages(&mut dest, &source, 1, &mut |progress| {
        if attempts.is_empty() {
            let outcome = writer
                .execute("INSERT INTO t(v) VALUES ('during the copy')", [])
                .map_err(Error::from);
            attempts.push((progress.remaining, outcome));
        }
    })
    .expect("copy");

    assert_eq!(attempts.len(), 1, "the copy took more than one step");
    let (remaining, outcome) = &attempts[0];
    assert!(*remaining > 0, "the write was attempted mid-copy");
    let err = outcome
        .as_ref()
        .expect_err("a writer must not commit while the copy holds the destination");
    assert!(
        comemory::store::busy::is_locked(err),
        "the blocked writer fails busy, got: {err}"
    );

    writer
        .execute("INSERT INTO t(v) VALUES ('after the copy')", [])
        .expect("the writer commits once the copy completes");
    let rows = values(&Connection::open(&live).expect("reader"));
    assert_eq!(
        rows.len(),
        BIG_ROWS + 1,
        "the new content plus the later write"
    );
    assert!(rows.contains(&"after the copy".to_string()));
    assert!(!rows.contains(&"during the copy".to_string()));
    assert!(!rows.contains(&"old".to_string()));
}

#[test]
fn a_mismatched_page_size_is_refused_naming_both_sizes_and_changes_nothing() {
    let dir = tempdir().expect("tempdir");
    let live = dir.path().join("live.db");
    let source = dir.path().join("source.db");
    drop(wal_db(&live, &["old"]));
    {
        let src = Connection::open(&source).expect("source");
        src.pragma_update(None, "page_size", 8192_i64)
            .expect("page size");
        src.pragma_update(None, "journal_mode", "WAL").expect("wal");
        src.execute_batch("CREATE TABLE t(v TEXT NOT NULL); INSERT INTO t VALUES ('new');")
            .expect("seed source");
        let size: i64 = src
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .expect("read page size");
        assert_eq!(size, 8192, "the fixture really has a different page size");
    }

    let mut dest = open_destination(&live, Duration::from_secs(5)).expect("destination");
    let err = replace_in_place(&mut dest, &source).expect_err("a page-size mismatch is refused");
    let msg = err.to_string();
    assert!(
        msg.contains("8192") && msg.contains("4096"),
        "the refusal names both page sizes, got: {msg}"
    );
    assert_eq!(values(&dest), ["old"], "the destination is unchanged");
}

#[test]
fn a_held_write_lock_fails_busy_and_changes_nothing() {
    let dir = tempdir().expect("tempdir");
    let live = dir.path().join("live.db");
    let source = dir.path().join("source.db");
    let blocker = wal_db(&live, &["old"]);
    drop(wal_db(&source, &["new"]));
    blocker
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO t(v) VALUES ('uncommitted');")
        .expect("hold the write lock");

    let mut dest = open_destination(&live, Duration::from_millis(100)).expect("destination");
    let err = replace_in_place(&mut dest, &source).expect_err("a held write lock blocks the copy");
    assert!(matches!(err, Error::Busy(_)), "fails busy, got: {err}");

    blocker.execute_batch("ROLLBACK;").expect("release");
    assert_eq!(values(&dest), ["old"], "the destination is unchanged");
}

#[test]
fn the_file_keeps_its_inode_and_no_sidecar_is_removed() {
    let dir = tempdir().expect("tempdir");
    let live = dir.path().join("live.db");
    let source = dir.path().join("source.db");
    let holder = wal_db(&live, &["old"]);
    let inode = std::fs::metadata(&live).expect("stat").ino();
    for suffix in ["-wal", "-shm"] {
        assert!(sidecar(&live, suffix).exists(), "fixture holds {suffix}");
    }
    drop(wal_db(&source, &["new"]));

    let mut dest = open_destination(&live, Duration::from_secs(5)).expect("destination");
    replace_in_place(&mut dest, &source).expect("replace");

    assert_eq!(
        std::fs::metadata(&live).expect("stat").ino(),
        inode,
        "the content was replaced in the same file, not renamed over it"
    );
    for suffix in ["-wal", "-shm"] {
        assert!(
            sidecar(&live, suffix).exists(),
            "the live {suffix} an open connection still uses was not removed"
        );
    }
    assert_eq!(values(&holder), ["new"]);
}
