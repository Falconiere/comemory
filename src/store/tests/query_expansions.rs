#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/query_expansions.rs` — the `mine --apply`
//! replace-all pair (`delete_all` + `insert`).

use comemory::store::connection;
use comemory::store::query_expansions::{self, NewExpansion};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn stored_rows(conn: &rusqlite::Connection) -> Vec<(String, String, i64)> {
    let mut stmt = conn
        .prepare("SELECT term, expansion, support FROM query_expansions ORDER BY term")
        .expect("prepare");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

#[test]
fn insert_then_read_round_trips() {
    let (_d, conn) = seed_db();
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "pg",
            expansion: "postgres",
            support: 3,
            last_mined: "2026-07-20T00:00:00Z",
        },
    )
    .expect("insert");

    let rows = stored_rows(&conn);
    assert_eq!(rows, vec![("pg".to_string(), "postgres".to_string(), 3)]);
}

#[test]
fn delete_all_clears_the_table() {
    let (_d, conn) = seed_db();
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "pg",
            expansion: "postgres",
            support: 1,
            last_mined: "2026-07-20T00:00:00Z",
        },
    )
    .expect("insert");
    query_expansions::delete_all(&conn).expect("delete_all");
    assert!(stored_rows(&conn).is_empty());
}

#[test]
fn replace_all_pattern_drops_stale_mappings() {
    let (_d, conn) = seed_db();
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "stale",
            expansion: "old",
            support: 5,
            last_mined: "2026-07-01T00:00:00Z",
        },
    )
    .expect("seed stale");

    // A re-mine run: delete then insert only the current mappings.
    query_expansions::delete_all(&conn).expect("delete_all");
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "fresh",
            expansion: "new",
            support: 2,
            last_mined: "2026-07-20T00:00:00Z",
        },
    )
    .expect("seed fresh");

    let rows = stored_rows(&conn);
    assert_eq!(rows, vec![("fresh".to_string(), "new".to_string(), 2)]);
}

#[test]
fn matching_terms_orders_by_support_desc_then_term_then_expansion() {
    let (_d, conn) = seed_db();
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "auth",
            expansion: "oauth",
            support: 2,
            last_mined: "2026-08-01T00:00:00Z",
        },
    )
    .expect("seed");
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "auth",
            expansion: "session",
            support: 7,
            last_mined: "2026-08-01T00:00:00Z",
        },
    )
    .expect("seed");
    query_expansions::insert(
        &conn,
        &NewExpansion {
            term: "billing",
            expansion: "invoice",
            support: 9,
            last_mined: "2026-08-01T00:00:00Z",
        },
    )
    .expect("seed");

    let terms = vec!["auth".to_string()];
    let rows = query_expansions::matching_terms(&conn, &terms, 10).expect("query");
    let pairs: Vec<(&str, &str, i64)> = rows
        .iter()
        .map(|r| (r.term.as_str(), r.expansion.as_str(), r.support))
        .collect();
    assert_eq!(
        pairs,
        vec![("auth", "session", 7), ("auth", "oauth", 2)],
        "only the requested term, strongest support first"
    );
}

#[test]
fn matching_terms_respects_the_limit() {
    let (_d, conn) = seed_db();
    for (expansion, support) in [("a", 1), ("b", 2), ("c", 3)] {
        query_expansions::insert(
            &conn,
            &NewExpansion {
                term: "t",
                expansion,
                support,
                last_mined: "2026-08-01T00:00:00Z",
            },
        )
        .expect("seed");
    }

    let terms = vec!["t".to_string()];
    let rows = query_expansions::matching_terms(&conn, &terms, 1).expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].expansion, "c",
        "the strongest-support row wins the cap"
    );
}

#[test]
fn matching_terms_with_no_terms_never_queries_the_db() {
    let (_d, conn) = seed_db();
    let rows = query_expansions::matching_terms(&conn, &[], 10).expect("query");
    assert!(rows.is_empty());
}

#[test]
fn count_and_page_agree_with_the_table_and_page_ordering() {
    let (_d, conn) = seed_db();
    assert_eq!(query_expansions::count(&conn).expect("count on empty"), 0);

    for (term, expansion, support) in [
        ("lock", "advisory", 5_i64),
        ("wal", "checkpoint", 9),
        ("index", "btree", 1),
    ] {
        query_expansions::insert(
            &conn,
            &NewExpansion {
                term,
                expansion,
                support,
                last_mined: "2026-09-01T00:00:00Z",
            },
        )
        .expect("seed");
    }

    assert_eq!(query_expansions::count(&conn).expect("count"), 3);

    let first_page = query_expansions::page(&conn, 2, 0).expect("page 1");
    assert_eq!(first_page.len(), 2);
    assert_eq!(first_page[0].term, "wal", "strongest support first");
    assert_eq!(first_page[1].term, "lock");

    let second_page = query_expansions::page(&conn, 2, 2).expect("page 2");
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].term, "index");

    // A negative LIMIT means "no limit" — the `Page`-"all" sentinel.
    let all = query_expansions::page(&conn, -1, 0).expect("all");
    assert_eq!(all.len(), 3);
}
