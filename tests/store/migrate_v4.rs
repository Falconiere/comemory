//! Upgrade tests for the 0004 migration: a database built by a pre-v4
//! binary (0001..0003 replayed verbatim) must gain the access-tracking
//! columns, a Rust-backfilled memory simhash, and identifier-tokenized
//! FTS tables on the next `connection::open`.

use rusqlite::Connection;

/// Build a pre-v4 database by replaying the 0001..0003 SQL exactly as an
/// old binary would have, with one memory row + old-tokenizer FTS row.
///
/// The 0002 DDL creates `vec0` virtual tables, so the process-global
/// sqlite-vec auto-extension must be registered first; a throwaway
/// `connection::open` on a scratch file takes care of that. The old FTS
/// tokenizers (`porter unicode61`, `unicode61`) are SQLite built-ins, so
/// the replay works on a raw `Connection` without the custom
/// `identifier` tokenizer registered.
fn build_v3_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(comemory::store::connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    conn.execute_batch(comemory::store::migrate::M_BOOTSTRAP)
        .expect("0001");
    conn.execute_batch(comemory::store::migrate::M_V2)
        .expect("0002");
    conn.execute_batch(comemory::store::migrate::M_V3)
        .expect("0003");
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'), ('version','3');
         INSERT INTO memories(id, slug, kind, repo, author, quality, schema,
                              content_hash, body, created_at, updated_at, md_path)
         VALUES ('aabbccdd','vec-dim','bug','demo','f',3,1,'hash',
                 'the VecDimMismatch error fires on bad embedder dims',
                 '2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','memories/aabbccdd-vec-dim.md');
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('aabbccdd','the VecDimMismatch error fires on bad embedder dims','');",
    )
    .expect("seed v3 rows");
}

#[test]
fn open_migrates_v3_db_to_v4() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v3_db(&db);

    let conn = comemory::store::connection::open(&db).expect("open migrates");

    let (count, last, sim): (i64, String, i64) = conn
        .query_row(
            "SELECT access_count, last_accessed, simhash FROM memories WHERE id='aabbccdd'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("columns present");
    assert_eq!(count, 0);
    assert_eq!(last, "2026-01-01T00:00:00Z"); // backfilled from created_at
    assert_ne!(sim, 0); // Rust backfill computed a real simhash

    // code_symbols gained the access columns too (table is empty here).
    let code_cols: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('code_symbols')
              WHERE name IN ('access_count','last_accessed')",
            [],
            |r| r.get(0),
        )
        .expect("code_symbols columns");
    assert_eq!(code_cols, 2);

    // FTS rebuilt with identifier tokenizer: camelCase subtoken now matches
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'mismatch'",
            [],
            |r| r.get(0),
        )
        .expect("fts query");
    assert_eq!(hits, 1);

    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("version row");
    assert_eq!(v, "4");
}

#[test]
fn migration_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v3_db(&db);
    drop(comemory::store::connection::open(&db).expect("first open"));
    let conn = comemory::store::connection::open(&db).expect("second open");
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'mismatch'",
            [],
            |r| r.get(0),
        )
        .expect("fts still works");
    assert_eq!(hits, 1);
}
