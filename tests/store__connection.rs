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
    assert!(version.starts_with("v"), "got version: {version}");
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
