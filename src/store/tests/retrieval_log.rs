#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/retrieval_log.rs` — real `retrieval_log` rows
//! seeded on a real `comemory.db`, asserting the window/source/repo filter,
//! that a malformed `returned_ids` value is returned raw (unparsed), and
//! that [`insert`] writes every column back readably.

use comemory::config::paths::Paths;
use comemory::domains::learning::feedback_tracking::record_with_provenance;
use comemory::domains::learning::telemetry::StatsDb;
use comemory::store::connection;
use comemory::store::retrieval_log::{
    NewLogRow, count_since, distinct_prefix_matches, insert, pending_since, prefix_matches,
    queries_excluding_source, returned_ids_in_window,
};
use comemory::utilities::telemetry::{PROV_MANUAL, source};
use rusqlite::Connection;
use tempfile::{TempDir, tempdir};

const REPO: &str = "r";
const OTHER_REPO: &str = "other-repo";

fn seed_db() -> Connection {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    connection::open(&path).expect("open")
}

fn insert_row(conn: &Connection, query_id: &str, returned_ids: &str, at: &str, source: &str) {
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, repo, source) \
         VALUES (?1, 'q', ?2, ?3, 1, ?4, ?5)",
        rusqlite::params![query_id, returned_ids, at, REPO, source],
    )
    .expect("insert retrieval_log row");
}

/// Two in-window rows (one `search`, one `context`) plus one out-of-window
/// row and one in-window row with malformed JSON. The window query returns
/// exactly the three in-window rows, raw and unparsed — the malformed one
/// included, since parsing is a caller concern.
#[test]
fn returns_raw_rows_in_window_and_skips_out_of_window() {
    let conn = seed_db();
    insert_row(
        &conn,
        "q-in-search",
        r#"["aaaaaaa1"]"#,
        "2026-07-15T00:00:00Z",
        "search",
    );
    insert_row(
        &conn,
        "q-in-context",
        r#"["aaaaaaa2"]"#,
        "2026-07-16T00:00:00Z",
        "context",
    );
    insert_row(
        &conn,
        "q-out-of-window",
        r#"["aaaaaaa3"]"#,
        "2026-06-01T00:00:00Z",
        "search",
    );
    insert_row(
        &conn,
        "q-malformed",
        "not-json",
        "2026-07-17T00:00:00Z",
        "context",
    );

    let raws = returned_ids_in_window(
        &conn,
        "search",
        "context",
        "2026-07-10T00:00:00Z",
        "2026-07-20T00:00:00Z",
        Some(REPO),
    )
    .expect("query window");

    let mut sorted = raws.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec![
            r#"["aaaaaaa1"]"#.to_string(),
            r#"["aaaaaaa2"]"#.to_string(),
            "not-json".to_string(),
        ]
    );
}

/// A `source` outside `(source_a, source_b)` never matches, even inside the
/// window and repo.
#[test]
fn excludes_rows_outside_the_two_sources() {
    let conn = seed_db();
    insert_row(
        &conn,
        "q-other-source",
        r#"["aaaaaaa9"]"#,
        "2026-07-15T00:00:00Z",
        "mine",
    );

    let raws = returned_ids_in_window(
        &conn,
        "search",
        "context",
        "2026-07-10T00:00:00Z",
        "2026-07-20T00:00:00Z",
        Some(REPO),
    )
    .expect("query window");

    assert!(raws.is_empty());
}

/// `repo IS NULL` rows (unscoped searches) still match any repo filter.
#[test]
fn unscoped_repo_row_matches_any_repo_filter() {
    let conn = seed_db();
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, repo, source) \
         VALUES ('q-unscoped', 'q', '[\"aaaaaaa5\"]', '2026-07-15T00:00:00Z', 1, NULL, 'search')",
        [],
    )
    .expect("insert unscoped row");

    let raws = returned_ids_in_window(
        &conn,
        "search",
        "context",
        "2026-07-10T00:00:00Z",
        "2026-07-20T00:00:00Z",
        Some("some-other-repo"),
    )
    .expect("query window");

    assert_eq!(raws, vec![r#"["aaaaaaa5"]"#.to_string()]);
}

/// [`insert`] writes every column, and the row is readable back through
/// [`returned_ids_in_window`] with the exact JSON text supplied.
#[test]
fn insert_writes_a_row_readable_by_the_window_query() {
    let conn = seed_db();
    insert(
        &conn,
        &NewLogRow {
            query_id: "q-insert",
            query: "hello world",
            returned_ids: r#"["aaaaaaa7"]"#,
            at: "2026-07-15T00:00:00Z",
            duration_ms: 42,
            repo: Some(REPO),
            kind: Some("decision"),
            source: "search",
        },
    )
    .expect("insert");

    let raws = returned_ids_in_window(
        &conn,
        "search",
        "context",
        "2026-07-10T00:00:00Z",
        "2026-07-20T00:00:00Z",
        Some(REPO),
    )
    .expect("query window");
    assert_eq!(raws, vec![r#"["aaaaaaa7"]"#.to_string()]);

    let (query, dur, kind): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT query, duration_ms, kind FROM retrieval_log WHERE query_id = 'q-insert'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("row exists");
    assert_eq!(query, "hello world");
    assert_eq!(dur, 42);
    assert_eq!(kind, Some("decision".to_string()));
}

/// `repo: None` writes a NULL `repo` column, matching an unscoped run.
#[test]
fn insert_with_no_repo_writes_null() {
    let conn = seed_db();
    insert(
        &conn,
        &NewLogRow {
            query_id: "q-unscoped-insert",
            query: "q",
            returned_ids: "[]",
            at: "2026-07-15T00:00:00Z",
            duration_ms: 1,
            repo: None,
            kind: None,
            source: "search",
        },
    )
    .expect("insert");

    let repo: Option<String> = conn
        .query_row(
            "SELECT repo FROM retrieval_log WHERE query_id = 'q-unscoped-insert'",
            [],
            |r| r.get(0),
        )
        .expect("row exists");
    assert_eq!(repo, None);
}

/// `prefix_matches` drops `search-code` rows, matches only the `LIKE`
/// prefix, and orders newest first — the scan behind `retrieval::suggest`'s
/// "recent" list (dedup/limit stay a caller concern).
#[test]
fn prefix_matches_excludes_source_and_orders_newest_first() {
    let conn = seed_db();
    insert_row(&conn, "q-older", "[]", "2026-07-15T00:00:00Z", "search");
    insert_row(&conn, "q-newer", "[]", "2026-07-16T00:00:00Z", "context");
    insert_row(
        &conn,
        "q-excluded-source",
        "[]",
        "2026-07-17T00:00:00Z",
        "search-code",
    );
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, repo, source) \
         VALUES ('q-no-match', 'unrelated', '[]', '2026-07-16T00:00:00Z', 1, ?1, 'search')",
        rusqlite::params![REPO],
    )
    .expect("seed a non-matching query");
    conn.execute(
        "UPDATE retrieval_log SET query = 'front matter' WHERE query_id = 'q-older'",
        [],
    )
    .expect("set query text");
    conn.execute(
        "UPDATE retrieval_log SET query = 'front rules' WHERE query_id = 'q-newer'",
        [],
    )
    .expect("set query text");
    conn.execute(
        "UPDATE retrieval_log SET query = 'front code' WHERE query_id = 'q-excluded-source'",
        [],
    )
    .expect("set query text");

    let rows = prefix_matches(&conn, "search-code", "front%").expect("query");
    let ids: Vec<&str> = rows.iter().map(|r| r.query_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["q-newer", "q-older"],
        "excludes the search-code source and orders newest first"
    );
}

#[test]
fn distinct_prefix_matches_keeps_newest_id_and_unicode_case_dedup() {
    let conn = seed_db();
    for (id, query, at, source) in [
        ("a-old", "Draft Ä", "2026-07-15T00:00:00Z", "search"),
        ("b-new", "draft ä", "2026-07-16T00:00:00Z", "context"),
        ("c-next", "draft beta", "2026-07-16T00:00:00Z", "search"),
        (
            "d-code",
            "draft code",
            "2026-07-17T00:00:00Z",
            "search-code",
        ),
    ] {
        conn.execute(
            "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms, source) \
             VALUES (?1, ?2, '[]', ?3, 1, ?4)",
            rusqlite::params![id, query, at, source],
        )
        .unwrap();
    }

    let rows = distinct_prefix_matches(&conn, "search-code", "DRAFT%", 2).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.query_id.as_str()).collect::<Vec<_>>(),
        ["c-next", "b-new"],
        "same-time rows use descending id, and the newer Unicode variant wins"
    );
    assert_eq!(rows[1].query, "draft ä");
    assert_eq!(
        distinct_prefix_matches(&conn, "search-code", "DRAFT%", 10)
            .unwrap()
            .len(),
        2,
        "the older Unicode case variant is deduplicated"
    );
    assert!(
        distinct_prefix_matches(&conn, "search-code", "DRAFT%", 0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn retrieval_log_reads_use_the_time_indexes() {
    let conn = seed_db();
    let recent_plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT query, query_id, at FROM retrieval_log \
             WHERE source != ?1 AND query LIKE ?2 ESCAPE '\\' \
             ORDER BY at DESC, query_id DESC",
            rusqlite::params!["search-code", "draft%"],
            |r| r.get(3),
        )
        .unwrap();
    assert!(
        recent_plan.contains("idx_retrieval_log_recent"),
        "{recent_plan}"
    );

    let window_plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT returned_ids FROM retrieval_log \
             WHERE source IN (?1, ?2) AND at >= ?3 AND at <= ?4 \
             AND (repo IS NULL OR repo = ?5)",
            rusqlite::params!["search", "context", "2026-07-01", "2026-07-31", REPO],
            |r| r.get(3),
        )
        .unwrap();
    assert!(
        window_plan.contains("idx_retrieval_log_source_at"),
        "{window_plan}"
    );
}

#[test]
fn prefix_scans_propagate_row_decoding_errors() {
    let conn = seed_db();
    insert_row(&conn, "bad-time", "[]", "2026-07-15T00:00:00Z", "search");
    conn.execute(
        "UPDATE retrieval_log SET at = X'00' WHERE query_id = 'bad-time'",
        [],
    )
    .unwrap();
    for result in [
        prefix_matches(&conn, "search-code", "q%"),
        distinct_prefix_matches(&conn, "search-code", "q%", 1),
    ] {
        assert!(matches!(
            result,
            Err(comemory::errors::Error::Sqlite(
                rusqlite::Error::InvalidColumnType(2, _, rusqlite::types::Type::Blob)
            ))
        ));
    }
}

/// `queries_excluding_source` drops `search-code` rows and orders the rest
/// `(at, query_id)` — the scan behind `eval::mine`.
#[test]
fn queries_excluding_source_orders_by_at_then_query_id() {
    let conn = seed_db();
    insert_row(&conn, "q-b", "[]", "2026-07-15T00:00:01Z", "search");
    insert_row(&conn, "q-a", "[]", "2026-07-15T00:00:01Z", "context");
    insert_row(&conn, "q-earlier", "[]", "2026-07-15T00:00:00Z", "search");
    insert_row(
        &conn,
        "q-excluded",
        "[]",
        "2026-07-15T00:00:00Z",
        "search-code",
    );

    let rows = queries_excluding_source(&conn, "search-code").expect("query");
    let ids: Vec<&str> = rows.iter().map(|r| r.query_id.as_str()).collect();
    assert_eq!(ids, vec!["q-earlier", "q-a", "q-b"]);
}

/// Open a [`StatsDb`] over a fresh `comemory.db` in a tempdir — the same
/// connection [`record_with_provenance`] and [`insert`] must share so a
/// `feedback_events` row can judge a `retrieval_log` row written earlier in
/// the same test.
fn seed_stats_db() -> (StatsDb, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path());
    let db = StatsDb::open(paths.stats_db()).expect("open stats db");
    (db, tmp)
}

fn insert_pending_row(
    conn: &Connection,
    query_id: &str,
    source: &str,
    at: &str,
    ids: &str,
    repo: Option<&str>,
) {
    insert(
        conn,
        &NewLogRow {
            query_id,
            query: "q",
            returned_ids: ids,
            at,
            duration_ms: 1,
            repo,
            kind: None,
            source,
        },
    )
    .expect("insert retrieval_log row");
}

/// [`pending_since`] returns every tracked (`search`/`context`/`search-code`/
/// `find`) query at or after `since` with no `feedback_events` verdict, in
/// `at` order, with `returned_ids` decoded — the judged `context` row and the
/// other repo's row are both excluded once `repo` is scoped.
#[test]
fn pending_since_returns_unjudged_queries_in_order_and_respects_repo_and_since() {
    let (mut db, _tmp) = seed_stats_db();
    insert_pending_row(
        db.conn(),
        "q-search",
        source::SEARCH,
        "2026-07-15T00:00:00Z",
        r#"["a1"]"#,
        Some(REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-context",
        source::CONTEXT,
        "2026-07-15T00:01:00Z",
        r#"["a2"]"#,
        Some(REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-search-code",
        source::SEARCH_CODE,
        "2026-07-15T00:02:00Z",
        r#"["a3"]"#,
        Some(REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-find",
        source::FIND,
        "2026-07-15T00:03:00Z",
        r#"["a4"]"#,
        Some(REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-other-repo",
        source::SEARCH,
        "2026-07-15T00:01:30Z",
        r#"["a5"]"#,
        Some(OTHER_REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-no-repo",
        source::SEARCH,
        "2026-07-15T00:01:45Z",
        r#"["a6"]"#,
        None,
    );
    record_with_provenance(
        &mut db,
        "q-context",
        &["aaaaaaa1".to_string()],
        &[],
        PROV_MANUAL,
    )
    .expect("record verdict for q-context");

    let scoped = pending_since(db.conn(), Some(REPO), "2026-07-15T00:00:00Z").expect("query");
    let ids: Vec<&str> = scoped.iter().map(|r| r.query_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["q-search", "q-search-code", "q-find"],
        "judged q-context, the other repo's row, and the repo-less row are all excluded, \
         order is by at"
    );
    assert_eq!(scoped[0].query, "q");
    assert_eq!(scoped[0].source, source::SEARCH);
    assert_eq!(scoped[0].at, "2026-07-15T00:00:00Z");
    assert_eq!(scoped[0].returned_ids, vec!["a1".to_string()]);
    assert_eq!(scoped[2].returned_ids, vec!["a4".to_string()]);

    let unscoped = pending_since(db.conn(), None, "2026-07-15T00:00:00Z").expect("query");
    let unscoped_ids: Vec<&str> = unscoped.iter().map(|r| r.query_id.as_str()).collect();
    assert_eq!(
        unscoped_ids,
        vec![
            "q-search",
            "q-other-repo",
            "q-no-repo",
            "q-search-code",
            "q-find"
        ],
        "an unset repo filter includes every repo, including the repo-less row, \
         still ordered by at"
    );

    let future = pending_since(db.conn(), Some(REPO), "2026-07-16T00:00:00Z").expect("query");
    assert!(future.is_empty(), "a since in the future returns nothing");
}

/// A malformed `returned_ids` value propagates as [`Error::Json`] rather
/// than being silently dropped.
#[test]
fn pending_since_propagates_malformed_returned_ids_as_json_error() {
    let conn = seed_db();
    insert_pending_row(
        &conn,
        "q-malformed",
        source::SEARCH,
        "2026-07-15T00:00:00Z",
        "not json",
        Some(REPO),
    );

    let result = pending_since(&conn, Some(REPO), "2026-07-15T00:00:00Z");
    assert!(
        matches!(result, Err(comemory::errors::Error::Json(_))),
        "expected Error::Json for a malformed returned_ids value"
    );
}

/// [`count_since`] counts every tracked row in the window regardless of
/// whether it has been judged yet — unlike [`pending_since`], which excludes
/// judged rows.
#[test]
fn count_since_counts_pending_and_judged_rows_scoped_by_repo_and_since() {
    let (mut db, _tmp) = seed_stats_db();
    insert_pending_row(
        db.conn(),
        "q-search",
        source::SEARCH,
        "2026-07-15T00:00:00Z",
        r#"["a1"]"#,
        Some(REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-context",
        source::CONTEXT,
        "2026-07-15T00:01:00Z",
        r#"["a2"]"#,
        Some(REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-other-repo",
        source::SEARCH,
        "2026-07-15T00:01:30Z",
        r#"["a5"]"#,
        Some(OTHER_REPO),
    );
    insert_pending_row(
        db.conn(),
        "q-no-repo",
        source::SEARCH,
        "2026-07-15T00:01:45Z",
        r#"["a6"]"#,
        None,
    );
    record_with_provenance(
        &mut db,
        "q-context",
        &["aaaaaaa1".to_string()],
        &[],
        PROV_MANUAL,
    )
    .expect("record verdict for q-context");

    let scoped = count_since(db.conn(), Some(REPO), "2026-07-15T00:00:00Z").expect("count");
    assert_eq!(
        scoped, 2,
        "both the still-pending row and the now-judged row count for this repo; \
         the repo-less row does not match"
    );

    let unscoped = count_since(db.conn(), None, "2026-07-15T00:00:00Z").expect("count");
    assert_eq!(
        unscoped, 4,
        "an unset repo filter counts every repo, including the repo-less row"
    );

    let future = count_since(db.conn(), Some(REPO), "2026-07-16T00:00:00Z").expect("count");
    assert_eq!(future, 0, "a since in the future counts nothing");
}
