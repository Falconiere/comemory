//! Tests for [`comemory::stats::feedback::Feedback`].
//!
//! v0.2: feedback rows land in `comemory.db` (via `StatsDb::open` which
//! now delegates to `crate::store::connection::open`).

use comemory::config::paths::Paths;
use comemory::stats::feedback::{is_valid_query_id, record_with_provenance, Feedback};
use comemory::stats::sqlite::StatsDb;

use super::common;

#[test]
fn record_used_increments_count() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let mut db = StatsDb::open(paths.stats_db()).expect("open");
    let fb = Feedback::new(&mut db);
    fb.record_used("m1").expect("record_used 1");
    fb.record_used("m1").expect("record_used 2");
    let (used, irrelevant) = fb.counts("m1").expect("counts");
    assert_eq!(used, 2);
    assert_eq!(irrelevant, 0);
}

#[test]
fn record_irrelevant_increments_count() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let mut db = StatsDb::open(paths.stats_db()).expect("open");
    let fb = Feedback::new(&mut db);
    fb.record_irrelevant("m2").expect("record_irrelevant");
    let (used, irrelevant) = fb.counts("m2").expect("counts");
    assert_eq!(used, 0);
    assert_eq!(irrelevant, 1);
}

#[test]
fn record_with_provenance_writes_events_and_counters_atomically() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let mut db = StatsDb::open(paths.stats_db()).expect("open");
    record_with_provenance(
        &mut db,
        "q-20260610-aabbccdd",
        &["aaaaaaa1".into()],
        &["aaaaaaa2".into()],
    )
    .expect("record");

    let conn = db.conn();
    let events: i64 = conn
        .query_row(
            "SELECT count(*) FROM feedback_events WHERE query_id='q-20260610-aabbccdd'",
            [],
            |r| r.get(0),
        )
        .expect("events");
    assert_eq!(events, 2);
    let used: i64 = conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id='aaaaaaa1'",
            [],
            |r| r.get(0),
        )
        .expect("used");
    assert_eq!(used, 1);
    let verdict: String = conn
        .query_row(
            "SELECT verdict FROM feedback_events WHERE memory_id='aaaaaaa2'",
            [],
            |r| r.get(0),
        )
        .expect("verdict");
    assert_eq!(verdict, "irrelevant");
}

#[test]
fn valid_query_id_shape_is_accepted() {
    assert!(is_valid_query_id("q-20260610-a1b2c3d4"));
}

#[test]
fn malformed_query_ids_are_rejected() {
    let bad = [
        "",                     // empty
        "q-2026061-a1b2c3d4",   // 7-digit date
        "q-20260610-A1B2C3D4",  // uppercase hex
        "q-20260610-a1b2c3",    // short hex
        "x-20260610-a1b2c3d4",  // wrong prefix
        "q-20260610-a1b2c3d4x", // trailing garbage
    ];
    for s in bad {
        assert!(!is_valid_query_id(s), "must reject {s:?}");
    }
}

#[test]
fn counts_returns_error_on_schema_drift() {
    // Schema drift: if the `feedback` table is missing entirely, `counts`
    // must surface the SQLite error rather than swallow it as (0, 0). The
    // previous implementation used `unwrap_or((0, 0))` which masked any
    // failure mode that wasn't `QueryReturnedNoRows`.
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let mut db = StatsDb::open(paths.stats_db()).expect("open");
    db.conn()
        .execute("DROP TABLE feedback", [])
        .expect("drop feedback table");

    let fb = Feedback::new(&mut db);
    let result = fb.counts("anything");
    let err = result.expect_err("counts must error when feedback table is missing");
    let msg = err.to_string();
    assert!(
        msg.contains("feedback"),
        "error should mention 'feedback', got: {msg}"
    );
}
