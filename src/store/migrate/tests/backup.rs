#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for `crate::store::migrate::backup`: `snapshot`/`snapshot_path`
//! against real SQLite databases with real rows, `prune`'s prefix-scoped
//! `KEEP = 2` budget, and the stale-`.bak` replacement guard.

use std::path::Path;
use std::time::{Duration, SystemTime};

use comemory::store::connection;
use comemory::store::migrate::backup::{prune, snapshot, snapshot_path};
use rusqlite::Connection;
use tempfile::tempdir;

/// Insert one minimal `memories` row directly — mirrors the shape
/// `src/store/tests/migrate_2.rs::seed_memory` uses, kept local since these
/// suites are exempt from the duplication ratchet (`dup-check.sh` excludes
/// test trees) and this module has no other reason to depend on that one.
fn seed_memory(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, content_hash, body,
                              created_at, updated_at, md_path)
         VALUES(?1, 'note', 'decision', 'h', 'body',
                '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', 'm/x.md')",
        [id],
    )
    .expect("seed memory row");
}

/// True when `id` has a `memories` row in `conn`.
fn has_memory(conn: &Connection, id: &str) -> bool {
    conn.query_row("SELECT count(*) FROM memories WHERE id = ?1", [id], |r| {
        r.get::<_, i64>(0)
    })
    .expect("count probe")
        == 1
}

/// Write a stub file at `path` and set its mtime to a deterministic offset
/// from a fixed epoch, so [`prune`]'s newest-first ordering does not depend
/// on filesystem timestamp resolution or wall-clock sleeps.
fn touch_with_mtime(path: &Path, offset_secs: u64) {
    std::fs::write(path, b"stub").expect("write stub file");
    let file = std::fs::File::open(path).expect("open for mtime");
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + offset_secs);
    file.set_modified(time).expect("set mtime");
}

#[test]
fn snapshot_opens_and_matches_source_contents() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open");
    seed_memory(&conn, "aaaa1111");

    let dest = dir.path().join("comemory.db.pre-v12.bak");
    snapshot(&conn, &dest).expect("snapshot");

    let snap = Connection::open(&dest).expect("open snapshot as sqlite");
    assert!(
        has_memory(&snap, "aaaa1111"),
        "snapshot must contain the pre-upgrade row"
    );
}

#[test]
fn snapshot_is_idempotent_against_its_own_valid_output() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open");
    seed_memory(&conn, "aaaa1111");

    let dest = dir.path().join("comemory.db.pre-v12.bak");
    snapshot(&conn, &dest).expect("first snapshot");
    // VACUUM INTO errors on an existing destination; a second call must
    // detect the valid prior snapshot and skip rather than propagate that
    // error.
    snapshot(&conn, &dest).expect("second snapshot must skip, not error");
}

#[test]
fn snapshot_path_opens_its_own_connection_and_matches_source() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("comemory.db");
    {
        let conn = connection::open(&db_path).expect("open");
        seed_memory(&conn, "bbbb2222");
    }

    let dest = dir.path().join("comemory.db.pre-rebuild.bak");
    snapshot_path(&db_path, &dest).expect("snapshot_path");

    let snap = Connection::open(&dest).expect("open snapshot as sqlite");
    assert!(has_memory(&snap, "bbbb2222"));
}

#[test]
fn prune_keeps_newest_two_and_is_prefix_scoped() {
    let dir = tempdir().expect("tempdir");
    let data_dir = dir.path();

    for (name, secs) in [
        ("comemory.db.pre-v10.bak", 10),
        ("comemory.db.pre-v11.bak", 20),
        ("comemory.db.pre-v12.bak", 30),
    ] {
        touch_with_mtime(&data_dir.join(name), secs);
    }
    // A different prefix's snapshot must never be evicted by pruning the
    // migration budget, even though it is the oldest file overall.
    touch_with_mtime(&data_dir.join("comemory.db.pre-rebuild.bak"), 1);

    prune(data_dir, "comemory.db.pre-v").expect("prune");

    assert!(
        !data_dir.join("comemory.db.pre-v10.bak").exists(),
        "oldest matching snapshot must be pruned"
    );
    assert!(
        data_dir.join("comemory.db.pre-v11.bak").exists(),
        "second-newest matching snapshot must survive"
    );
    assert!(
        data_dir.join("comemory.db.pre-v12.bak").exists(),
        "newest matching snapshot must survive"
    );
    assert!(
        data_dir.join("comemory.db.pre-rebuild.bak").exists(),
        "a snapshot under a different prefix must be untouched"
    );
}

#[test]
fn truncated_existing_bak_is_replaced_not_trusted() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open");
    seed_memory(&conn, "cccc3333");

    let dest = dir.path().join("comemory.db.pre-v12.bak");
    snapshot(&conn, &dest).expect("first snapshot");

    // Simulate a process killed mid-write: keep only the first 200 bytes
    // of an otherwise-valid SQLite file.
    let bytes = std::fs::read(&dest).expect("read snapshot bytes");
    assert!(
        bytes.len() > 200,
        "fixture snapshot must be large enough to truncate"
    );
    std::fs::write(&dest, &bytes[..200]).expect("truncate snapshot");
    assert!(
        Connection::open(&dest)
            .and_then(|c| c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)))
            .is_ok_and(|r| r == "ok")
            .then_some(())
            .is_none(),
        "fixture must actually be invalid, or this test proves nothing"
    );

    // A second, distinguishable row is added to the source between the
    // truncation and the re-snapshot: only if the truncated `.bak` is
    // detected as invalid and replaced will this row show up in it.
    seed_memory(&conn, "dddd4444");
    snapshot(&conn, &dest).expect("second snapshot must replace the corrupt file");

    let snap = Connection::open(&dest).expect("open replaced snapshot");
    // The replaced snapshot carries `memory_fts` (`tokenize = 'identifier'`)
    // like any real comemory database, so `quick_check` needs the
    // tokenizer registered on this connection first — same reason
    // `backup::is_valid_sqlite` registers it internally.
    comemory::store::tokenizer::ffi::register(&snap).expect("register identifier tokenizer");
    let check: String = snap
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .expect("quick_check on replaced snapshot");
    assert_eq!(check, "ok", "replaced snapshot must be a valid SQLite file");
    assert!(
        has_memory(&snap, "dddd4444"),
        "replaced snapshot must reflect the source state at replace time, not the stale truncated one"
    );
}
