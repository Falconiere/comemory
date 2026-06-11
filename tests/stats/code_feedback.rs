//! Tests for [`comemory::stats::code_feedback`].
//!
//! Code-side sibling of `tests/stats/feedback.rs`: counter upsert semantics
//! (first insert → 1, conflict → +1, last_used refresh) are exercised
//! through `record_code_with_provenance`, the only src/ writer. Provenance
//! rows land in `feedback_events` with `target_kind = 'code'` and the
//! symbol id text-encoded into the `memory_id` column — the column name is
//! a memory-era wart the writer documents rather than hides.

use comemory::config::paths::Paths;
use comemory::stats::code_feedback::record_code_with_provenance;
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
    record_code_with_provenance(&mut db, "q-20260610-aabbccd1", &[42], &[]).expect("first record");
    let (used, last): (i64, String) = db
        .conn()
        .query_row(
            "SELECT used_count, last_used FROM code_feedback WHERE symbol_id = 42",
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
            "UPDATE code_feedback SET last_used = '2000-01-01T00:00:00Z' WHERE symbol_id = 42",
            [],
        )
        .expect("backdate last_used");
    record_code_with_provenance(&mut db, "q-20260610-aabbccd2", &[42], &[]).expect("second record");
    let (used, last): (i64, String) = db
        .conn()
        .query_row(
            "SELECT used_count, last_used FROM code_feedback WHERE symbol_id = 42",
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
        record_code_with_provenance(&mut db, qid, &[], &[43]).expect("record");
    }
    let (used, irrelevant, last): (i64, i64, Option<String>) = db
        .conn()
        .query_row(
            "SELECT used_count, irrelevant_count, last_used FROM code_feedback \
              WHERE symbol_id = 43",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row");
    assert_eq!(used, 0);
    assert_eq!(irrelevant, 2, "insert seeds 1, conflict bumps to 2");
    assert!(last.is_none(), "a dismissal is not a use");
}

#[test]
fn record_code_with_provenance_writes_code_tagged_events_and_counters() {
    let (_sb, mut db) = open_db();
    record_code_with_provenance(&mut db, "q-20260610-aabbccdd", &[42], &[43]).expect("record");

    let conn = db.conn();
    let events: i64 = conn
        .query_row(
            "SELECT count(*) FROM feedback_events \
              WHERE query_id = 'q-20260610-aabbccdd' AND target_kind = 'code'",
            [],
            |r| r.get(0),
        )
        .expect("events");
    assert_eq!(events, 2, "every code event must carry target_kind='code'");
    // The symbol id is text-encoded into the memory_id column (the
    // documented column-name wart): symbol 42 → memory_id '42'.
    let verdict: String = conn
        .query_row(
            "SELECT verdict FROM feedback_events WHERE memory_id = '42'",
            [],
            |r| r.get(0),
        )
        .expect("used verdict");
    assert_eq!(verdict, "used");
    let verdict: String = conn
        .query_row(
            "SELECT verdict FROM feedback_events WHERE memory_id = '43'",
            [],
            |r| r.get(0),
        )
        .expect("irrelevant verdict");
    assert_eq!(verdict, "irrelevant");
    let used: i64 = conn
        .query_row(
            "SELECT used_count FROM code_feedback WHERE symbol_id = 42",
            [],
            |r| r.get(0),
        )
        .expect("used counter");
    assert_eq!(used, 1);
    // The memory-side counter table must stay untouched by code feedback.
    let memory_rows: i64 = conn
        .query_row("SELECT count(*) FROM feedback", [], |r| r.get(0))
        .expect("memory feedback rows");
    assert_eq!(memory_rows, 0, "code feedback must not touch `feedback`");
}

#[test]
fn record_code_with_provenance_errors_on_schema_drift() {
    // Schema drift: if the `code_feedback` table is missing entirely, the
    // write must surface the SQLite error rather than swallow it, and the
    // all-or-nothing transaction must leave no event row behind.
    let (_sb, mut db) = open_db();
    db.conn()
        .execute("DROP TABLE code_feedback", [])
        .expect("drop code_feedback table");

    let err = record_code_with_provenance(&mut db, "q-20260610-aabbccdd", &[42], &[])
        .expect_err("record must error when code_feedback table is missing");
    let msg = err.to_string();
    assert!(
        msg.contains("code_feedback"),
        "error should mention 'code_feedback', got: {msg}"
    );
    let events: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM feedback_events", [], |r| r.get(0))
        .expect("count events");
    assert_eq!(events, 0, "failed batch must not leave a partial event row");
}
