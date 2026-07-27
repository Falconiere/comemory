//! Behavior tests for `crate::store::migrate` (part 2 — the v6 migration
//! and v5→v6 upgrade path, plus the v11 `memories.rank_score` migration
//! and its v10→v11 upgrade path).

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

/// Build a genuine v10 database by replaying 0001..0010 exactly as a v10
/// binary would have, including the `schema_meta` keys it wrote. The 0002
/// DDL needs the process-global sqlite-vec auto-extension and the 0004 FTS
/// rebuild needs the `identifier` tokenizer on this raw connection. Seeds
/// one pre-v11 memory row, which the additive column must backfill to 0.0.
fn build_v10_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    for (label, sql) in [
        ("0001", migrate::M_BOOTSTRAP),
        ("0002", migrate::M_V2),
        ("0003", migrate::M_V3),
        ("0004", migrate::M_V4),
        ("0005", migrate::M_V5),
        ("0006", migrate::M_V6),
        ("0007", migrate::M_V7),
        ("0008", migrate::M_V8),
        ("0009", migrate::M_V9),
        ("0010", migrate::M_V10),
    ] {
        conn.execute_batch(sql)
            .unwrap_or_else(|e| panic!("replay {label}: {e}"));
    }
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'),
            ('0004_v4_rank','1'), ('0004_simhash_backfill','1'),
            ('0005_v5_learning','1'), ('0005_simhash_rehash','1'),
            ('0006_v6_code_graph','1'), ('0007_v7_repo_root','1'),
            ('0008_v8_reinforcement','1'), ('0009_v9_code_refs','1'),
            ('0010_v10_bandit','1'), ('version','10');",
    )
    .expect("seed v10 schema_meta");
    seed_memory(&conn, "bbbb2222");
}

#[test]
fn open_migrates_v10_db_to_v11_backfilling_rank_score() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v10_db(&db);

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
