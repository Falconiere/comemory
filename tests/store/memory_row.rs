//! Verifies the shared `memory_row::insert` helper writes the `memories`
//! row, every `memory_tags` row, the `memory_fts` index entry, and the
//! v0.2 edges (in_repo / authored_by / tagged plus cross-link references)
//! that both `cli::save` and `cli::rebuild` depend on.

use comemory::memory::{Frontmatter, Kind, References, Relations};
use comemory::store::{connection, memory_row};
use rusqlite::Connection;
use tempfile::tempdir;
use time::OffsetDateTime;

const ID: &str = "abc12345";

fn sample_fm() -> Frontmatter {
    Frontmatter {
        id: ID.to_string(),
        kind: Kind::Decision,
        repo: "qwick".to_string(),
        tags: vec!["db".to_string(), "postgres".to_string()],
        author: "alice".to_string(),
        created: OffsetDateTime::now_utc(),
        quality: 4,
        schema: 1,
        content_hash: "deadbeef".to_string(),
        references: References::default(),
        relations: Relations::default(),
    }
}

fn count_by_id(conn: &Connection, table: &str, col: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM {table} WHERE {col} = ?1");
    conn.query_row(&sql, [ID], |r| r.get(0)).expect("count")
}

fn assert_edge(conn: &Connection, rel: &str, dst_kind: &str, dst_id: &str) {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1 \
               AND rel = ?2 AND dst_kind = ?3 AND dst_id = ?4",
            rusqlite::params![ID, rel, dst_kind, dst_id],
            |r| r.get(0),
        )
        .expect("count edges");
    assert_eq!(n, 1, "expected edge {rel} -> {dst_kind}:{dst_id}");
}

fn assert_row_counts(conn: &Connection) {
    assert_eq!(count_by_id(conn, "memories", "id"), 1);
    assert_eq!(count_by_id(conn, "memory_tags", "memory_id"), 2);
    assert_eq!(count_by_id(conn, "memory_fts", "memory_id"), 1);
}

fn assert_all_edges(conn: &Connection) {
    assert_edge(conn, "in_repo", "repo", "qwick");
    assert_edge(conn, "authored_by", "author", "alice");
    assert_edge(conn, "tagged", "tag", "db");
    assert_edge(conn, "tagged", "tag", "postgres");
    assert_edge(conn, "references_file", "file", "qwick:src/lib.rs");
    assert_edge(
        conn,
        "references_symbol",
        "symbol",
        "qwick:src/lib.rs:start",
    );
}

#[test]
fn inserts_row_tags_fts_and_edges() {
    let dir = tempdir().expect("tempdir");
    let mut conn = connection::open(dir.path().join("comemory.db")).expect("open");
    let tx = conn.transaction().expect("tx");
    let fm = sample_fm();
    let body = "use `qwick:src/lib.rs:start` for bootstrap";
    memory_row::insert(&tx, &fm, body, "slug-x", "/abs/path.md", &fm.tags).expect("insert");
    tx.commit().expect("commit");

    assert_row_counts(&conn);
    assert_all_edges(&conn);
}
