//! Tests for [`comemory::eval::mine`] — reformulation mining over
//! `retrieval_log` + the rebuild-not-increment `apply` semantics.

use comemory::eval::mine::{MinedMapping, apply, mine};
use comemory::store::connection;

/// Open a real migrated db in a tempdir.
fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("c.db")).expect("open");
    (dir, conn)
}

/// Insert a `retrieval_log` row with the given id, query text, and time
/// (the v6 `source` column keeps its `'search'` default).
fn log_query(conn: &rusqlite::Connection, qid: &str, query: &str, at: &str) {
    log_query_src(conn, qid, query, at, "search");
}

/// Insert a `retrieval_log` row with an explicit `source` value.
fn log_query_src(conn: &rusqlite::Connection, qid: &str, query: &str, at: &str, source: &str) {
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, source) \
         VALUES (?1, ?2, '[]', ?3, ?4)",
        rusqlite::params![qid, query, at, source],
    )
    .expect("insert retrieval_log row");
}

/// Insert a `used` feedback event for the given query id.
fn mark_used(conn: &rusqlite::Connection, qid: &str) {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at) \
         VALUES (?1, 'aaaa0001', 'used', '2026-06-09T10:30:00Z')",
        [qid],
    )
    .expect("insert feedback event");
}

/// Insert a `used` feedback event tagged `target_kind='code'` for the given
/// query id (symbol id text-encoded into the memory_id column, as
/// `stats::code_feedback` writes it).
fn mark_used_code(conn: &rusqlite::Connection, qid: &str) {
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind) \
         VALUES (?1, '42', 'used', '2026-06-09T10:30:00Z', 'code')",
        [qid],
    )
    .expect("insert code feedback event");
}

#[test]
fn mine_distills_term_diff_mappings_from_reformulation_pair() {
    // Worked example (same tokenizer as the impl — identifier split,
    // non-colocated parts only, >1 char):
    //   q1 "embedding size error" (NO used feedback)
    //     tokens = {embedding, size, error}
    //   q2 "VecDimMismatch error" at T+5min (one used feedback row)
    //     non-colocated tokens = {vec, dim, mismatch, error}
    //   shared = {error} (>= 1, same-intent guard passes)
    //   failed = q1 \ q2 = {embedding, size}
    //   fix    = q2 \ q1 = {vec, dim, mismatch}
    //   expected mappings = failed x fix = 6, each with support 1.
    //   q3 "unrelated banana" at T+6min (used feedback) shares zero
    //   tokens with q1, so the q1->q3 pair yields nothing; q2 earned
    //   used feedback so it is never a failed-query candidate itself.
    let (_d, conn) = open_db();
    log_query(
        &conn,
        "q-20260609-00000001",
        "embedding size error",
        "2026-06-09T10:00:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "VecDimMismatch error",
        "2026-06-09T10:05:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000003",
        "unrelated banana",
        "2026-06-09T10:06:00Z",
    );
    mark_used(&conn, "q-20260609-00000002");
    mark_used(&conn, "q-20260609-00000003");

    let mined = mine(&conn).expect("mine");
    let expected: Vec<MinedMapping> = [
        ("embedding", "dim"),
        ("embedding", "mismatch"),
        ("embedding", "vec"),
        ("size", "dim"),
        ("size", "mismatch"),
        ("size", "vec"),
    ]
    .iter()
    .map(|(t, e)| MinedMapping {
        term: (*t).into(),
        expansion: (*e).into(),
        support: 1,
    })
    .collect();
    assert_eq!(
        mined, expected,
        "exactly the 6 failed x fix mappings, sorted"
    );
}

#[test]
fn mine_yields_nothing_when_failed_diff_is_empty() {
    // q1 "dim mismatch" tokens = {dim, mismatch}; q2 "VecDimMismatch"
    // non-colocated parts = {vec, dim, mismatch}. Shared {dim, mismatch}
    // passes the intent guard, but failed = q1 \ q2 = {} — the rewording
    // only *added* tokens, so there is no failed term to map from.
    let (_d, conn) = open_db();
    log_query(
        &conn,
        "q-20260609-00000001",
        "dim mismatch",
        "2026-06-09T10:00:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "VecDimMismatch",
        "2026-06-09T10:05:00Z",
    );
    mark_used(&conn, "q-20260609-00000002");

    let mined = mine(&conn).expect("mine");
    assert!(
        mined.is_empty(),
        "empty failed-diff must yield nothing: {mined:?}"
    );
}

#[test]
fn mine_ignores_pairs_outside_the_reformulation_window() {
    // Same pair as the worked example but 20 minutes apart — outside the
    // 10-minute reformulation window, so it is not a rewording.
    let (_d, conn) = open_db();
    log_query(
        &conn,
        "q-20260609-00000001",
        "embedding size error",
        "2026-06-09T10:00:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "VecDimMismatch error",
        "2026-06-09T10:20:00Z",
    );
    mark_used(&conn, "q-20260609-00000002");

    let mined = mine(&conn).expect("mine");
    assert!(
        mined.is_empty(),
        "20-minute gap must yield nothing: {mined:?}"
    );
}

#[test]
fn mine_aggregates_support_across_repeated_pairs() {
    // The same reformulation observed twice (two disjoint pairs inside
    // their own windows) must aggregate to support 2, not two rows.
    let (_d, conn) = open_db();
    log_query(
        &conn,
        "q-20260609-00000001",
        "sizing error",
        "2026-06-09T10:00:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "mismatch error",
        "2026-06-09T10:04:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000003",
        "sizing error",
        "2026-06-09T11:00:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000004",
        "mismatch error",
        "2026-06-09T11:04:00Z",
    );
    mark_used(&conn, "q-20260609-00000002");
    mark_used(&conn, "q-20260609-00000004");

    let mined = mine(&conn).expect("mine");
    assert_eq!(
        mined,
        vec![MinedMapping {
            term: "sizing".into(),
            expansion: "mismatch".into(),
            support: 2,
        }]
    );
}

#[test]
fn mine_ignores_failed_code_search_rows() {
    // Same shape as the worked example, but the failed q1 came from
    // `comemory search-code` (source='search-code'). Code-search rows can
    // only ever receive code-target feedback, so without memory verdicts
    // they would read as permanently failed — mining must skip them
    // entirely instead of minting spurious expansions.
    let (_d, conn) = open_db();
    log_query_src(
        &conn,
        "q-20260609-00000001",
        "embedding size error",
        "2026-06-09T10:00:00Z",
        "search-code",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "VecDimMismatch error",
        "2026-06-09T10:05:00Z",
    );
    mark_used(&conn, "q-20260609-00000002");

    let mined = mine(&conn).expect("mine");
    assert!(
        mined.is_empty(),
        "search-code rows must not seed mining: {mined:?}"
    );
}

#[test]
fn mine_ignores_code_target_used_feedback_on_the_follow_up() {
    // Failed q1 + a follow-up q2 whose ONLY used feedback is code-target:
    // a code verdict says nothing about memory retrieval quality, so q2 is
    // not a successful rewording and the pair must mint no mappings.
    let (_d, conn) = open_db();
    log_query(
        &conn,
        "q-20260609-00000001",
        "sizing error",
        "2026-06-09T10:00:00Z",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "mismatch error",
        "2026-06-09T10:04:00Z",
    );
    mark_used_code(&conn, "q-20260609-00000002");

    let mined = mine(&conn).expect("mine");
    assert!(
        mined.is_empty(),
        "code-only used feedback must not mark q2 successful: {mined:?}"
    );
}

#[test]
fn mine_includes_failed_context_rows() {
    // Context lookups are first-class mining citizens (they carry
    // query_ids and can receive feedback since M2): a failed
    // source='context' row pairs with a later used-feedback query
    // exactly like a failed search row.
    let (_d, conn) = open_db();
    log_query_src(
        &conn,
        "q-20260609-00000001",
        "sizing error",
        "2026-06-09T10:00:00Z",
        "context",
    );
    log_query(
        &conn,
        "q-20260609-00000002",
        "mismatch error",
        "2026-06-09T10:04:00Z",
    );
    mark_used(&conn, "q-20260609-00000002");

    let mined = mine(&conn).expect("mine");
    assert_eq!(
        mined,
        vec![MinedMapping {
            term: "sizing".into(),
            expansion: "mismatch".into(),
            support: 1,
        }],
        "context rows must still participate in mining"
    );
}

#[test]
fn apply_rebuilds_the_table_instead_of_accumulating() {
    let (_d, mut conn) = open_db();
    let first = vec![
        MinedMapping {
            term: "sizing".into(),
            expansion: "mismatch".into(),
            support: 2,
        },
        MinedMapping {
            term: "sizing".into(),
            expansion: "vec".into(),
            support: 1,
        },
    ];
    apply(&mut conn, &first, "2026-06-09T12:00:00Z").expect("first apply");

    let second = vec![MinedMapping {
        term: "embedding".into(),
        expansion: "dim".into(),
        support: 3,
    }];
    apply(&mut conn, &second, "2026-06-09T13:00:00Z").expect("second apply");

    let rows: Vec<(String, String, i64, String)> = conn
        .prepare("SELECT term, expansion, support, last_mined FROM query_expansions")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        rows,
        vec![(
            "embedding".into(),
            "dim".into(),
            3,
            "2026-06-09T13:00:00Z".into()
        )],
        "second apply must fully replace the first set"
    );
}
