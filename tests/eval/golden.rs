//! Tests for [`comemory::eval::golden`] — YAML loading, feedback harvest,
//! and the file-wins merge.

use std::path::Path;

use comemory::eval::golden::{harvest, load_file, merge, resolve, GoldenPair};

/// Open a real `comemory.db` (with migrations applied) in a tempdir.
fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    (dir, conn)
}

/// Insert a minimal live or soft-deleted memory row satisfying the
/// `memories` NOT NULL columns (0002 DDL + 0004 defaults).
fn insert_memory(conn: &rusqlite::Connection, id: &str, body: &str, deleted_at: Option<&str>) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, deleted_at, md_path, simhash)
         VALUES (?1, ?1, 'note', 'd', 'f', 3, 1, ?1, ?2,
                 '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?3, ?1, 0)",
        rusqlite::params![id, body, deleted_at],
    )
    .expect("insert memory");
}

/// Insert a `retrieval_log` row plus a used-verdict `feedback_events` row.
fn mark_used(conn: &rusqlite::Connection, query_id: &str, query: &str, memory_id: &str) {
    conn.execute(
        "INSERT OR IGNORE INTO retrieval_log(query_id, query, returned_ids, at, duration_ms)
         VALUES (?1, ?2, '[]', '2026-06-09T00:00:00Z', 1)",
        rusqlite::params![query_id, query],
    )
    .expect("insert retrieval_log");
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at)
         VALUES (?1, ?2, 'used', '2026-06-09T00:00:00Z')",
        rusqlite::params![query_id, memory_id],
    )
    .expect("insert feedback_events");
}

#[test]
fn load_file_parses_two_pairs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("golden.yaml");
    std::fs::write(
        &path,
        "- query: postgres pool\n  relevant: [aaaaaaa1, aaaaaaa2]\n\
         - query: auth race\n  relevant: [bbbbbbb1]\n",
    )
    .expect("write yaml");
    let pairs = load_file(&path).expect("load");
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].query, "postgres pool");
    assert_eq!(pairs[0].relevant, vec!["aaaaaaa1", "aaaaaaa2"]);
    assert_eq!(pairs[1].query, "auth race");
    assert_eq!(pairs[1].relevant, vec!["bbbbbbb1"]);
}

#[test]
fn load_file_malformed_yaml_names_the_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("broken.yaml");
    std::fs::write(&path, "- query: [unterminated\n").expect("write yaml");
    let err = load_file(&path).expect_err("malformed yaml must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("broken.yaml"),
        "error must name the offending path, got: {msg}"
    );
}

#[test]
fn harvest_drops_soft_deleted_and_missing_ids() {
    let (_d, conn) = open_db();
    insert_memory(&conn, "aaaaaaa1", "postgres pool exhausted fix", None);
    insert_memory(
        &conn,
        "aaaaaaa2",
        "old postgres pool note",
        Some("2026-06-09T00:00:00Z"),
    );
    mark_used(&conn, "q-20260609-aabbccdd", "postgres pool", "aaaaaaa1");
    mark_used(&conn, "q-20260609-aabbccdd", "postgres pool", "aaaaaaa2");
    // An id never persisted as a memories row carries no signal either.
    mark_used(&conn, "q-20260609-aabbccdd", "postgres pool", "deadbee1");

    let pairs = harvest(&conn).expect("harvest");
    assert_eq!(
        pairs.len(),
        1,
        "one query with live relevant ids: {pairs:?}"
    );
    assert_eq!(pairs[0].query, "postgres pool");
    assert_eq!(pairs[0].relevant, vec!["aaaaaaa1"]);
}

#[test]
fn harvest_deduplicates_repeated_verdicts() {
    let (_d, conn) = open_db();
    insert_memory(&conn, "aaaaaaa1", "postgres pool exhausted fix", None);
    mark_used(&conn, "q-20260609-aabbccdd", "postgres pool", "aaaaaaa1");
    mark_used(&conn, "q-20260610-aabbccdd", "postgres pool", "aaaaaaa1");

    let pairs = harvest(&conn).expect("harvest");
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0].relevant,
        vec!["aaaaaaa1"],
        "same id used twice for the same query must appear once"
    );
}

#[test]
fn merge_file_wins_on_duplicate_query_and_sorts() {
    let file = vec![GoldenPair {
        query: "postgres pool".into(),
        relevant: vec!["aaaaaaa1".into()],
    }];
    let harvested = vec![
        GoldenPair {
            query: "postgres pool".into(),
            relevant: vec!["bbbbbbb1".into()],
        },
        GoldenPair {
            query: "auth race".into(),
            relevant: vec!["ccccccc1".into()],
        },
    ];
    let merged = merge(file, harvested);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].query, "auth race", "output sorted by query text");
    assert_eq!(merged[1].query, "postgres pool");
    assert_eq!(
        merged[1].relevant,
        vec!["aaaaaaa1"],
        "file pair must win over harvested pair"
    );
}

#[test]
fn resolve_errors_when_no_pairs_exist() {
    let (_d, conn) = open_db();
    let err = resolve(&conn, None, false).expect_err("empty golden set must error");
    assert!(
        matches!(err, comemory::errors::Error::Unavailable(_)),
        "expected Unavailable, got: {err:?}"
    );
    assert!(
        err.to_string().contains("no golden pairs"),
        "error must explain the empty set, got: {err}"
    );
}

#[test]
fn resolve_golden_only_skips_harvest() {
    let (_d, conn) = open_db();
    insert_memory(&conn, "aaaaaaa1", "postgres pool exhausted fix", None);
    mark_used(&conn, "q-20260609-aabbccdd", "postgres pool", "aaaaaaa1");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("golden.yaml");
    std::fs::write(&path, "- query: auth race\n  relevant: [bbbbbbb1]\n").expect("write yaml");

    let pairs = resolve(&conn, Some(Path::new(&path)), true).expect("resolve");
    assert_eq!(pairs.len(), 1, "harvest must be skipped: {pairs:?}");
    assert_eq!(pairs[0].query, "auth race");
}
