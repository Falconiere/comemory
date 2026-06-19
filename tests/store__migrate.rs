//! Behavior tests for `crate::store::migrate` — the versioned schema
//! migration runner. Task 3 of the v0.2 plan introduces `run` and
//! `CURRENT_VERSION`; these tests assert that a fresh database is
//! brought to the current version on first call and that subsequent
//! calls are idempotent (no panics, no duplicate inserts, no version
//! regression). The M2 v5 tests cover the learning-loop migration:
//! new tables, the `search_stats` drop, and the run-once simhash
//! re-hash after `simhash::tokens` changed casing/folding. The M3 v6
//! tests cover the code-graph migration: extended edge rel kinds +
//! weight (table rebuild), rank/chunk columns, logged search filters,
//! and the `code_feedback` table.

use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn fresh_db_runs_all_migrations_to_current_version() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    migrate::run(&mut conn).expect("migrate");

    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |row| row.get(0),
        )
        .expect("read schema version");
    assert_eq!(version, migrate::CURRENT_VERSION);
}

#[test]
fn running_migrations_twice_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    migrate::run(&mut conn).expect("first run");
    migrate::run(&mut conn).expect("second run is a no-op");
}

/// `count(*)` over `sqlite_master`/`pragma_*` for the given `where`-less query
/// body. Shared by the migration assertions so each call site stays small.
fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count query")
}

/// Assert the v5 learning tables exist and `search_stats` was dropped — the
/// invariant shared by both the upgrade-in-place and v4→v5 migration tests.
fn assert_learning_tables_and_no_search_stats(conn: &Connection) {
    assert_eq!(
        count(
            conn,
            "SELECT count(*) FROM sqlite_master
              WHERE name IN ('feedback_events','query_expansions')",
        ),
        2,
        "learning tables should exist",
    );
    assert_eq!(
        count(
            conn,
            "SELECT count(*) FROM sqlite_master WHERE name='search_stats'",
        ),
        0,
        "search_stats should be dropped",
    );
}

/// Assert `schema_meta.version` landed on the latest schema. `migrate::run`
/// always migrates fully forward, so every migration test ends here.
fn assert_version_current(conn: &Connection) {
    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("version row");
    assert_eq!(v, migrate::CURRENT_VERSION);
}

/// Insert one `memories` row with a placeholder `simhash=0` so a re-hash pass
/// must overwrite it with the value aligned to the current `simhash::tokens`.
fn seed_unhashed_memory(conn: &Connection) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema,
                              content_hash, body, created_at, updated_at,
                              md_path, simhash)
         VALUES ('aaaaaaaa','cafe-notes','note','r','a',3,1,'h','Café notes',
                 '2026-01-01T00:00:00Z','2026-01-01T00:00:00Z',
                 'memories/aaaaaaaa-cafe-notes.md',0)",
        [],
    )
    .expect("seed memory");
}

/// Read the stored simhash of memory `id` as `u64`.
fn simhash_of(conn: &Connection, id: &str) -> u64 {
    let sh: i64 = conn
        .query_row("SELECT simhash FROM memories WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .expect("simhash");
    sh as u64
}

/// Collect the column names of `table` in a fresh migrated db, sorted.
fn column_names(conn: &Connection, table: &str) -> Vec<String> {
    let sql = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY name");
    let mut stmt = conn.prepare(&sql).expect("prepare pragma");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("query cols")
        .map(|c| c.expect("col name"))
        .collect()
}

#[test]
fn v9_creates_code_ref_table_with_expected_columns() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    migrate::run(&mut conn).expect("migrate");

    // PRAGMA table_info exposes exactly the spec'd anchor columns (sorted).
    assert_eq!(
        column_names(&conn, "code_ref"),
        [
            "branch",
            "created_at",
            "dst_id",
            "memory_id",
            "pinned_blob",
            "pinned_commit",
            "rel"
        ]
    );

    // The dst-side lookup index is present.
    let idx_exists: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type='index' AND name='idx_code_ref_dst'",
            [],
            |r| r.get(0),
        )
        .expect("index query");
    assert_eq!(idx_exists, 1, "idx_code_ref_dst should exist after v9");
}

#[test]
fn v5_adds_learning_tables_drops_search_stats_and_rehashes() {
    // Build a current database.
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let mut conn = connection::open(&db).expect("open runs migrations");

    // Seed a memory whose simhash under the OLD tokens() differs from the
    // new one (non-ASCII uppercase), with a simhash=0 placeholder so the v5
    // re-hash must produce the aligned value.
    seed_unhashed_memory(&conn);

    // Force the re-hash to run again as if upgrading: delete its marker.
    conn.execute(
        "DELETE FROM schema_meta WHERE key='0005_simhash_rehash'",
        [],
    )
    .expect("clear marker");
    migrate::run(&mut conn).expect("re-run migrations");

    assert_eq!(
        simhash_of(&conn, "aaaaaaaa"),
        comemory::simhash::of_body("Café notes")
    );

    // Learning tables exist; search_stats is gone.
    assert_learning_tables_and_no_search_stats(&conn);

    // retrieval_log gained duration_ms.
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM pragma_table_info('retrieval_log')
              WHERE name='duration_ms'",
        ),
        1,
        "retrieval_log should have duration_ms",
    );

    // Version bumped. `migrate::run` always lands on the latest schema,
    // so the v5 artifacts above coexist with every later migration.
    assert_version_current(&conn);
}

/// Build a genuine v4 database by replaying the 0001..0004 SQL exactly
/// as a v4 binary would have, including the `schema_meta` keys it
/// wrote. The 0002 DDL needs the process-global sqlite-vec
/// auto-extension (scratch `connection::open` registers it) and the
/// 0004 FTS rebuild needs the `identifier` tokenizer registered on
/// this raw connection. Seeds one memory and one code symbol carrying
/// stale (pre-M2-tokens) simhashes, plus a `search_stats` row that the
/// v5 DROP must discard.
fn build_v4_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    conn.execute_batch(migrate::M_BOOTSTRAP).expect("0001");
    conn.execute_batch(migrate::M_V2).expect("0002");
    conn.execute_batch(migrate::M_V3).expect("0003");
    conn.execute_batch(migrate::M_V4).expect("0004");
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'),
            ('0004_v4_rank','1'), ('version','4');
         INSERT INTO memories(id, slug, kind, repo, author, quality, schema,
                              content_hash, body, created_at, updated_at,
                              md_path, simhash)
         VALUES ('cafecafe','cafe-notes','note','demo','f',3,1,'h','Café notes',
                 '2026-01-01T00:00:00Z','2026-01-01T00:00:00Z',
                 'memories/cafecafe-cafe-notes.md', 1);
         INSERT INTO code_symbols(id, repo, path, blob_oid, symbol, kind, lang,
                                  line_start, line_end, snippet, simhash,
                                  indexed_at)
         VALUES (1,'demo','src/naive.rs','beef0000','naiveFn','function','rust',
                 1,3,'fn naïve_fn() {}',1,'2026-01-02T00:00:00Z');
         INSERT INTO search_stats(query, hit_count, duration_ms, ran_at)
         VALUES ('old query', 3, 12, '2026-01-03T00:00:00Z');",
    )
    .expect("seed v4 rows");
}

#[test]
fn open_migrates_v4_db_to_v5() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v4_db(&db);

    let conn = connection::open(&db).expect("open migrates");

    // Both stored simhashes were recomputed with the new tokens().
    assert_eq!(
        simhash_of(&conn, "cafecafe"),
        comemory::simhash::of_body("Café notes")
    );

    let code_sh: i64 = conn
        .query_row("SELECT simhash FROM code_symbols WHERE id=1", [], |r| {
            r.get(0)
        })
        .expect("code simhash");
    let toks = comemory::simhash::tokens("fn naïve_fn() {}");
    assert_eq!(
        code_sh as u64,
        comemory::simhash::simhash64(toks.iter().map(|t| t.as_str()))
    );

    // Learning tables present; search_stats dropped.
    assert_learning_tables_and_no_search_stats(&conn);

    // `connection::open` always migrates to the latest schema, so a
    // v4 db lands on CURRENT_VERSION (v5 + every later migration).
    assert_version_current(&conn);
}
