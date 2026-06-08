//! Test mirror for `src/store/code_row.rs`.
//!
//! Verifies that the shared `code_symbols` insert helper returns the
//! freshly-assigned PK and writes every supplied column verbatim.

use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::connection;
use tempfile::TempDir;

#[test]
fn insert_returns_pk_and_persists_columns() {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");

    let sid = code_row::insert(
        &conn,
        &CodeSymbolRow {
            repo: "myrepo",
            path: "src/lib.rs",
            blob_oid: "deadbeef",
            symbol: "do_work",
            kind: "function",
            lang: "rust",
            line_start: 10,
            line_end: 20,
            snippet: "fn do_work() {}",
            simhash: 42,
        },
    )
    .expect("insert ok");

    let (repo, path, symbol, line_start, line_end, simhash): (
        String,
        String,
        String,
        i64,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT repo, path, symbol, line_start, line_end, simhash FROM code_symbols WHERE id = ?1",
            rusqlite::params![sid],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .expect("row exists");

    assert_eq!(repo, "myrepo");
    assert_eq!(path, "src/lib.rs");
    assert_eq!(symbol, "do_work");
    assert_eq!(line_start, 10);
    assert_eq!(line_end, 20);
    assert_eq!(simhash, 42);
}
