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
    mark_used_filtered(conn, query_id, query, memory_id, None, None);
}

/// Like [`mark_used`] but recording the repo/kind filters the originating
/// search ran with (`None` → NULL, i.e. unfiltered).
fn mark_used_filtered(
    conn: &rusqlite::Connection,
    query_id: &str,
    query: &str,
    memory_id: &str,
    repo: Option<&str>,
    kind: Option<&str>,
) {
    conn.execute(
        "INSERT OR IGNORE INTO retrieval_log(query_id, query, returned_ids, at, duration_ms,
                                             repo, kind)
         VALUES (?1, ?2, '[]', '2026-06-09T00:00:00Z', 1, ?3, ?4)",
        rusqlite::params![query_id, query, repo, kind],
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
fn load_file_parses_repo_kind_and_defaults_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("golden.yaml");
    std::fs::write(
        &path,
        "- query: postgres pool\n  relevant: [aaaaaaa1]\n  repo: r\n  kind: decision\n\
         - query: auth race\n  relevant: [bbbbbbb1]\n",
    )
    .expect("write yaml");
    let pairs = load_file(&path).expect("load");
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].repo.as_deref(), Some("r"));
    assert_eq!(pairs[0].kind.as_deref(), Some("decision"));
    assert_eq!(
        pairs[1].repo, None,
        "pair without repo key must default None (backward compatible)"
    );
    assert_eq!(
        pairs[1].kind, None,
        "pair without kind key must default None (backward compatible)"
    );
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
fn load_file_missing_path_names_the_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("absent.yaml");
    let err = load_file(&path).expect_err("missing file must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("absent.yaml"),
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
fn harvest_carries_repo_and_kind_from_originating_search() {
    let (_d, conn) = open_db();
    insert_memory(&conn, "aaaaaaa1", "postgres pool exhausted fix", None);
    insert_memory(&conn, "aaaaaaa2", "postgres pool sizing decision", None);
    mark_used_filtered(
        &conn,
        "q-20260609-aabbccdd",
        "postgres pool",
        "aaaaaaa2",
        Some("r"),
        Some("decision"),
    );
    mark_used(&conn, "q-20260610-aabbccdd", "postgres pool", "aaaaaaa1");

    let pairs = harvest(&conn).expect("harvest");
    assert_eq!(
        pairs.len(),
        2,
        "same query under different filters must yield distinct pairs: {pairs:?}"
    );
    // BTreeMap key order: NULL repo/kind (None) sorts before Some.
    assert_eq!(pairs[0].query, "postgres pool");
    assert_eq!(pairs[0].repo, None, "unfiltered search must harvest None");
    assert_eq!(pairs[0].kind, None, "unfiltered search must harvest None");
    assert_eq!(pairs[0].relevant, vec!["aaaaaaa1"]);
    assert_eq!(pairs[1].query, "postgres pool");
    assert_eq!(pairs[1].repo.as_deref(), Some("r"));
    assert_eq!(pairs[1].kind.as_deref(), Some("decision"));
    assert_eq!(pairs[1].relevant, vec!["aaaaaaa2"]);
}

#[test]
fn harvest_excludes_code_target_events() {
    // Code feedback text-encodes the symbol id into feedback_events.memory_id
    // (the documented column-name wart). A symbol id like 12345678 is also a
    // valid 8-hex memory id, so without the target_kind='memory' guard a code
    // event could join a live memories row and pollute the golden harvest.
    let (_d, conn) = open_db();
    insert_memory(&conn, "12345678", "postgres pool exhausted fix", None);
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms)
         VALUES ('q-20260609-aabbccdd', 'postgres pool', '[]', '2026-06-09T00:00:00Z', 1)",
        [],
    )
    .expect("insert retrieval_log");
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES ('q-20260609-aabbccdd', '12345678', 'used', '2026-06-09T00:00:00Z', 'code')",
        [],
    )
    .expect("insert code feedback event");

    let pairs = harvest(&conn).expect("harvest");
    assert!(
        pairs.is_empty(),
        "code-target events must not seed golden pairs: {pairs:?}"
    );
}

#[test]
fn harvest_excludes_search_code_query_rows() {
    // A memory verdict manually recorded against a `search-code` query id
    // joins a retrieval_log row whose `kind` column carries a LANGUAGE
    // (the --lang filter), not a memory kind. Without the source guard the
    // harvest would mint a golden pair whose kind filter is `rust`, which
    // eval would replay as a memory-kind filter and score garbage.
    let (_d, conn) = open_db();
    insert_memory(&conn, "aaaaaaa1", "postgres pool exhausted fix", None);
    conn.execute(
        "INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms,
                                   repo, kind, source)
         VALUES ('q-20260609-aabbccdd', 'postgres pool', '[]', '2026-06-09T00:00:00Z', 1,
                 'r', 'rust', 'search-code')",
        [],
    )
    .expect("insert search-code retrieval_log row");
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES ('q-20260609-aabbccdd', 'aaaaaaa1', 'used', '2026-06-09T00:00:00Z', 'memory')",
        [],
    )
    .expect("insert memory verdict against the code query");

    let pairs = harvest(&conn).expect("harvest");
    assert!(
        pairs.is_empty(),
        "search-code query rows must not seed golden pairs: {pairs:?}"
    );
}

#[test]
fn merge_file_wins_on_duplicate_query_and_sorts() {
    let file = vec![GoldenPair {
        query: "postgres pool".into(),
        relevant: vec!["aaaaaaa1".into()],
        repo: None,
        kind: None,
    }];
    let harvested = vec![
        GoldenPair {
            query: "postgres pool".into(),
            relevant: vec!["bbbbbbb1".into()],
            repo: None,
            kind: None,
        },
        GoldenPair {
            query: "auth race".into(),
            relevant: vec!["ccccccc1".into()],
            repo: None,
            kind: None,
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
fn merge_keys_on_query_repo_and_kind() {
    let file = vec![GoldenPair {
        query: "postgres pool".into(),
        relevant: vec!["aaaaaaa1".into()],
        repo: None,
        kind: Some("decision".into()),
    }];
    let harvested = vec![GoldenPair {
        query: "postgres pool".into(),
        relevant: vec!["bbbbbbb1".into()],
        repo: None,
        kind: None,
    }];
    let merged = merge(file, harvested);
    assert_eq!(
        merged.len(),
        2,
        "file pair beats harvested only on full (query, repo, kind) match: {merged:?}"
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
