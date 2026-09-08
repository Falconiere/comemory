#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/store/repos_inventory.rs`.

use comemory::store::connection;
use comemory::store::repos_inventory::fetch;

fn seed_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = connection::open(dir.path().join("comemory.db")).expect("open");
    (dir, conn)
}

fn seed_repo(conn: &rusqlite::Connection, repo: &str) {
    conn.execute(
        "INSERT INTO repo_marker(repo, last_head, last_indexed_at) \
         VALUES (?1, 'deadbeef', '2025-01-01T00:00:00Z')",
        [repo],
    )
    .expect("seed marker");
}

fn seed_code_symbol(conn: &rusqlite::Connection, repo: &str, path: &str) {
    conn.execute(
        "INSERT INTO code_symbols(repo, path, blob_oid, symbol, kind, lang, \
                                   line_start, line_end, snippet, simhash, indexed_at) \
         VALUES (?1, ?2, 'deadbeef', 'f', 'function', 'rust', 1, 2, 'fn f() {}', 0, \
                 '2025-01-01T00:00:00Z')",
        rusqlite::params![repo, path],
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
fn fetch_joins_per_repo_counters() {
    let (_d, conn) = seed_db();
    seed_repo(&conn, "repo-a");
    seed_repo(&conn, "repo-b");
    seed_code_symbol(&conn, "repo-a", "src/lib.rs");
    seed_code_symbol(&conn, "repo-a", "src/main.rs");
    seed_indexed_file(&conn, "repo-a", "src/lib.rs");
    seed_indexed_file(&conn, "repo-a", "src/main.rs");

    let rows = fetch(&conn, None).expect("fetch");
    assert_eq!(rows.len(), 2);
    let a = rows.iter().find(|r| r.repo == "repo-a").expect("repo-a");
    assert_eq!(a.symbols, 2);
    assert_eq!(a.files, 2);
    let b = rows.iter().find(|r| r.repo == "repo-b").expect("repo-b");
    assert_eq!(b.symbols, 0);
}

#[test]
fn fetch_narrows_to_one_repo() {
    let (_d, conn) = seed_db();
    seed_repo(&conn, "repo-a");
    seed_repo(&conn, "repo-b");

    let rows = fetch(&conn, Some("repo-a")).expect("fetch");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].repo, "repo-a");
}
