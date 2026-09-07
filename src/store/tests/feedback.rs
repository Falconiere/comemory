#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration coverage for `src/store/feedback.rs` — the `feedback` /
//! memory-tagged `feedback_events` CRUD moved out of `stats::feedback`.
//! Driven through the real [`StatsDb`] + `record_with_provenance` path
//! (the domain writer that still owns the transaction boundary) rather
//! than calling the `pub(crate)` store helpers with a bare connection, so
//! the test exercises the same integration path the pre-move code took.

use comemory::config::paths::Paths;
use comemory::stats::feedback::record_with_provenance;
use comemory::stats::sqlite::StatsDb;
use tempfile::TempDir;

/// Open a [`StatsDb`] over a fresh `comemory.db` in a tempdir.
fn open_db() -> (StatsDb, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path());
    let db = StatsDb::open(paths.stats_db()).expect("open stats db");
    (db, tmp)
}

#[test]
fn absent_memory_row_precedes_first_record() {
    let (db, _tmp) = open_db();
    // No feedback row exists yet for a memory that has never been scored —
    // this is the "row absent" side of the moved SQL's contract.
    let row: Option<i64> = db
        .conn()
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = 'aaaaaaa1'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(row, None, "no counter row before any feedback is recorded");
}

#[test]
fn record_with_provenance_seeds_counters_and_tagged_events() {
    let (mut db, _tmp) = open_db();
    record_with_provenance(
        &mut db,
        "q-20260610-aabbccdd",
        &["aaaaaaa1".into()],
        &["aaaaaaa2".into()],
    )
    .expect("record");

    let conn = db.conn();
    let (used, irrelevant): (i64, i64) = conn
        .query_row(
            "SELECT used_count, irrelevant_count FROM feedback WHERE memory_id = 'aaaaaaa1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("used row");
    assert_eq!((used, irrelevant), (1, 0));

    let target_kind: String = conn
        .query_row(
            "SELECT target_kind FROM feedback_events WHERE memory_id = 'aaaaaaa2'",
            [],
            |r| r.get(0),
        )
        .expect("event row");
    assert_eq!(
        target_kind, "memory",
        "the store helper writes the caller's target_kind verbatim"
    );
}

#[test]
fn conflict_bumps_used_count_and_refreshes_last_used() {
    let (mut db, _tmp) = open_db();
    record_with_provenance(&mut db, "q-20260610-aabbccd1", &["aaaaaaa1".into()], &[])
        .expect("first record");
    db.conn()
        .execute(
            "UPDATE feedback SET last_used = '2000-01-01T00:00:00Z' WHERE memory_id = 'aaaaaaa1'",
            [],
        )
        .expect("backdate last_used");

    record_with_provenance(&mut db, "q-20260610-aabbccd2", &["aaaaaaa1".into()], &[])
        .expect("second record");
    let (used, last): (i64, String) = db
        .conn()
        .query_row(
            "SELECT used_count, last_used FROM feedback WHERE memory_id = 'aaaaaaa1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row after conflict");
    assert_eq!(
        used, 2,
        "ON CONFLICT bumps used_count rather than resetting it"
    );
    assert!(
        last.as_str() > "2000-01-01T00:00:00Z",
        "ON CONFLICT refreshes last_used, got {last}"
    );
}
