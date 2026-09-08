#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/store/indexed_files.rs`.

use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use comemory::store::indexed_files::{blob_oid_for, delete_for_repo};
use rusqlite::Connection;
use tempfile::tempdir;

fn seed_db() -> Connection {
    let dir = tempdir().expect("tempdir");
    connection::open(dir.path().join("comemory.db")).expect("open")
}

/// Seed one `code_symbols` row and its `indexed_files` cursor exactly as
/// `api::index_code::walk::index_file` does: insert the symbol, then
/// upsert the cursor via the production writer.
fn seed_indexed_file(conn: &Connection, repo: &str, path: &str, oid: &str) {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path,
            blob_oid: oid,
            symbol: "f",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 2,
            snippet: "fn f() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol");
    code_row::upsert_indexed_file(conn, repo, path, oid).expect("upsert indexed_files cursor");
}

#[test]
fn blob_oid_for_returns_none_when_never_indexed() {
    let conn = seed_db();
    assert_eq!(blob_oid_for(&conn, "r", "src/lib.rs").expect("query"), None);
}

#[test]
fn blob_oid_for_returns_the_recorded_oid() {
    let conn = seed_db();
    seed_indexed_file(&conn, "r", "src/lib.rs", "oid-1");
    assert_eq!(
        blob_oid_for(&conn, "r", "src/lib.rs").expect("query"),
        Some("oid-1".to_string())
    );
    // A different repo or path never matches.
    assert_eq!(
        blob_oid_for(&conn, "other", "src/lib.rs").expect("query"),
        None
    );
    assert_eq!(
        blob_oid_for(&conn, "r", "src/other.rs").expect("query"),
        None
    );
}

#[test]
fn delete_for_repo_drops_only_that_repos_cursors() {
    let conn = seed_db();
    seed_indexed_file(&conn, "r", "src/lib.rs", "oid-1");
    seed_indexed_file(&conn, "other", "src/lib.rs", "oid-2");

    delete_for_repo(&conn, "r").expect("delete");

    assert_eq!(blob_oid_for(&conn, "r", "src/lib.rs").expect("query"), None);
    assert_eq!(
        blob_oid_for(&conn, "other", "src/lib.rs").expect("query"),
        Some("oid-2".to_string()),
        "an unrelated repo's cursor must survive"
    );
}
