#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration coverage for `src/store/feedback.rs` — the `feedback` /
//! memory-tagged `feedback_events` CRUD moved out of `domains::learning::feedback_tracking`.
//! Driven through the real [`StatsDb`] + `record_with_provenance` path
//! (the domain writer that still owns the transaction boundary) rather
//! than calling the `pub(crate)` store helpers with a bare connection, so
//! the test exercises the same integration path the pre-move code took.

use comemory::config::paths::Paths;
use comemory::domains::learning::feedback_tracking::{
    record_implicit_used, record_with_provenance,
};
use comemory::domains::learning::telemetry::StatsDb;
use comemory::store::connection;
use comemory::store::feedback::{
    event_counts, events_for_query, events_since, used_events_for_golden, used_query_ids,
};
use comemory::store::retrieval_log::{self, NewLogRow};
use comemory::utilities::telemetry::{COACTIVATION_QUERY_ID, PROV_AUTO_COACTIVATION, PROV_MANUAL};
use tempfile::TempDir;

const REPO: &str = "r";
const OTHER_REPO: &str = "other-repo";

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
        PROV_MANUAL,
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

    let (target_kind, provenance): (String, String) = conn
        .query_row(
            "SELECT target_kind, provenance FROM feedback_events WHERE memory_id = 'aaaaaaa2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("event row");
    assert_eq!(
        target_kind, "memory",
        "the store helper writes the caller's target_kind verbatim"
    );
    assert_eq!(
        provenance, PROV_MANUAL,
        "the store helper writes the caller's provenance verbatim"
    );
}

#[test]
fn conflict_bumps_used_count_and_refreshes_last_used() {
    let (mut db, _tmp) = open_db();
    record_with_provenance(
        &mut db,
        "q-20260610-aabbccd1",
        &["aaaaaaa1".into()],
        &[],
        PROV_MANUAL,
    )
    .expect("first record");
    db.conn()
        .execute(
            "UPDATE feedback SET last_used = '2000-01-01T00:00:00Z' WHERE memory_id = 'aaaaaaa1'",
            [],
        )
        .expect("backdate last_used");

    record_with_provenance(
        &mut db,
        "q-20260610-aabbccd2",
        &["aaaaaaa1".into()],
        &[],
        PROV_MANUAL,
    )
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

/// `used_query_ids` returns only `used`-verdict, `target_kind`- and
/// `provenance`-matching query ids, deduplicated — the scan behind
/// `eval::mine`. `q4`'s only `used` is HTTP-implicit (#130) and must not
/// mark that query succeeded.
#[test]
fn used_query_ids_filters_verdict_target_kind_and_provenance() {
    let dir = TempDir::new().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance) VALUES
           ('q1', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory', 'manual'),
           ('q1', 'aaaaaaa2', 'used', '2026-07-15T00:00:01Z', 'memory', 'manual'),
           ('q2', 'aaaaaaa3', 'irrelevant', '2026-07-15T00:00:00Z', 'memory', 'manual'),
           ('q3', '1', 'used', '2026-07-15T00:00:00Z', 'code', 'manual'),
           ('q4', 'aaaaaaa4', 'used', '2026-07-15T00:00:00Z', 'memory', 'implicit');",
    )
    .expect("seed feedback_events");

    let ids = used_query_ids(&conn, "memory", PROV_MANUAL).expect("query");
    assert_eq!(
        ids,
        vec!["q1".to_string()],
        "dedup + verdict + target_kind + provenance filter"
    );
    assert_eq!(
        used_query_ids(&conn, "memory", "implicit").expect("query"),
        vec!["q4".to_string()],
        "the provenance parameter is a filter, not a hardcoded 'manual'"
    );
}

/// `used_events_for_golden` joins `feedback_events` to its originating
/// `retrieval_log` row, drops the excluded source, only counts live
/// memories, and (#130) only the requested provenance — `aaaaaaa3` is a
/// live memory marked `used` on the harvestable query, but implicitly.
#[test]
fn used_events_for_golden_excludes_source_dead_memories_and_other_provenance() {
    let dir = TempDir::new().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    conn.execute_batch(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash, deleted_at) VALUES
           ('aaaaaaa1','a','note',NULL,'f',3,1,'h1','b','2026-07-15T00:00:00Z',
            '2026-07-15T00:00:00Z','a',0,NULL),
           ('aaaaaaa2','a','note',NULL,'f',3,1,'h2','b','2026-07-15T00:00:00Z',
            '2026-07-15T00:00:00Z','a',0,'2026-07-16T00:00:00Z'),
           ('aaaaaaa3','a','note',NULL,'f',3,1,'h3','b','2026-07-15T00:00:00Z',
            '2026-07-15T00:00:00Z','a',0,NULL);
         INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, repo, kind, source) VALUES
           ('q1', 'find x', '[]', '2026-07-15T00:00:00Z', 1, 'r', 'note', 'search'),
           ('q2', 'find y', '[]', '2026-07-15T00:00:00Z', 1, 'r', 'note', 'search-code');
         INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind, provenance) VALUES
           ('q1', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory', 'manual'),
           ('q1', 'aaaaaaa2', 'used', '2026-07-15T00:00:00Z', 'memory', 'manual'),
           ('q1', 'aaaaaaa3', 'used', '2026-07-15T00:00:00Z', 'memory', 'implicit'),
           ('q2', 'aaaaaaa1', 'used', '2026-07-15T00:00:00Z', 'memory', 'manual');",
    )
    .expect("seed rows");

    let rows = used_events_for_golden(&conn, "memory", "search-code", PROV_MANUAL).expect("query");
    assert_eq!(
        rows.len(),
        1,
        "dead memory, excluded source, and implicit provenance all drop out"
    );
    assert_eq!(rows[0].query, "find x");
    assert_eq!(rows[0].repo, Some("r".to_string()));
    assert_eq!(rows[0].kind, Some("note".to_string()));
    assert_eq!(rows[0].memory_id, "aaaaaaa1");
}

/// `event_counts` sums the whole `feedback_events` table and the
/// `provenance != 'manual'` implicit-share numerator in one scan, reading
/// the `NULL`-on-empty conditional sums back as `0` — behind
/// `domains::learning::console::summary`'s header tiles.
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

/// `events_since` counts `feedback_events` at or after `since`, scoped by
/// the originating `retrieval_log` row's `repo` when one is given, and `0`
/// once `since` moves past the recorded verdict — the verdict leg of
/// `domains::learning::recall_status`'s window report.
#[test]
fn events_since_counts_by_window_and_repo() {
    let (mut db, _tmp) = open_db();
    retrieval_log::insert(
        db.conn(),
        &NewLogRow {
            query_id: "q-1",
            query: "q",
            returned_ids: "[]",
            at: "2026-07-15T00:00:00Z",
            duration_ms: 1,
            repo: Some(REPO),
            kind: None,
            source: "search",
        },
    )
    .expect("insert retrieval_log row");

    assert_eq!(
        events_since(db.conn(), Some(REPO), "2026-07-14T00:00:00Z").expect("count"),
        0,
        "no feedback_events row yet"
    );

    record_with_provenance(&mut db, "q-1", &["aaaaaaa1".to_string()], &[], PROV_MANUAL)
        .expect("record verdict");

    assert_eq!(
        events_since(db.conn(), Some(REPO), "2026-07-14T00:00:00Z").expect("count"),
        1
    );
    assert_eq!(
        events_since(db.conn(), None, "2026-07-14T00:00:00Z").expect("count"),
        1,
        "no repo filter counts every repo"
    );
    assert_eq!(
        events_since(db.conn(), Some(OTHER_REPO), "2026-07-14T00:00:00Z").expect("count"),
        0,
        "a different repo excludes the verdict"
    );
    assert_eq!(
        events_since(db.conn(), Some(REPO), "2200-01-01T00:00:00Z").expect("count"),
        0,
        "a since in the future excludes the verdict"
    );
}

/// A co-activation reward (the real sentinel path: `record_implicit_used`
/// with [`PROV_AUTO_COACTIVATION`] and [`COACTIVATION_QUERY_ID`], the same
/// call `domains::graph::coactivate::reward_pair` makes) writes a
/// `feedback_events` row with NO matching `retrieval_log` row by design
/// (`COACTIVATION_QUERY_ID` is deliberately not a real query id). The
/// `LEFT JOIN` in `events_since` must still count it when `repo` is `None`,
/// and exclude it when `repo` is `Some` — there is no repo to match.
#[test]
fn events_since_counts_a_coactivation_reward_with_no_retrieval_log_row_only_when_repo_is_none() {
    let (db, _tmp) = open_db();
    record_implicit_used(
        db.conn(),
        "aaaaaaa1",
        "2026-07-15T00:00:00Z",
        PROV_AUTO_COACTIVATION,
        COACTIVATION_QUERY_ID,
    )
    .expect("record co-activation reward");

    assert_eq!(
        events_since(db.conn(), None, "2026-07-14T00:00:00Z").expect("count"),
        1,
        "no repo filter counts the sentinel row despite its missing retrieval_log join"
    );
    assert_eq!(
        events_since(db.conn(), Some(REPO), "2026-07-14T00:00:00Z").expect("count"),
        0,
        "a repo filter excludes it: the LEFT JOIN has no repo to match"
    );
}

/// `events_for_query` reads back exactly what landed against one query id —
/// id, verdict and provenance — over two real `record_with_provenance`
/// calls with DIFFERENT provenance, so a mapping that collapsed the two
/// would fail here. The rows of a second query id must not leak in.
#[test]
fn events_for_query_reads_back_each_verdict_with_its_provenance() {
    let (mut db, _tmp) = open_db();
    record_with_provenance(
        &mut db,
        "q-20260918-aabbccdd",
        &["aaaaaaa1".into()],
        &["aaaaaaa2".into()],
        "implicit",
    )
    .expect("implicit record");
    record_with_provenance(
        &mut db,
        "q-20260918-aabbccdd",
        &["aaaaaaa3".into()],
        &[],
        PROV_MANUAL,
    )
    .expect("manual record");
    record_with_provenance(
        &mut db,
        "q-20260918-11223344",
        &["aaaaaaa9".into()],
        &[],
        PROV_MANUAL,
    )
    .expect("other query record");

    let rows = events_for_query(db.conn(), "q-20260918-aabbccdd").expect("events for query");
    let shape: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r.memory_id.as_str(),
                r.verdict.as_str(),
                r.provenance.as_str(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("aaaaaaa1", "used", "implicit"),
            ("aaaaaaa2", "irrelevant", "implicit"),
            ("aaaaaaa3", "used", PROV_MANUAL),
        ],
        "ordered (memory_id, verdict), one row per verdict, provenance verbatim"
    );

    assert!(
        events_for_query(db.conn(), "q-20260918-00000000")
            .expect("events for an unjudged query")
            .is_empty(),
        "a query with no verdicts reads back empty, not an error"
    );
}
