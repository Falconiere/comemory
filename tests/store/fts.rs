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

    let hits = fts::search_memory(&conn, "advisory lock", 10).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "mem1");
}
