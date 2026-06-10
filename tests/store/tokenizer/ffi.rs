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
fn store_open_registers_tokenizer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = comemory::store::connection::open(&db).expect("open store");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
        .expect("memory_fts exists and is queryable");
    assert_eq!(n, 0);
}
