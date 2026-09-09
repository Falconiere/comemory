#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/stats_counts.rs`.

use comemory::store::connection;
use comemory::store::stats_counts::{count_table, db_bytes, scoped_count};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, repo: &str, deleted: bool) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, deleted_at, md_path)
         VALUES (?1, ?1, 'note', ?2, 'f', 3, 1, 'h', 'body', \
                 '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', ?3, 'm/' || ?1 || '.md')",
        rusqlite::params![id, repo, deleted.then_some("2025-07-01T00:00:00Z")],
    )
    .expect("seed memory");
}

#[test]
fn scoped_count_narrows_by_repo() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", "repo-a", false);
    seed_memory(&conn, "aaaa0002", "repo-b", false);

    let all = scoped_count(&conn, "memories", "deleted_at IS NULL", None).expect("count");
    assert_eq!(all, 2);
    let scoped =
        scoped_count(&conn, "memories", "deleted_at IS NULL", Some("repo-a")).expect("count");
    assert_eq!(scoped, 1);
}

#[test]
fn scoped_count_predicate_selects_deleted_rows() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", "repo-a", false);
    seed_memory(&conn, "aaaa0002", "repo-a", true);

    let trashed = scoped_count(&conn, "memories", "deleted_at IS NOT NULL", None).expect("count");
    assert_eq!(trashed, 1);
}

#[test]
fn count_table_counts_the_whole_table() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", "repo-a", false);
    seed_memory(&conn, "aaaa0002", "repo-a", true);

    assert_eq!(count_table(&conn, "memories").expect("count"), 2);
}

#[test]
fn db_bytes_is_pages_times_page_size() {
    let (_d, conn) = seed_db();
    let pages: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .expect("page_count");
    let size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .expect("page_size");
    let expected = (pages.max(0) as u64) * (size.max(0) as u64);
    assert_eq!(db_bytes(&conn).expect("db_bytes"), expected);
}
