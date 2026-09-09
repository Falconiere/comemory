#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Verifies `store::connection::open` returns a SQLite connection with
//! WAL journal mode active and the sqlite-vec extension registered.

use comemory::store::connection;
use tempfile::tempdir;

#[test]
fn opens_db_and_loads_sqlite_vec() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    let mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("query journal_mode");
    assert_eq!(mode.to_lowercase(), "wal");

    // sqlite-vec exposes a `vec_version()` SQL function. If the
    // extension didn't register, this query errors.
    let version: String = conn
        .query_row("SELECT vec_version()", [], |row| row.get(0))
        .expect("vec_version");
    assert!(version.starts_with('v'), "got version: {version}");
}

/// Regression: opening a DB whose schema is already current must perform
/// zero row writes. A write on open takes SQLite's single WAL write lock,
/// so read-only commands (`search`, `list`, `context`) would contend with
/// any concurrent writer and fail with "database is locked". Previously
/// `migrate::set_version` UPSERTed the version on every open unconditionally.
#[test]
fn open_on_current_schema_performs_no_writes() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");

    // First open creates the schema and records the current version.
    drop(connection::open(&path).expect("first open"));

    // A second open on the now-current schema must not write any rows.
    let conn = connection::open(&path).expect("second open");
    assert_eq!(
        conn.total_changes(),
        0,
        "open wrote rows on an already-current schema (write-on-open contends for the WAL lock)"
    );
}

/// `open_read_only` opens a genuinely writable-looking file but refuses
/// every write — the read-only forward-compat fallback (`api::doctor`) must
/// never be able to mutate a schema it does not understand.
#[test]
fn open_read_only_refuses_writes() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    // Regular open first, so the file and schema already exist.
    drop(connection::open(&path).expect("first open"));

    let conn = connection::open_read_only(&path).expect("read-only open");
    let err = conn
        .execute("DELETE FROM memories", [])
        .expect_err("a read-only connection must refuse a write");
    // Assert the structured error CODE, not just the rendered text: SQLite's
    // message wording is not a stability contract, and a substring match
    // would also accept an unrelated error that happened to contain the word.
    assert!(
        matches!(
            err,
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::ReadOnly,
                    ..
                },
                _
            )
        ),
        "expected SQLITE_READONLY, got: {err}"
    );
}
