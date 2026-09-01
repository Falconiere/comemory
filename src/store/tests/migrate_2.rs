#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Behavior tests for `crate::store::migrate` (part 2 — the v6 migration
//! and v5→v6 upgrade path, the v11 `memories.rank_score` migration and its
//! v10→v11 upgrade path, the v12 `edge_fts` triplet index with its
//! v11→v12 upgrade path, and the v13 unified-document-indexing tables with
//! their v12→v13 upgrade path).

use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::tempdir;

/// The `schema_meta` value stored under `key`.
fn schema_meta(conn: &Connection, key: &str) -> String {
    conn.query_row("SELECT value FROM schema_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .expect("schema_meta value")
}

#[test]
fn v6_extends_edges_adds_code_graph_columns() {
    let tmp = tempdir().expect("tmpdir");
    let db = tmp.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v6");

    assert_v6_edge_rel_kinds(&conn);
    assert_v6_columns_and_tables(&conn);

    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
    assert_eq!(schema_meta(&conn, "code_format_version"), "2");
}

/// The extended v6 `edges` CHECK: the new rel kinds insert, a legacy kind
/// still defaults `weight` to 1, and an unknown kind is still rejected.
fn assert_v6_edge_rel_kinds(conn: &Connection) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
         VALUES('file','file:r:a.rs','file','file:r:b.rs','co_changed',3,'2026-01-01T00:00:00Z')",
        [],
    )
    .expect("co_changed edge");
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
         VALUES('file','file:r:a.rs','file','file:r:c.rs','imports',1,'2026-01-01T00:00:00Z')",
        [],
    )
    .expect("imports edge");
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at)
         VALUES('memory','m1','memory','m2','supersedes','2026-01-01T00:00:00Z')",
        [],
    )
    .expect("legacy kind, weight defaults to 1");
    let w: i64 = conn
        .query_row("SELECT weight FROM edges WHERE rel='supersedes'", [], |r| {
            r.get(0)
        })
        .expect("w");
    assert_eq!(w, 1);
    // an unknown rel still violates the CHECK
    assert!(
        conn.execute(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at)
             VALUES('a','a','b','b','bogus','2026-01-01T00:00:00Z')",
            []
        )
        .is_err()
    );
}

/// Every column and table the v6 migration adds is present.
fn assert_v6_columns_and_tables(conn: &Connection) {
    for (table, col) in [
        ("code_symbols", "rank_score"),
        ("code_symbols", "parent_id"),
        ("retrieval_log", "repo"),
        ("retrieval_log", "kind"),
        ("retrieval_log", "source"),
        ("feedback_events", "target_kind"),
        ("repo_marker", "last_mined_commit"),
        ("repo_marker", "root_path"),
    ] {
        let n: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name='{col}'"),
                [],
                |r| r.get(0),
            )
            .expect("col probe");
        assert_eq!(n, 1, "{table}.{col} missing");
    }
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='code_feedback'",
            [],
            |r| r.get(0),
        )
        .expect("table");
    assert_eq!(n, 1);
}

/// Build a genuine v5 database by replaying the 0001..0005 SQL exactly
/// as an M2 binary would have, including the `schema_meta` keys it
/// wrote (apply markers, run-once simhash markers, version=5). The
/// 0002 DDL needs the process-global sqlite-vec auto-extension and the
/// 0004 FTS rebuild needs the `identifier` tokenizer on this raw
/// connection. Seeds a pre-existing `supersedes` edge (the v6 edges
/// rebuild must carry it across with weight defaulting to 1) and a
/// `retrieval_log` row (the new `source` column default must backfill).
fn build_v5_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    conn.execute_batch(migrate::M_BOOTSTRAP).expect("0001");
    conn.execute_batch(migrate::M_V2).expect("0002");
    conn.execute_batch(migrate::M_V3).expect("0003");
    conn.execute_batch(migrate::M_V4).expect("0004");
    conn.execute_batch(migrate::M_V5).expect("0005");
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'),
            ('0004_v4_rank','1'), ('0004_simhash_backfill','1'),
            ('0005_v5_learning','1'), ('0005_simhash_rehash','1'),
            ('version','5');
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa1111','memory','bbbb2222','supersedes',
                 '2026-02-01T00:00:00Z');
         INSERT INTO edges(src_kind, src_id, dst_kind, dst_id, rel, created_at)
         VALUES ('memory','aaaa1111','repo','demo','in_repo',
                 '2026-02-01T00:00:00Z');
         INSERT INTO retrieval_log(query_id, query, returned_ids, at, duration_ms)
         VALUES ('q-1','vec dim mismatch','[\"aaaa1111\"]',
                 '2026-02-02T00:00:00Z', 12);",
    )
    .expect("seed v5 rows");
}

#[test]
fn open_migrates_v5_db_to_v6() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v5_db(&db);

    let conn = connection::open(&db).expect("open migrates");

    // Both pre-existing edges survived the table rebuild, weight = 1.
    let edges: Vec<(String, i64)> = conn
        .prepare("SELECT rel, weight FROM edges ORDER BY rel")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        edges,
        vec![("in_repo".to_string(), 1), ("supersedes".to_string(), 1)]
    );

    // The pre-v6 retrieval_log row reads back with the column default.
    let source: String = conn
        .query_row(
            "SELECT source FROM retrieval_log WHERE query_id='q-1'",
            [],
            |r| r.get(0),
        )
        .expect("source");
    assert_eq!(source, "search");

    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);

    // The rebuild must recreate both edge indexes — a dropped index would
    // silently degrade every edge walk rather than fail loudly.
    let idx: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index'
              AND name IN ('idx_edges_src','idx_edges_dst')",
            [],
            |r| r.get(0),
        )
        .expect("index probe");
    assert_eq!(idx, 2, "edge indexes missing after v6 rebuild");
}

/// Insert one `memories` row, omitting `rank_score` so the v11 column
/// default is what gets exercised. Every other NOT NULL column without a
/// default is supplied explicitly.
fn seed_memory(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, content_hash, body,
                              created_at, updated_at, md_path)
         VALUES(?1, 'note', 'decision', 'h', 'body',
                '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z', 'm/x.md')",
        [id],
    )
    .expect("seed memory row");
}

/// `memories.rank_score` for `id`.
fn rank_score(conn: &Connection, id: &str) -> f64 {
    conn.query_row("SELECT rank_score FROM memories WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .expect("rank_score")
}

#[test]
fn v11_adds_memories_rank_score_defaulting_to_zero() {
    let tmp = tempdir().expect("tmpdir");
    let db = tmp.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v11");

    let has_col: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('memories') WHERE name='rank_score'",
            [],
            |r| r.get(0),
        )
        .expect("col probe");
    assert_eq!(has_col, 1, "memories.rank_score missing");

    // A row written without the column reads back the neutral default —
    // the prior stays at 1.0 until the first recompute.
    seed_memory(&conn, "aaaa1111");
    assert_eq!(rank_score(&conn, "aaaa1111"), 0.0);

    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}

/// Every migration in apply order, each [`migrate::list::Migration`] projected
/// to its `(key, sql)` pair — the replay source [`build_legacy_db`] slices to
/// reconstruct an older schema. Only those two fields are needed here: replay
/// re-runs the SQL and writes the marker, while `class`/`post`/`markers` are
/// the runtime concern of `run()` and `preflight`. Sourced from `MIGRATIONS`
/// rather than hand-listed, so this harness cannot fall behind `run()` again.
/// `pub(crate)` so `store::migrate::backup`/`preflight`'s own colocated
/// tests can reuse it too, rather than hand-copying the replay logic
/// (Binding Rule 1).
pub(crate) fn migration_chain() -> Vec<(&'static str, &'static str)> {
    migrate::list::MIGRATIONS
        .iter()
        .map(|m| (m.key, m.sql))
        .collect()
}

/// Build a genuine pre-upgrade database by replaying the first `through`
/// migrations exactly as a binary of that vintage would have, including the
/// `schema_meta` keys it wrote. The 0002 DDL needs the process-global
/// sqlite-vec auto-extension and the 0004 FTS rebuild needs the
/// `identifier` tokenizer on this raw connection. Seeds one pre-upgrade
/// memory row so the additive v11 column has something to backfill.
///
/// 0001 is replayed but records no marker: `migrate::apply` special-cases
/// the bootstrap migration, whose SQL is `CREATE TABLE IF NOT EXISTS`. The
/// two simhash post-pass markers are seeded only when their migration was
/// actually replayed (`through >= 4` / `>= 5`) — seeding them unconditionally
/// would fabricate a state no binary ever wrote and suppress the
/// corresponding Rust post-pass on the next real `connection::open`.
///
/// `pub(crate)` so `store::migrate::backup`/`preflight`'s own colocated
/// tests can build the same historical fixtures (Binding Rule 1).
pub(crate) fn build_legacy_db(
    path: &std::path::Path,
    through: usize,
    version: &str,
    memory_id: &str,
) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    let chain = migration_chain();
    for (marker, sql) in &chain[..through] {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("replay {marker}: {e}"));
    }
    for (marker, _) in &chain[1..through] {
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES(?1, '1')",
            [marker],
        )
        .expect("seed migration marker");
    }
    if through >= 4 {
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES('0004_simhash_backfill','1')",
            [],
        )
        .expect("seed 0004 simhash backfill marker");
    }
    if through >= 5 {
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES('0005_simhash_rehash','1')",
            [],
        )
        .expect("seed 0005 simhash rehash marker");
    }
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES('version', ?1)",
        [version],
    )
    .expect("seed version");
    seed_memory(&conn, memory_id);
    if through >= 4 {
        // The 0004/0005 markers seeded above say the two simhash post-passes
        // already ran against every pre-existing row at this vintage — so a
        // real `comemory save` writing this row afterward would already
        // compute and store a genuine simhash, not the v4 `DEFAULT 0`
        // seed_memory's bare INSERT leaves behind. Overwrite it here so the
        // fixture matches what a real vintage database actually holds.
        seed_real_simhash(&conn, memory_id);
    }
}

/// Overwrite `memories.simhash` for `id` with a real value computed from its
/// stored body, the same way `store::memory_row::insert` would at save
/// time. Only meaningful once the column exists (v4+); [`build_legacy_db`]
/// is the sole caller and only calls it under that guard.
fn seed_real_simhash(conn: &Connection, id: &str) {
    let body: String = conn
        .query_row("SELECT body FROM memories WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .expect("seeded row body");
    let hash = comemory::simhash::of_body(&body) as i64;
    conn.execute(
        "UPDATE memories SET simhash = ?1 WHERE id = ?2",
        rusqlite::params![hash, id],
    )
    .expect("seed a real simhash");
}

#[test]
fn open_migrates_v10_db_to_v11_backfilling_rank_score() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_legacy_db(&db, 10, "10", "bbbb2222");

    let conn = connection::open(&db).expect("open migrates v10 -> v11");

    // The pre-v11 row gained the column at the neutral default: an
    // upgraded database ranks exactly as it did before v11.
    assert_eq!(rank_score(&conn, "bbbb2222"), 0.0);

    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}

#[test]
fn v11_migration_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let mut conn = connection::open(&db).expect("open runs v11");

    // A computed score must survive a second run — a re-applied ALTER would
    // error outright, and any backfill pass would reset this to 0.0.
    seed_memory(&conn, "cccc3333");
    conn.execute(
        "UPDATE memories SET rank_score = 0.375 WHERE id = 'cccc3333'",
        [],
    )
    .expect("write computed rank");

    migrate::run(&mut conn).expect("second migrate run is a no-op");

    assert_eq!(rank_score(&conn, "cccc3333"), 0.375);
    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}

/// Row count in `edge_fts`.
fn edge_fts_rows(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM edge_fts", [], |r| r.get(0))
        .expect("edge_fts row count")
}

/// Insert one rendered triplet directly, bypassing `edge_fts::refresh` —
/// these tests assert what the *migration* does, not what refresh renders.
fn seed_edge_fts(conn: &Connection, src_text: &str) {
    conn.execute(
        "INSERT INTO edge_fts(src_text, rel_text, dst_text, src_kind, src_id,
                              rel, dst_kind, dst_id, weight)
         VALUES(?1, 'supersedes', 'decision queue-v1', 'memory', 'aaaa1111',
                'supersedes', 'memory', 'bbbb2222', 1)",
        [src_text],
    )
    .expect("seed edge_fts row");
}

#[test]
fn v12_creates_edge_fts_empty_with_the_identifier_tokenizer() {
    let tmp = tempdir().expect("tmpdir");
    let db = tmp.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v12");

    // Created empty: the migration ships no backfill, because rendering
    // lives once in `store::edge_fts::refresh`.
    assert_eq!(edge_fts_rows(&conn), 0);

    // The `identifier` tokenizer is in force, not FTS5's default: it splits
    // `queue-design-v2` so a bare `design` term hits.
    seed_edge_fts(&conn, "decision queue-design-v2");
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM edge_fts WHERE edge_fts MATCH '\"design\"'",
            [],
            |r| r.get(0),
        )
        .expect("match probe");
    assert_eq!(hits, 1, "identifier tokenizer not applied to edge_fts");

    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}

#[test]
fn open_migrates_v11_db_to_v12_adding_edge_fts() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_legacy_db(&db, 11, "11", "dddd4444");

    // Pre-upgrade the table does not exist at all.
    let raw = Connection::open(&db).expect("open raw");
    let before: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='edge_fts'",
            [],
            |r| r.get(0),
        )
        .expect("table probe");
    assert_eq!(before, 0, "v11 replay should not have edge_fts");
    drop(raw);

    let conn = connection::open(&db).expect("open migrates v11 -> v12");

    // The upgraded database gains an EMPTY index; it self-heals on the
    // first `comemory edges` run rather than being backfilled in SQL.
    assert_eq!(edge_fts_rows(&conn), 0);
    assert_eq!(rank_score(&conn, "dddd4444"), 0.0);
    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}

#[test]
fn v12_migration_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let mut conn = connection::open(&db).expect("open runs v12");

    // Indexed content must survive a second run — a re-applied CREATE
    // VIRTUAL TABLE would error outright, and a backfill would wipe this.
    seed_edge_fts(&conn, "decision queue-design-v2");

    migrate::run(&mut conn).expect("second migrate run is a no-op");

    assert_eq!(edge_fts_rows(&conn), 1);
    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}

/// True when `name` exists in `sqlite_master` (any object type).
fn table_exists(conn: &Connection, name: &str) -> bool {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = ?1",
            [name],
            |r| r.get(0),
        )
        .expect("table probe");
    n == 1
}

/// Both `edges` lookup indexes exist — the invariant every `edges` rebuild
/// (0006, 0008, 0013, ...) must preserve.
fn assert_edge_indexes_exist(conn: &Connection) {
    let idx: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index'
              AND name IN ('idx_edges_src','idx_edges_dst')",
            [],
            |r| r.get(0),
        )
        .expect("index probe");
    assert_eq!(idx, 2, "edge indexes missing");
}

/// Every rel kind the `edges` CHECK accepted before v13, verbatim from the
/// 0008 rebuild (the last migration to touch the CHECK before this one).
const PRE_V13_REL_KINDS: [&str; 12] = [
    "in_repo",
    "authored_by",
    "tagged",
    "references_file",
    "references_symbol",
    "relates_to",
    "supersedes",
    "conflicts_with",
    "derived_from",
    "co_changed",
    "imports",
    "co_activated",
];

/// Insert one probe `edges` row using `kind` as both `rel` and the
/// src/dst id — `rel` is part of the primary key, so distinct kinds never
/// collide even against the same fixed 'probe'/'dst' pair. `kind` is
/// bound via `?1` (reused for both occurrences) rather than interpolated
/// into the SQL text.
fn insert_edge_kind(conn: &Connection, kind: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('probe',?1,'probe','dst',?1,'2026-01-01T00:00:00Z')",
        [kind],
    )
}

/// Every rel kind allowed before v13 still inserts, both new v13 kinds
/// insert, and an unknown kind is still rejected by the CHECK.
fn assert_v13_edges_rel_kinds(conn: &Connection) {
    for kind in PRE_V13_REL_KINDS {
        insert_edge_kind(conn, kind)
            .unwrap_or_else(|e| panic!("legacy rel kind {kind} should still insert: {e}"));
    }
    for kind in ["member_of_source", "references_document"] {
        insert_edge_kind(conn, kind)
            .unwrap_or_else(|e| panic!("new rel kind {kind} should insert: {e}"));
    }
    assert!(insert_edge_kind(conn, "bogus").is_err());
}

#[test]
fn v13_creates_document_tables_and_extends_edges_rel() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v13");

    for table in [
        "source_roots",
        "source_files",
        "documents",
        "document_chunks",
        "document_fts",
    ] {
        assert!(table_exists(&conn, table), "{table} missing after v13");
    }

    assert_v13_edges_rel_kinds(&conn);
    assert_edge_indexes_exist(&conn);

    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
    // Pin: bump this when CURRENT_VERSION advances past v15.
    assert_eq!(migrate::CURRENT_VERSION, "15");
}

/// The `document_fts` DDL uses the same per-connection `identifier`
/// tokenizer as `code_fts` (since 0004) and `edge_fts` (since 0012): a
/// hyphenated `path_tokens` value must split so a bare sub-word matches.
#[test]
fn v13_document_fts_uses_identifier_tokenizer() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v13");

    conn.execute(
        "INSERT INTO document_fts(document_id, ordinal, title, headings, passage, path_tokens)
         VALUES('doc1', 0, 'Queue Design', '', 'a passage', 'docs/queue-design-v2.md')",
        [],
    )
    .expect("seed document_fts row");

    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM document_fts WHERE document_fts MATCH '\"design\"'",
            [],
            |r| r.get(0),
        )
        .expect("match probe");
    assert_eq!(hits, 1, "identifier tokenizer not applied to document_fts");
}

/// Seed two pre-v13 `edges` rows directly against the raw db at `path` —
/// the v13 create-copy-drop-rename rebuild must carry both forward
/// verbatim. Opened and dropped eagerly so the caller reopens through the
/// real `connection::open` migration path afterward.
fn seed_pre_v13_edges(path: &std::path::Path) {
    let raw = Connection::open(path).expect("open raw v12 db");
    raw.execute_batch(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
         VALUES('memory','ffff6666','repo','demo','in_repo',1,'2026-03-01T00:00:00Z');
         INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
         VALUES('file','file:r:a.rs','file','file:r:b.rs','co_changed',5,
                '2026-03-01T00:00:00Z');",
    )
    .expect("seed pre-v13 edges");
}

/// `(src_kind, src_id, dst_kind, dst_id, rel, weight)` for every `edges`
/// row, ordered by `rel` for a stable comparison.
fn edges_snapshot(conn: &Connection) -> Vec<(String, String, String, String, String, i64)> {
    conn.prepare("SELECT src_kind, src_id, dst_kind, dst_id, rel, weight FROM edges ORDER BY rel")
        .expect("prepare")
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

#[test]
fn open_migrates_v12_db_to_v13_preserving_edges_losslessly() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_legacy_db(&db, 12, "12", "ffff6666");
    seed_pre_v13_edges(&db);

    // Pre-upgrade, none of the new tables exist yet.
    let before = Connection::open(&db).expect("open raw before upgrade");
    assert!(
        !table_exists(&before, "source_roots"),
        "v12 replay should not have source_roots yet"
    );
    drop(before);

    let conn = connection::open(&db).expect("open migrates v12 -> v13");

    assert_eq!(
        edges_snapshot(&conn),
        vec![
            (
                "file".to_string(),
                "file:r:a.rs".to_string(),
                "file".to_string(),
                "file:r:b.rs".to_string(),
                "co_changed".to_string(),
                5
            ),
            (
                "memory".to_string(),
                "ffff6666".to_string(),
                "repo".to_string(),
                "demo".to_string(),
                "in_repo".to_string(),
                1
            ),
        ]
    );

    assert_edge_indexes_exist(&conn);
    assert_eq!(schema_meta(&conn, "version"), migrate::CURRENT_VERSION);
}
