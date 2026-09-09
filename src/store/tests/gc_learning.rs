#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/gc_learning.rs`. Pins the retention cutoff
//! boundary directly: `comemory gc` deletes whatever this evicts, so a
//! flipped `<`/`<=` here silently deletes (or keeps) one extra retention
//! window's worth of telemetry every run.

use comemory::store::connection;
use comemory::store::gc_learning::evict_before;

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_retrieval_log(conn: &rusqlite::Connection, query_id: &str, at: &str) {
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at) \
         VALUES (?1, 'q', '[]', ?2)",
        rusqlite::params![query_id, at],
    )
    .expect("seed retrieval_log");
}

fn seed_feedback_event(conn: &rusqlite::Connection, id: i64, at: &str) {
    conn.execute(
        "INSERT INTO feedback_events(id, query_id, memory_id, verdict, at) \
         VALUES (?1, 'q', 'aaaa0001', 'used', ?2)",
        rusqlite::params![id, at],
    )
    .expect("seed feedback_events");
}

#[test]
fn cutoff_boundary_is_exclusive() {
    let (_d, conn) = seed_db();
    // A row exactly AT the cutoff must survive (strict `<`); one just past
    // it (a second earlier) must be evicted.
    let cutoff = "2026-01-01T00:00:00.000000000Z";
    seed_retrieval_log(&conn, "at-cutoff", cutoff);
    seed_retrieval_log(&conn, "past-cutoff", "2025-12-31T23:59:59.000000000Z");
    seed_feedback_event(&conn, 1, cutoff);
    seed_feedback_event(&conn, 2, "2025-12-31T23:59:59.000000000Z");

    let (logs, events) = evict_before(&conn, cutoff).expect("evict");
    assert_eq!(
        logs, 1,
        "only the row strictly before the cutoff is evicted"
    );
    assert_eq!(
        events, 1,
        "only the row strictly before the cutoff is evicted"
    );

    let remaining_log: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count");
    assert_eq!(remaining_log, 1, "the at-cutoff row must survive");
    let remaining_events: i64 = conn
        .query_row("SELECT COUNT(*) FROM feedback_events", [], |r| r.get(0))
        .expect("count");
    assert_eq!(remaining_events, 1, "the at-cutoff row must survive");
}

#[test]
fn no_rows_past_cutoff_is_a_no_op() {
    let (_d, conn) = seed_db();
    let cutoff = "2026-01-01T00:00:00.000000000Z";
    seed_retrieval_log(&conn, "future", "2026-06-01T00:00:00.000000000Z");
    seed_feedback_event(&conn, 1, "2026-06-01T00:00:00.000000000Z");

    let (logs, events) = evict_before(&conn, cutoff).expect("evict");
    assert_eq!(logs, 0);
    assert_eq!(events, 0);
}
