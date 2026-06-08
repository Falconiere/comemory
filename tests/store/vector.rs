//! Test mirror for `src/store/vector.rs`.
//!
//! Exercises `insert_memory` / `knn_memory` end-to-end against a real
//! `sqlite-vec`-backed connection plus the dim guard surfaced through
//! the schema_meta lookup.

use comemory::store::{connection, vector};
use tempfile::tempdir;

use crate::vectors;

#[test]
fn insert_and_knn_returns_nearest_neighbor() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    // Seed a row in memories so the FK in memory_vec.memory_id is satisfiable
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('aaaa1111','a','note','hash1','body','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','a.md')",
        [],
    ).expect("seed memories");
    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('bbbb2222','b','note','hash2','body','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','b.md')",
        [],
    ).expect("seed memories");

    let v_a = vectors::vector("alpha", 1024);
    let v_b = vectors::vector("beta", 1024);
    let v_q = v_a.clone();

    vector::insert_memory(&conn, "aaaa1111", &v_a).expect("insert a");
    vector::insert_memory(&conn, "bbbb2222", &v_b).expect("insert b");

    let hits = vector::knn_memory(&conn, &v_q, 1, None).expect("knn");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory_id, "aaaa1111");
}

#[test]
fn insert_rejects_mismatched_dim() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    conn.execute(
        "INSERT INTO memories(id,slug,kind,content_hash,body,created_at,updated_at,md_path) \
         VALUES('cccc3333','c','note','hash3','body','2026-06-08T00:00:00Z','2026-06-08T00:00:00Z','c.md')",
        [],
    ).expect("seed memories");

    let bad = vectors::vector("c", 16);
    let err = vector::insert_memory(&conn, "cccc3333", &bad).expect_err("dim mismatch");
    assert!(format!("{err}").contains("dim mismatch"));
}
