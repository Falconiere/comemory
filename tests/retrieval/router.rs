use comemory::config::Config;
use comemory::retrieval::router;
use comemory::store::{connection, fts, vector};
use tempfile::tempdir;

#[path = "../common/vectors.rs"]
mod vectors;

#[test]
fn lexical_path_when_no_vector() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    seed_memory(&conn, "lex1", "advisory lock postgres");
    fts::index_memory(&conn, "lex1", "advisory lock postgres", "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "advisory lock", None, None).expect("route");
    assert_eq!(hits[0].memory_id, "lex1");
}

#[test]
fn vector_path_when_vector_provided() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    seed_memory(&conn, "vec1", "irrelevant text");
    let v = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "vec1", &v).expect("vec");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "no lex match", Some(&v), None).expect("route");
    assert_eq!(hits[0].memory_id, "vec1");
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, body: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1, 'x','note','h', ?2, 't','t','x.md')",
        rusqlite::params![id, body],
    )
    .expect("seed memory");
}
