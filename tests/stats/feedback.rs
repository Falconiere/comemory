//! Tests for [`comemory::stats::feedback`].
//!
//! v0.2: feedback rows land in `comemory.db` (via `StatsDb::open` which
//! now delegates to `crate::store::connection::open`). Counter upsert
//! semantics (first insert → 1, conflict → +1, last_used refresh) are
//! exercised through `record_with_provenance`, the only src/ writer.

use comemory::config::paths::Paths;
use comemory::stats::feedback::{generate_query_id, is_valid_query_id, record_with_provenance};
use comemory::stats::sqlite::StatsDb;

use super::common;

/// Open a [`StatsDb`] in a fresh sandbox, returning the guard with it.
fn open_db() -> (common::runner::Sandbox, StatsDb) {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let db = StatsDb::open(paths.stats_db()).expect("open");
    (sb, db)
}

#[test]
fn used_counter_inserts_then_increments_and_refreshes_last_used() {
    let (_sb, mut db) = open_db();
    record_with_provenance(&mut db, "q-20260610-aabbccd1", &["aaaaaaa1".into()], &[])
        .expect("first record");
    let (used, last): (i64, String) = db
        .conn()
        .query_row(
            "SELECT used_count, last_used FROM feedback WHERE memory_id='aaaaaaa1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row after insert");
    assert_eq!(used, 1, "first insert seeds used_count = 1");
    assert!(!last.is_empty(), "insert sets last_used");

    // Backdate last_used so the conflict path's refresh is observable
    // without sleeping between the two records.
    db.conn()
        .execute(
            "UPDATE feedback SET last_used='2000-01-01T00:00:00Z' WHERE memory_id='aaaaaaa1'",
            [],
        )
        .expect("backdate last_used");
    record_with_provenance(&mut db, "q-20260610-aabbccd2", &["aaaaaaa1".into()], &[])
        .expect("second record");
    let (used, last): (i64, String) = db
        .conn()
        .query_row(
            "SELECT used_count, last_used FROM feedback WHERE memory_id='aaaaaaa1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row after conflict");
    assert_eq!(used, 2, "conflict bumps used_count");
    assert!(
        last.as_str() > "2000-01-01T00:00:00Z",
        "conflict refreshes last_used, got {last}"
    );
}

#[test]
fn irrelevant_counter_inserts_then_increments_without_touching_last_used() {
    let (_sb, mut db) = open_db();
    for qid in ["q-20260610-aabbccd1", "q-20260610-aabbccd2"] {
        record_with_provenance(&mut db, qid, &[], &["aaaaaaa2".into()]).expect("record");
    }
    let (used, irrelevant, last): (i64, i64, Option<String>) = db
        .conn()
        .query_row(
            "SELECT used_count, irrelevant_count, last_used FROM feedback \
              WHERE memory_id='aaaaaaa2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row");
    assert_eq!(used, 0);
    assert_eq!(irrelevant, 2, "insert seeds 1, conflict bumps to 2");
    assert!(last.is_none(), "a dismissal is not a use");
}

#[test]
fn record_with_provenance_writes_events_and_counters_atomically() {
    let (_sb, mut db) = open_db();
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
    let (verdict, target_kind): (String, String) = conn
        .query_row(
            "SELECT verdict, target_kind FROM feedback_events WHERE memory_id='aaaaaaa2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("verdict");
    assert_eq!(verdict, "irrelevant");
    assert_eq!(
        target_kind, "memory",
        "memory-side events must be explicitly tagged target_kind='memory'"
    );
}

#[test]
fn record_with_provenance_errors_on_schema_drift() {
    // Schema drift: if the `feedback` table is missing entirely, the write
    // must surface the SQLite error rather than swallow it, and the
    // all-or-nothing transaction must leave no event row behind.
    let (_sb, mut db) = open_db();
    db.conn()
        .execute("DROP TABLE feedback", [])
        .expect("drop feedback table");

    let err = record_with_provenance(&mut db, "q-20260610-aabbccdd", &["aaaaaaa1".into()], &[])
        .expect_err("record must error when feedback table is missing");
    let msg = err.to_string();
    assert!(
        msg.contains("feedback"),
        "error should mention 'feedback', got: {msg}"
    );
    let events: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM feedback_events", [], |r| r.get(0))
        .expect("count events");
    assert_eq!(events, 0, "failed batch must not leave a partial event row");
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
fn generated_query_id_always_validates() {
    // Round-trip of the writer/checker contract: every id the generator
    // emits must pass the validator, for both a fixed and a live clock.
    let fixed = time::macros::datetime!(2026-06-10 12:34:56.789 UTC);
    for query in ["", "sqlite busy", "Café VecDimMismatch \"quoted\""] {
        let id = generate_query_id(query, fixed);
        assert!(is_valid_query_id(&id), "generated id must validate: {id}");
        assert!(id.starts_with("q-20260610-"), "day-sortable prefix: {id}");
    }
    let live = generate_query_id("any query", time::OffsetDateTime::now_utc());
    assert!(
        is_valid_query_id(&live),
        "live-clock id must validate: {live}"
    );
}
