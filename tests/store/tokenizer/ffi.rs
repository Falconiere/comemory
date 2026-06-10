//! Tests for the `identifier` FTS5 tokenizer FFI registration.

use rusqlite::Connection;

fn conn_with_tokenizer() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    comemory::store::tokenizer::ffi::register(&conn).expect("register tokenizer");
    conn
}

#[test]
fn identifier_tokenizer_creates_table_and_matches_subtokens() {
    let conn = conn_with_tokenizer();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(body, tokenize = 'identifier');
         INSERT INTO t(body) VALUES ('returns VecDimMismatch on bad embedder');",
    )
    .expect("create + insert");

    let n: i64 = conn
        .query_row("SELECT count(*) FROM t WHERE t MATCH 'mismatch'", [], |r| {
            r.get(0)
        })
        .expect("query");
    assert_eq!(n, 1);

    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM t WHERE t MATCH 'vecdimmismatch'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(n, 1);

    conn.execute(
        "INSERT INTO t(body) VALUES ('the dim mismatch guard fires')",
        [],
    )
    .expect("insert");
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM t WHERE t MATCH 'DimMismatch'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(n, 2);
}

#[test]
fn porter_wraps_identifier() {
    let conn = conn_with_tokenizer();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t2 USING fts5(body, tokenize = 'porter identifier');
         INSERT INTO t2(body) VALUES ('indexing the repository');",
    )
    .expect("create + insert");
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM t2 WHERE t2 MATCH 'indexed'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(n, 1);
}

#[test]
fn register_twice_on_same_connection_still_matches() {
    let conn = conn_with_tokenizer();
    comemory::store::tokenizer::ffi::register(&conn).expect("re-register tokenizer");
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t3 USING fts5(body, tokenize = 'identifier');
         INSERT INTO t3(body) VALUES ('VecDimMismatch guard');",
    )
    .expect("create + insert");
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM t3 WHERE t3 MATCH 'mismatch'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(n, 1);
}

#[test]
fn invalid_utf8_blob_inserts_matches_and_highlights() {
    let conn = conn_with_tokenizer();
    conn.execute_batch("CREATE VIRTUAL TABLE t4 USING fts5(body, tokenize = 'identifier');")
        .expect("create");
    // Invalid UTF-8 before the identifiers: lossy decode widens each bad
    // byte into U+FFFD (3 bytes), shifting offsets — exercises the clamp.
    let blob: &[u8] = b"\xFF\xFE DimGuard fires VecDimMismatch \xF0\x28";
    conn.execute("INSERT INTO t4(body) VALUES (?1)", [blob])
        .expect("insert invalid-utf8 blob");
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM t4 WHERE t4 MATCH 'mismatch'",
            [],
            |r| r.get(0),
        )
        .expect("match over invalid-utf8 row");
    assert_eq!(n, 1);
    let highlighted: Vec<u8> = conn
        .query_row(
            "SELECT CAST(highlight(t4, 0, '[', ']') AS BLOB) FROM t4 WHERE t4 MATCH 'guard'",
            [],
            |r| r.get(0),
        )
        .expect("highlight over invalid-utf8 row");
    assert!(highlighted.contains(&b'['));
}

#[test]
fn store_open_registers_tokenizer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = comemory::store::connection::open(&db).expect("open store");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
        .expect("memory_fts exists and is queryable");
    assert_eq!(n, 0);
}
