//! Test mirror for `src/store/fts.rs`.

use comemory::store::{connection, fts};
use tempfile::tempdir;

#[test]
fn bm25_returns_seeded_match() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('mem1','m','note','h','postgres advisory locks for migration','t','t','m.md')",
        [],
    )
    .expect("seed memory");

    fts::index_memory(
        &conn,
        "mem1",
        "postgres advisory locks for migration",
        "db,postgres",
    )
    .expect("index");

    let hits = fts::search_memory(&conn, "advisory lock", 10, None).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");
}

#[test]
fn search_memory_skips_soft_deleted() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,deleted_at,md_path) \
         VALUES('mem1','m','note','h','postgres advisory locks for migration','t','t','t','m.md')",
        [],
    )
    .expect("seed memory");

    fts::index_memory(
        &conn,
        "mem1",
        "postgres advisory locks for migration",
        "db,postgres",
    )
    .expect("index");

    let hits = fts::search_memory(&conn, "advisory lock", 10, None).expect("search");
    assert!(
        hits.is_empty(),
        "soft-deleted memories must not appear in FTS results, got {hits:?}",
        hits = hits
            .iter()
            .map(|h| h.memory_id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn code_fts_returns_seeded_match() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    let symbol_path = "src/auth/login.rs";
    conn.execute(
        "INSERT INTO code_symbols\
            (id,repo,path,blob_oid,symbol,kind,lang,line_start,line_end,snippet,simhash,indexed_at) \
         VALUES(1,'r',?1,'oid','login::handle','function','rust',1,10,\
                'fn handle() { /* advisory login flow */ }',0,'t')",
        [symbol_path],
    )
    .expect("seed code symbol");

    let path_tokens = fts::path_to_tokens(symbol_path);
    fts::index_code(
        &conn,
        1,
        "login::handle",
        "fn handle() { /* advisory login flow */ }",
        &path_tokens,
    )
    .expect("index code");

    let hits = fts::search_code(&conn, "login", 10).expect("search code");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol_id, 1);
}

#[test]
fn path_to_tokens_lowercases_and_splits_non_alnum() {
    assert_eq!(
        fts::path_to_tokens("src/Foo/Bar_baz.rs"),
        "src foo bar baz rs"
    );
    assert_eq!(
        fts::path_to_tokens("crates/CoreLib/mod-utils.ts"),
        "crates corelib mod utils ts"
    );
    assert_eq!(fts::path_to_tokens(""), "");
}
