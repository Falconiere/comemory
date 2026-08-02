//! Test mirror for `src/store/document_fts.rs`.

use comemory::store::connection;
use comemory::store::document_fts;
use rusqlite::Connection;
use tempfile::TempDir;

fn open_db() -> (Connection, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let conn = connection::open(tmp.path().join("comemory.db")).expect("open db");
    (conn, tmp)
}

fn seed(
    conn: &Connection,
    document_id: &str,
    ordinal: i64,
    title: &str,
    headings: &str,
    passage: &str,
    path: &str,
) {
    document_fts::insert(conn, document_id, ordinal, title, headings, passage, path)
        .expect("insert chunk");
}

#[test]
fn insert_then_match_finds_the_row() {
    let (conn, _tmp) = open_db();
    seed(
        &conn,
        "doc-a",
        0,
        "Guide",
        "Installation",
        "install via cargo install --path .",
        "guide.md",
    );

    let hits = document_fts::search(&conn, "cargo install", 10).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, "doc-a");
    assert_eq!(hits[0].ordinal, 0);
}

#[test]
fn delete_document_removes_all_its_chunks() {
    let (conn, _tmp) = open_db();
    seed(
        &conn,
        "doc-b",
        0,
        "T",
        "",
        "unique needle phrase here",
        "b.md",
    );
    seed(
        &conn,
        "doc-b",
        1,
        "T",
        "",
        "another passage also mentions needle",
        "b.md",
    );

    document_fts::delete_document(&conn, "doc-b").expect("delete");

    let hits = document_fts::search(&conn, "needle", 10).expect("search");
    assert!(hits.is_empty(), "deleted document's chunks must not match");
}

#[test]
fn search_ranks_the_denser_match_first() {
    let (conn, _tmp) = open_db();
    seed(
        &conn,
        "doc-off-topic",
        0,
        "Changelog",
        "",
        "unrelated release notes about pagerank tuning, rate limiter mentioned once",
        "changelog.txt",
    );
    seed(
        &conn,
        "doc-on-topic",
        0,
        "Guide",
        "Rate limiting",
        "rate limiter rate limiter token bucket rate limiter design deep dive",
        "guide.md",
    );

    let hits = document_fts::search(&conn, "rate limiter", 10).expect("search");
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0].document_id, "doc-on-topic",
        "denser match must rank first"
    );
}

#[test]
fn empty_query_and_zero_k_return_empty() {
    let (conn, _tmp) = open_db();
    seed(&conn, "doc-c", 0, "T", "", "content", "c.md");
    assert!(
        document_fts::search(&conn, "", 10)
            .expect("search")
            .is_empty()
    );
    assert!(
        document_fts::search(&conn, "content", 0)
            .expect("search")
            .is_empty()
    );
}
