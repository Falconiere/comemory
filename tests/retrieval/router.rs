//! Tests for [`comemory::retrieval::router::route`].
//!
//! Covers pure-lexical, pure-vector (empty query), and hybrid (vec + query)
//! paths.

use comemory::config::Config;
use comemory::retrieval::router::{self, Source};
use comemory::store::{connection, fts, vector};
use tempfile::tempdir;

#[path = "../common/vectors.rs"]
mod vectors;

fn seed_memory(conn: &rusqlite::Connection, id: &str, body: &str) {
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES(?1, 'x','note','h', ?2, 't','t','x.md')",
        rusqlite::params![id, body],
    )
    .expect("seed memory");
}

#[test]
fn lexical_path_when_no_vector() {
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_memory(&conn, "lex1", "advisory lock postgres");
    fts::index_memory(&conn, "lex1", "advisory lock postgres", "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "advisory lock", None, None).expect("route");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].memory_id, "lex1");
    assert_eq!(hits[0].source, Source::Lexical);
}

#[test]
fn pure_vector_path_when_empty_query() {
    // Empty query string → pure-vector path (no hybrid).
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_memory(&conn, "vec1", "irrelevant text");
    let v = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "vec1", &v).expect("vec");

    let cfg = Config::defaults();
    // Empty query → pure-vector branch.
    let hits = router::route(&cfg, &conn, "", Some(&v), None).expect("route");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].memory_id, "vec1");
    assert_eq!(hits[0].source, Source::Vector);
}

#[test]
fn hybrid_path_when_both_vector_and_query() {
    // Non-empty query + vector → hybrid RRF path.
    let dir = tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    seed_memory(&conn, "h1", "advisory lock postgres migration");
    let v = vectors::vector("seed", 1024);
    vector::insert_memory(&conn, "h1", &v).expect("vec");
    fts::index_memory(&conn, "h1", "advisory lock postgres migration", "").expect("fts");

    let cfg = Config::defaults();
    let hits = router::route(&cfg, &conn, "advisory lock", Some(&v), None).expect("route");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].source, Source::Hybrid);
}
