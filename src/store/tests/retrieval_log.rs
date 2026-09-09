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

use comemory::store::connection;
use comemory::store::retrieval_log::{
    NewLogRow, insert, queries_excluding_source, returned_ids_in_window,
};
use rusqlite::Connection;
use tempfile::tempdir;

const REPO: &str = "r";

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
