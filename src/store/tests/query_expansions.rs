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
