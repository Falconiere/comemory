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
use comemory::store::connection;
use comemory::store::feedback::{event_counts, used_events_for_golden, used_query_ids};
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

/// `used_query_ids` returns only `used`-verdict, `target_kind`-matching
/// query ids, deduplicated — the scan behind `eval::mine`.
#[test]
fn used_query_ids_filters_verdict_and_target_kind() {
    let dir = TempDir::new().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind) VALUES
           ('q1', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory'),
           ('q1', 'aaaaaaa2', 'used', '2026-07-15T00:00:01Z', 'memory'),
           ('q2', 'aaaaaaa3', 'irrelevant', '2026-07-15T00:00:00Z', 'memory'),
           ('q3', '1', 'used', '2026-07-15T00:00:00Z', 'code');",
    )
    .expect("seed feedback_events");

    let ids = used_query_ids(&conn, "memory").expect("query");
    assert_eq!(
        ids,
        vec!["q1".to_string()],
        "dedup + verdict + target_kind filter"
    );
}

/// `used_events_for_golden` joins `feedback_events` to its originating
/// `retrieval_log` row, drops the excluded source, and only counts live
/// memories.
#[test]
fn used_events_for_golden_excludes_source_and_dead_memories() {
    let dir = TempDir::new().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash, deleted_at) VALUES
           ('aaaaaaa1','a','note',NULL,'f',3,1,'h1','b','2026-07-15T00:00:00Z',
            '2026-07-15T00:00:00Z','a',0,NULL),
           ('aaaaaaa2','a','note',NULL,'f',3,1,'h2','b','2026-07-15T00:00:00Z',
            '2026-07-15T00:00:00Z','a',0,'2026-07-16T00:00:00Z');
         INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, repo, kind, source) VALUES
           ('q1', 'find x', '[]', '2026-07-15T00:00:00Z', 1, 'r', 'note', 'search'),
           ('q2', 'find y', '[]', '2026-07-15T00:00:00Z', 1, 'r', 'note', 'search-code');
         INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind) VALUES
           ('q1', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory'),
           ('q1', 'aaaaaaa2', 'used', '2026-07-15T00:00:00Z', 'memory'),
           ('q2', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory');",
    )
    .expect("seed rows");

    let rows = used_events_for_golden(&conn, "memory", "search-code").expect("query");
    assert_eq!(
        rows.len(),
        1,
        "dead memory and excluded source both drop out"
    );
    assert_eq!(rows[0].query, "find x");
    assert_eq!(rows[0].repo, Some("r".to_string()));
    assert_eq!(rows[0].kind, Some("note".to_string()));
    assert_eq!(rows[0].memory_id, "aaaaaaa1");
}

/// `event_counts` sums the whole `feedback_events` table and the
/// `provenance != 'manual'` implicit-share numerator in one scan, reading
/// the `NULL`-on-empty conditional sums back as `0` — behind
/// `api::learning::summary`'s header tiles.
#[test]
fn event_counts_sums_verdicts_and_the_implicit_share_numerator() {
    let dir = TempDir::new().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    assert_eq!(
        event_counts(&conn).expect("event_counts on an empty table"),
        (0, 0, 0, 0)
    );

    conn.execute_batch(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance) VALUES
           ('q1', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory', 'manual'),
           ('q2', 'aaaaaaa2', 'used', '2026-07-15T00:00:00Z', 'memory', 'auto_search_edit'),
           ('q3', 'aaaaaaa3', 'irrelevant', '2026-07-15T00:00:00Z', 'memory', 'manual');",
    )
    .expect("seed feedback_events");

    let (total, implicit, used, irrelevant) = event_counts(&conn).expect("event_counts");
    assert_eq!((total, implicit, used, irrelevant), (3, 1, 2, 1));
}
