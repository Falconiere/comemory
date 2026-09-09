#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/trash_list.rs`.

use comemory::store::connection;
use comemory::store::trash_list::deleted_memories;

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, deleted_at: Option<&str>) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, deleted_at, md_path)
         VALUES (?1, ?1, 'note', 'demo', 'f', 3, 1, 'h', 'body ' || ?1, \
                 '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', ?2, 'm/' || ?1 || '.md')",
        rusqlite::params![id, deleted_at],
    )
    .expect("seed memory");
}

#[test]
fn deleted_memories_excludes_live_rows_and_orders_newest_first() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", None);
    seed_memory(&conn, "aaaa0002", Some("2025-06-01T00:00:00Z"));
    seed_memory(&conn, "aaaa0003", Some("2025-07-01T00:00:00Z"));

    let rows = deleted_memories(&conn).expect("query");
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["aaaa0003", "aaaa0002"],
        "newest deletion first, live row excluded"
    );
}

#[test]
fn deleted_memories_ties_break_on_id() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "bbbb0002", Some("2025-06-01T00:00:00Z"));
    seed_memory(&conn, "aaaa0001", Some("2025-06-01T00:00:00Z"));

    let rows = deleted_memories(&conn).expect("query");
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["aaaa0001", "bbbb0002"]);
}
