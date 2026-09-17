#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
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

// `set_version` is private; reached directly (rather than only indirectly
// through `migrate::run`) so its monotonicity guard is asserted against
// itself, per the Testing convention for a colocated private-item test.
use super::set_version;
// `apply` is private; reached directly below so a broken migration's
// rendered error message can be asserted without needing an actual shipped
// migration to fail (F7).
use super::apply;

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

    // toolu-orm supplies the schema declaration and the generate loop only;
    // its runner is never called, so its `_migrations` bookkeeping table
    // (the name `toolu_orm_cli::migrate::migrations_table_ddl` creates) must
    // never appear — `schema_meta` markers remain the one applied-set. If
    // upstream ever renames that table this assertion goes vacuous, which is
    // why it is paired with the runner never being linked in the first place.
    let orm_tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = '_migrations'",
            [],
            |row| row.get(0),
        )
        .expect("probe sqlite_master");
    assert_eq!(
        orm_tables, 0,
        "no toolu-orm `_migrations` table on a fresh database"
    );
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
fn v10_creates_bandit_arms_table() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");
    migrate::run(&mut conn).expect("migrate");

    assert_eq!(
        column_names(&conn, "bandit_arms"),
        [
            "alpha",
            "arm_id",
            "beta",
            "bm25_body",
            "bm25_tags",
            "decay",
            "last_mrr",
            "mmr_lambda",
            "pulls",
            "rrf_k",
            "updated_at",
        ]
    );
    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .expect("schema version");
    assert_eq!(v, migrate::CURRENT_VERSION);
    // Update this pin alongside the next CURRENT_VERSION change.
    assert_eq!(migrate::CURRENT_VERSION, "19");
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
        comemory::utilities::simhash::of_body("Café notes")
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
        comemory::utilities::simhash::of_body("Café notes")
    );

    let code_sh: i64 = conn
        .query_row("SELECT simhash FROM code_symbols WHERE id=1", [], |r| {
            r.get(0)
        })
        .expect("code simhash");
    let toks = comemory::utilities::simhash::tokens("fn naïve_fn() {}");
    assert_eq!(
        code_sh as u64,
        comemory::utilities::simhash::simhash64(toks.iter().map(std::string::String::as_str))
    );

    // Learning tables present; search_stats dropped.
    assert_learning_tables_and_no_search_stats(&conn);

    // `connection::open` always migrates to the latest schema, so a
    // v4 db lands on CURRENT_VERSION (v5 + every later migration).
    assert_version_current(&conn);
}

/// The `schema_meta.version` value stored under key `'version'`.
fn stored_version(conn: &Connection) -> String {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'version'",
        [],
        |r| r.get(0),
    )
    .expect("stored version")
}

/// F7 regression: a broken migration's `Error::Migration` message must stay
/// compact, not embed rusqlite's `SqlInputError` Display — which
/// interpolates the ENTIRE SQL batch verbatim (`"{msg} in {sql} at offset
/// {offset}"`) — now that `doctor` propagates this error to the operator
/// instead of swallowing it. The padded, invalid SQL below mimics a real
/// migration file's size (~200 lines for the larger migrations).
#[test]
fn a_broken_migration_batch_reports_a_compact_message_not_the_whole_sql_text() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    let padding = "-- padding comment line\n".repeat(60);
    let bogus_sql = format!("{padding}CREATE TABLE this is not valid sql;");

    let err = apply(&mut conn, "9999_bogus_migration", &bogus_sql)
        .expect_err("invalid SQL must fail to apply");
    let msg = err.to_string();

    assert!(
        msg.len() < 300,
        "error message must stay compact even for a large migration batch, got {} bytes: {msg}",
        msg.len()
    );
    assert!(
        !msg.contains("padding comment line"),
        "error message must not embed the migration's SQL text, got: {msg}"
    );
    assert!(
        msg.contains("9999_bogus_migration"),
        "error must still name the failing migration's key, got: {msg}"
    );
}

#[test]
fn set_version_refuses_to_lower_a_stored_version() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    // A crash-after-a-newer-migration (or a peer newer binary) left "99"
    // stored — newer than any CURRENT_VERSION this build will ever carry.
    conn.execute(
        "UPDATE schema_meta SET value = '99' WHERE key = 'version'",
        [],
    )
    .expect("seed a newer stored version");

    set_version(&conn, migrate::CURRENT_VERSION).expect("set_version must not error");

    assert_eq!(
        stored_version(&conn),
        "99",
        "a newer stored version must not be stomped downward"
    );
}

#[test]
fn set_version_raises_an_older_stored_version() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    // A pre-v0.11 (version "9") database's stored value predates
    // CURRENT_VERSION's two-digit width; the numeric compare, not a
    // lexical one, must still raise it.
    conn.execute(
        "UPDATE schema_meta SET value = '9' WHERE key = 'version'",
        [],
    )
    .expect("seed an older stored version");

    set_version(&conn, migrate::CURRENT_VERSION).expect("set_version must not error");

    assert_eq!(
        stored_version(&conn),
        migrate::CURRENT_VERSION,
        "an older stored version must be raised to current"
    );
}

#[test]
fn set_version_raises_an_unparsable_stored_version() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let conn = connection::open(&path).expect("open");

    // A stored value that fails to parse as u32 (corrupt row, or a
    // pre-numeric-version schema_meta layout) is treated as lower than any
    // real version and overwritten — per set_version's own doc comment.
    conn.execute(
        "UPDATE schema_meta SET value = 'unknown' WHERE key = 'version'",
        [],
    )
    .expect("seed an unparsable stored version");

    set_version(&conn, migrate::CURRENT_VERSION).expect("set_version must not error");

    assert_eq!(
        stored_version(&conn),
        migrate::CURRENT_VERSION,
        "an unparsable stored version must be raised to current, not left in place"
    );
}

#[test]
fn the_repush_migration_rewinds_the_push_cursor_exactly_once() {
    // AC-16: every memory the old push filter skipped sits BEHIND `pushed_seq`
    // (the empty-batch branch advanced past it), so removing the filter alone
    // would never re-offer it. The reset runs once and is then inert.
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    comemory::store::sync_state::ensure(&conn, "ws-1", "https://api.example").expect("ensure");
    comemory::store::sync_state::set_pushed(&conn, "ws-1", 42, "2026-09-14T00:00:00Z")
        .expect("set");

    // Stand in for a database written before v17: the cursor is where the old
    // push left it and the migration has not been applied.
    conn.execute("DELETE FROM schema_meta WHERE key='0017_sync_repush'", [])
        .expect("clear marker");
    migrate::run(&mut conn).expect("upgrade");

    let row = comemory::store::sync_state::get(&conn, "ws-1")
        .expect("get")
        .expect("row");
    assert_eq!(row.pushed_seq, 0, "the cursor must be rewound once");

    // A device that has since pushed again keeps its new cursor: the marker is
    // set, so a later run is a no-op rather than a second rewind.
    comemory::store::sync_state::set_pushed(&conn, "ws-1", 7, "2026-09-14T01:00:00Z").expect("set");
    migrate::run(&mut conn).expect("re-run");
    let row = comemory::store::sync_state::get(&conn, "ws-1")
        .expect("get")
        .expect("row");
    assert_eq!(
        row.pushed_seq, 7,
        "a second migration run must not rewind the cursor again"
    );
}

/// Seed one `edges` row with an explicit rel and endpoint kinds.
fn seed_edge(conn: &Connection, src_kind: &str, src: &str, dst_kind: &str, dst: &str, rel: &str) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES(?1,?2,?3,?4,?5,'t')",
        rusqlite::params![src_kind, src, dst_kind, dst, rel],
    )
    .expect("seed edge");
}

/// Every `dst_id` in `table` (ordered), for the before/after comparison.
fn dst_ids(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("SELECT dst_id FROM {table} ORDER BY dst_id"))
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<std::result::Result<Vec<String>, _>>()
        .expect("rows")
}

/// Issue #153: until v18, `cross_link` minted `references_*` edges from
/// `file:/…`, `./…` and `../…` path expressions — a pseudo-repo named after
/// the scheme that no store can resolve. The migration removes them from
/// all three places a reference lives (`edges`, its `code_ref` anchor, its
/// `edge_fts` triplet), keeps every repo-relative citation and every
/// non-reference edge, and is inert on a second run.
#[test]
fn the_scheme_path_migration_drops_junk_refs_and_keeps_real_ones() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("comemory.db");
    let mut conn = connection::open(&path).expect("open");

    // Junk: the three shapes the old guard let through.
    seed_edge(
        &conn,
        "memory",
        "m1",
        "file",
        "file:/tmp/check.db",
        "references_file",
    );
    seed_edge(
        &conn,
        "memory",
        "m1",
        "file",
        "sqlite:./data/local.db",
        "references_file",
    );
    seed_edge(
        &conn,
        "memory",
        "m1",
        "symbol",
        "file:../x/local.db:main",
        "references_symbol",
    );
    // Real: repo-relative citations, plus a code-graph edge that is not a
    // memory reference at all and must never be touched.
    seed_edge(
        &conn,
        "memory",
        "m1",
        "file",
        "demo:src/db.rs",
        "references_file",
    );
    seed_edge(
        &conn,
        "memory",
        "m1",
        "symbol",
        "demo:src/db.rs:run_migration",
        "references_symbol",
    );
    seed_edge(
        &conn,
        "file",
        "demo:src/a.rs",
        "file",
        "demo:src/b.rs",
        "imports",
    );
    // A colon-less destination is not the `<repo>:<path>` shape at all; the
    // migration requires the colon rather than judging the id's own first
    // character, so this row survives even though it starts with `/`.
    seed_edge(
        &conn,
        "memory",
        "m1",
        "file",
        "/no/colon/at/all.db",
        "references_file",
    );
    for dst in ["file:/tmp/check.db", "demo:src/db.rs"] {
        conn.execute(
            "INSERT INTO code_ref(memory_id, rel, dst_id, pinned_blob, created_at) \
             VALUES('m1', 'references_file', ?1, 'blob', 't')",
            [dst],
        )
        .expect("seed anchor");
    }
    let triplets = comemory::store::edge_fts::refresh(&mut conn).expect("materialize edge_fts");
    assert_eq!(
        triplets, 7,
        "every seeded edge is indexed before the migration"
    );

    // Stand in for a database written before v18: rows exist and the
    // migration has not been applied.
    conn.execute(
        "DELETE FROM schema_meta WHERE key='0018_scheme_path_refs'",
        [],
    )
    .expect("clear marker");
    migrate::run(&mut conn).expect("upgrade");

    let kept = [
        "/no/colon/at/all.db",
        "demo:src/b.rs",
        "demo:src/db.rs",
        "demo:src/db.rs:run_migration",
    ];
    assert_eq!(dst_ids(&conn, "edges"), kept, "only the junk references go");
    assert_eq!(
        dst_ids(&conn, "code_ref"),
        ["demo:src/db.rs"],
        "the junk anchor goes with it"
    );
    assert_eq!(
        dst_ids(&conn, "edge_fts"),
        kept,
        "the triplet index mirrors the edges"
    );

    // Marker set → a second run changes nothing.
    migrate::run(&mut conn).expect("re-run");
    assert_eq!(dst_ids(&conn, "edges"), kept);
    assert_eq!(dst_ids(&conn, "edge_fts"), kept);
}
