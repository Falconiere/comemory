#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for `src/store/code_text.rs` against a real `comemory.db`: rows go
//! in through the production `code_row::insert` writer and come back out
//! through `code_text::fetch`, so the column list and the identity/version
//! pairing are proved against the live schema rather than a hand-built table.

use comemory::config::paths::Paths;
use comemory::store::code_row::{self, CodeSymbolRow};
use comemory::store::{code_text, connection};
use tempfile::TempDir;

/// Open a migrated `comemory.db` in a tempdir.
fn open_db() -> (comemory::store::Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::new(tmp.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (conn, tmp)
}

/// Insert one real `code_symbols` row through the production writer.
fn seed(conn: &comemory::store::Connection, symbol: &str, oid: &str, snippet: &str) -> i64 {
    code_row::insert(
        conn,
        &CodeSymbolRow {
            repo: "demo",
            path: "src/a.rs",
            blob_oid: oid,
            symbol,
            kind: "function",
            lang: "rust",
            line_start: 4,
            line_end: 9,
            snippet,
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol")
}

#[test]
fn fetch_returns_identity_version_lines_and_snippet_for_every_id() {
    let (conn, _tmp) = open_db();
    let alpha = seed(&conn, "alpha", "0f1e2d3c", "fn alpha() { let x = 1; }");
    let beta = seed(&conn, "beta", "4b5a6978", "fn beta() {}");

    let map = code_text::fetch(&conn, &[alpha, beta]).expect("fetch");
    assert_eq!(map.len(), 2, "both ids resolve");

    let a = map.get(&alpha).expect("alpha present");
    assert_eq!(a.repo, "demo");
    assert_eq!(a.path, "src/a.rs");
    assert_eq!(a.symbol, "alpha");
    assert_eq!(a.blob_oid, "0f1e2d3c", "the content version, not the rowid");
    assert_eq!((a.line_start, a.line_end), (4, 9));
    assert_eq!(a.snippet, "fn alpha() { let x = 1; }");
    assert_eq!(map.get(&beta).expect("beta present").symbol, "beta");
}

#[test]
fn fetch_omits_an_id_with_no_live_row_rather_than_erroring() {
    let (conn, _tmp) = open_db();
    let alpha = seed(&conn, "alpha", "0f1e2d3c", "fn alpha() {}");

    let map = code_text::fetch(&conn, &[alpha, 9_999]).expect("a vanished rowid is not an error");
    assert!(map.contains_key(&alpha));
    assert!(
        !map.contains_key(&9_999),
        "a re-indexed-away rowid is simply absent, matching the raced-delete contract"
    );
}

#[test]
fn fetch_short_circuits_on_an_empty_id_list() {
    let (conn, _tmp) = open_db();
    // A malformed `IN ()` would be a SQL error, so reaching Ok here is the
    // assertion: the guard ran before any statement was prepared.
    let map = code_text::fetch(&conn, &[]).expect("empty id list is not a query");
    assert!(map.is_empty());
}

#[test]
fn fetch_collapses_duplicate_ids_into_one_entry() {
    let (conn, _tmp) = open_db();
    let alpha = seed(&conn, "alpha", "0f1e2d3c", "fn alpha() {}");
    let map = code_text::fetch(&conn, &[alpha, alpha, alpha]).expect("fetch");
    assert_eq!(
        map.len(),
        1,
        "the map is keyed by rowid, so duplicates merge"
    );
}
