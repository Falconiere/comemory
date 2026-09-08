#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/prune_apply.rs`. Pins the correlated
//! `NOT EXISTS` join on `(repo, path)`: `prune --apply` deletes whatever
//! [`stale_code_files`] / [`purge_stale_code_rows`] flag, so an
//! accidentally uncorrelated rewrite (matching on ANY `indexed_files` row
//! existing anywhere, rather than the same repo+path) would delete a live
//! file's code index the moment ANY other repo was indexed.

use comemory::store::connection;
use comemory::store::prune_apply::{
    count_orphan_memory_edges, delete_orphan_memory_edges, drop_dangling_edges,
    drop_orphan_code_refs, memory_for_prune, purge_stale_code_rows, stale_code_files,
};

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, deleted: bool) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, deleted_at, md_path,
                              access_count, last_accessed, simhash)
         VALUES (?1, ?1, 'note', 'demo', 'f', 3, 1, 'h-' || ?1, 'body ' || ?1,
                 '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', ?2,
                 'm/' || ?1 || '.md', 2, '2025-06-01T00:00:00Z', 0)",
        rusqlite::params![id, deleted.then_some("2025-07-01T00:00:00Z")],
    )
    .expect("seed memory");
}

fn seed_edge(conn: &rusqlite::Connection, src_kind: &str, src_id: &str, rel: &str, dst_id: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at) \
         VALUES (?1, ?2, 'file', ?4, ?3, '2025-01-01T00:00:00Z')",
        rusqlite::params![src_kind, src_id, rel, dst_id],
    )
    .expect("seed edge");
}

fn seed_code_symbol(conn: &rusqlite::Connection, repo: &str, path: &str, symbol: &str) {
    conn.execute(
        "INSERT INTO code_symbols(repo, path, blob_oid, symbol, kind, lang, \
                                   line_start, line_end, snippet, simhash, indexed_at) \
         VALUES (?1, ?2, 'deadbeef', ?3, 'function', 'rust', 1, 2, 'fn f() {}', 0, \
                 '2025-01-01T00:00:00Z')",
        rusqlite::params![repo, path, symbol],
    )
    .expect("seed code_symbols");
}

fn seed_indexed_file(conn: &rusqlite::Connection, repo: &str, path: &str) {
    conn.execute(
        "INSERT INTO indexed_files(repo, path, blob_oid, indexed_at) \
         VALUES (?1, ?2, 'deadbeef', '2025-01-01T00:00:00Z')",
        rusqlite::params![repo, path],
    )
    .expect("seed indexed_files");
}

#[test]
fn stale_code_files_join_is_correlated_on_repo_and_path() {
    let (_d, conn) = seed_db();
    // Same path, two different repos. Repo "a" has a matching indexed_files
    // cursor (fresh); repo "b" shares the SAME path but has NO indexed_files
    // row at all (stale). An uncorrelated rewrite of the `NOT EXISTS`
    // subquery would see repo "a"'s indexed_files row and wrongly clear
    // repo "b"'s file too.
    seed_code_symbol(&conn, "a", "src/lib.rs", "f");
    seed_indexed_file(&conn, "a", "src/lib.rs");
    seed_code_symbol(&conn, "b", "src/lib.rs", "g");

    let stale = stale_code_files(&conn).expect("query");
    assert_eq!(
        stale,
        vec!["b:src/lib.rs".to_string()],
        "only the repo with no matching indexed_files cursor is stale: {stale:?}"
    );
}

#[test]
fn purge_stale_code_rows_is_correlated_on_repo_and_path() {
    let (_d, conn) = seed_db();
    seed_code_symbol(&conn, "a", "src/lib.rs", "f");
    seed_indexed_file(&conn, "a", "src/lib.rs");
    seed_code_symbol(&conn, "b", "src/lib.rs", "g");

    purge_stale_code_rows(&conn).expect("purge");

    let remaining: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT repo, path FROM code_symbols")
            .expect("prepare");
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(
        remaining,
        vec![("a".to_string(), "src/lib.rs".to_string())],
        "only repo b's stale row is purged: {remaining:?}"
    );
}

#[test]
fn count_and_delete_orphan_memory_edges_agree() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", false);
    // Orphan: sourced at a memory id with no live row at all.
    seed_edge(&conn, "memory", "aaaa0002", "references_file", "x:y");
    // Not orphan: sourced at a live memory.
    seed_edge(&conn, "memory", "aaaa0001", "references_file", "x:y");

    let count = count_orphan_memory_edges(&conn).expect("count");
    assert_eq!(count, 1);

    delete_orphan_memory_edges(&conn).expect("delete");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .expect("count");
    assert_eq!(remaining, 1, "the live memory's edge must survive");
}

#[test]
fn count_orphan_memory_edges_excludes_soft_deleted() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", true);
    seed_edge(&conn, "memory", "aaaa0001", "references_file", "x:y");

    let count = count_orphan_memory_edges(&conn).expect("count");
    assert_eq!(
        count, 1,
        "a soft-deleted memory's edge counts as orphan (deleted_at IS NULL predicate)"
    );
}

#[test]
fn memory_for_prune_reads_the_display_fields() {
    let (_d, conn) = seed_db();
    seed_memory(&conn, "aaaa0001", false);

    let row = memory_for_prune(&conn, "aaaa0001").expect("read");
    assert_eq!(row.body, "body aaaa0001");
    assert_eq!(row.created_at, "2025-01-01T00:00:00Z");
    assert_eq!(row.access_count, 2);
    assert_eq!(row.last_accessed.as_deref(), Some("2025-06-01T00:00:00Z"));
}

#[test]
fn dangling_edges_and_orphan_code_refs_are_dropped_together() {
    let (_d, conn) = seed_db();
    // No code_symbols row at all for "demo:src/gone.rs" — every edge below
    // dangles.
    seed_edge(
        &conn,
        "memory",
        "aaaa0001",
        "references_file",
        "demo:src/gone.rs",
    );
    seed_edge(
        &conn,
        "memory",
        "aaaa0001",
        "references_symbol",
        "demo:src/gone.rs:f",
    );
    seed_edge(
        &conn,
        "memory",
        "aaaa0001",
        "co_activated",
        "file:demo:src/gone.rs",
    );
    conn.execute(
        "INSERT INTO code_ref(memory_id, rel, dst_id, created_at) \
         VALUES ('aaaa0001', 'references_file', 'demo:src/gone.rs', '2025-01-01T00:00:00Z')",
        [],
    )
    .expect("seed code_ref");

    drop_dangling_edges(&conn).expect("drop dangling");
    drop_orphan_code_refs(&conn).expect("drop orphan refs");

    let edges: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .expect("count");
    assert_eq!(edges, 0, "every dangling edge must be gone");
    let refs: i64 = conn
        .query_row("SELECT COUNT(*) FROM code_ref", [], |r| r.get(0))
        .expect("count");
    assert_eq!(refs, 0, "the now-orphaned code_ref row must be gone");
}
