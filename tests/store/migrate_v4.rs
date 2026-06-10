//! Upgrade tests for the 0004 migration: a database built by a pre-v4
//! binary (0001..0003 replayed verbatim) must gain the access-tracking
//! columns, a Rust-backfilled memory simhash, and identifier-tokenized
//! FTS tables on the next `connection::open`.

use rusqlite::Connection;

/// Build a pre-v4 database by replaying the 0001..0003 SQL exactly as an
/// old binary would have: one live memory (with old-tokenizer FTS row,
/// tag, edge, and 1024-dim vector), one soft-deleted memory with a stale
/// FTS row, and one code symbol with an old-tokenizer `code_fts` row.
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
         VALUES ('aabbccdd','the VecDimMismatch error fires on bad embedder dims','');
         INSERT INTO memory_tags(memory_id, tag) VALUES ('aabbccdd','postgres');
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aabbccdd','repo','demo','in_repo','2026-01-01T00:00:00Z');
         INSERT INTO memories(id, slug, kind, repo, author, quality, schema,
                              content_hash, body, created_at, updated_at, deleted_at, md_path)
         VALUES ('deadbeef','ghost','note','demo','f',3,1,'hash2',
                 'tombstoned ghost body that must vanish from fts',
                 '2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','2026-01-02T00:00:00Z',
                 'memories/deadbeef-ghost.md');
         INSERT INTO memory_fts(memory_id, body, tags)
         VALUES ('deadbeef','tombstoned ghost body that must vanish from fts','');
         INSERT INTO code_symbols(id, repo, path, blob_oid, symbol, kind, lang,
                                  line_start, line_end, snippet, simhash, indexed_at)
         VALUES (1,'demo','src/parse_config.rs','beef0000','parseConfig','function','rust',
                 1,3,'fn parse_config() {}',42,'2026-01-02T00:00:00Z');
         INSERT INTO code_fts(symbol_id, symbol, snippet, path_tokens)
         VALUES (1,'parseConfig','fn parse_config() {}','src parse config rs');",
    )
    .expect("seed v3 rows");
    // One 1024-dim memory vector; the migration must leave memory_vec alone.
    let blob = comemory::store::embed::to_vec_blob(&vec![0.03125_f32; 1024]);
    conn.execute(
        "INSERT INTO memory_vec(memory_id, embedding) VALUES ('aabbccdd', ?1)",
        rusqlite::params![blob],
    )
    .expect("seed memory_vec");
}

/// Count helper for single-integer assertions against the migrated DB.
fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count query")
}

#[test]
fn open_migrates_v3_db_to_v4() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v3_db(&db);

    let conn = comemory::store::connection::open(&db).expect("open migrates");

    let (accesses, last, sim): (i64, String, i64) = conn
        .query_row(
            "SELECT access_count, last_accessed, simhash FROM memories WHERE id='aabbccdd'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("columns present");
    assert_eq!(accesses, 0);
    assert_eq!(last, "2026-01-01T00:00:00Z"); // backfilled from created_at
    assert_ne!(sim, 0); // Rust backfill computed a real simhash

    // code_symbols gained the access columns; last_accessed backfilled
    // from indexed_at.
    let (code_count, code_last): (i64, String) = conn
        .query_row(
            "SELECT access_count, last_accessed FROM code_symbols WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("code_symbols columns");
    assert_eq!(code_count, 0);
    assert_eq!(code_last, "2026-01-02T00:00:00Z");

    // Untouched-by-migration rows survive: vector, edge, tag.
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM memory_vec WHERE memory_id = 'aabbccdd'"
        ),
        1
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM edges"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM memory_tags"), 1);

    // FTS rebuilt with identifier tokenizer: camelCase subtoken now matches
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'mismatch'"
        ),
        1
    );
    // ...the rebuilt tags column carries the memory_tags row...
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'postgres'"
        ),
        1
    );
    // ...the soft-deleted memory is excluded from the rebuilt index...
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'tombstoned'"
        ),
        0
    );
    // ...and code_fts finds the symbol via a camelCase subtoken. The
    // column-scoped query is the discriminating form: under the old
    // unicode61 tokenizer `parseConfig` is a single symbol token, so
    // `symbol:config` only matches after the identifier rebuild.
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM code_fts WHERE code_fts MATCH 'symbol:config'"
        ),
        1
    );

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
