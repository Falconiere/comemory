#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/busy.rs`.

use comemory::errors::Error;
use comemory::store::{busy, connection};
use tempfile::TempDir;

/// A deterministic `SQLITE_BUSY`: a second connection to the same file, with
/// its own `busy_timeout` forced to `0` (the project default of 5000ms would
/// make it WAIT instead of failing — the trap this test exists to avoid),
/// attempts a write while the first connection holds `BEGIN EXCLUSIVE`. No
/// sleeping and no racing: the lock is already held before the write is even
/// attempted, so the failure is guaranteed on every run.
#[test]
fn is_locked_true_for_a_real_sqlite_busy() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("comemory.db");

    let holder = connection::open(&db_path).expect("open holder connection");
    holder
        .execute("BEGIN EXCLUSIVE", [])
        .expect("begin exclusive");

    let contender = rusqlite::Connection::open(&db_path).expect("open second connection");
    contender
        .pragma_update(None, "busy_timeout", 0_i64)
        .expect("set busy_timeout=0 on contender");

    let write_err = contender
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES ('probe', '1')",
            [],
        )
        .expect_err("write must fail while the holder's exclusive lock is live");
    let crate_err = Error::Sqlite(write_err);

    assert!(
        busy::is_locked(&crate_err),
        "expected a SQLITE_BUSY/SQLITE_LOCKED error, got {crate_err:?}"
    );

    holder.execute("ROLLBACK", []).expect("rollback holder");
}

/// The negative case is what makes the predicate worth having: it must not
/// fire for a merely absent row, nor for a non-`Sqlite` `Error` variant —
/// either would turn an ordinary error into a spurious HTTP 503.
#[test]
fn is_locked_false_for_no_rows_and_non_sqlite_errors() {
    let no_rows = Error::Sqlite(rusqlite::Error::QueryReturnedNoRows);
    assert!(!busy::is_locked(&no_rows));

    let not_found = Error::NotFound("nope".into());
    assert!(!busy::is_locked(&not_found));
}
