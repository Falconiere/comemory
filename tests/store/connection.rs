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
