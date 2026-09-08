#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/doctor_probes.rs`.

use comemory::store::connection;
use comemory::store::doctor_probes::{live_memory_count, live_memory_hashes, repo_roots};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, hash: &str, deleted: bool) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, deleted_at, md_path)
         VALUES (?1, ?1, 'note', 'demo', 'f', 3, 1, ?2, 'body', \
                 '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', ?3, 'm/' || ?1 || '.md')",
        rusqlite::params![id, hash, deleted.then_some("2025-07-01T00:00:00Z")],
    )
    .expect("seed memory");
}

#[test]
fn live_memory_hashes_excludes_soft_deleted() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", "hash1", false);
    seed_memory(&conn, "aaaa0002", "hash2", true);

    let hashes = live_memory_hashes(&conn).expect("query");
    assert_eq!(hashes.len(), 1);
    assert_eq!(hashes.get("aaaa0001").map(String::as_str), Some("hash1"));
}

#[test]
fn live_memory_count_excludes_soft_deleted() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", "hash1", false);
    seed_memory(&conn, "aaaa0002", "hash2", true);

    assert_eq!(live_memory_count(&conn).expect("count"), 1);
}

#[test]
fn repo_roots_excludes_null_root_path() {
    let (_d, conn) = seed_db();
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES ('with-root', '/tmp/x')",
        [],
    )
    .expect("seed marker");
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES ('no-root', NULL)",
        [],
    )
    .expect("seed marker");

    let rows = repo_roots(&conn).expect("query");
    assert_eq!(rows, vec![("with-root".to_string(), "/tmp/x".to_string())]);
}
