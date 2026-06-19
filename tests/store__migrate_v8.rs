//! Upgrade + behavior tests for the 0008 migration (auto-reinforcement
//! schema). The `edges` table is rebuilt to add the `co_activated` rel
//! kind to its `rel` CHECK, and `feedback_events` gains a `provenance`
//! column defaulting to `'manual'`. These tests assert three things on a
//! REAL SQLite database:
//!
//!   * fresh DB — after the full migration, `co_activated` and every
//!     prior rel kind insert successfully, an unknown rel still violates
//!     the CHECK, and `feedback_events.provenance` defaults to `'manual'`;
//!   * upgrade path — a genuine v7 database (0001..0007 replayed
//!     verbatim, seeded with one edge per existing rel + feedback rows)
//!     carries every edge row across the rebuild (count + a sampled row),
//!     keeps both edge indexes, and backfills `provenance='manual'` onto
//!     pre-0008 feedback rows;
//!   * idempotency — running the migration twice is a no-op.

use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::tempdir;

/// Every `rel` kind allowed by the v6/v7 `edges` CHECK, i.e. the full set
/// that must survive the v8 rebuild unchanged.
const PRE_V8_RELS: &[&str] = &[
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
];

#[test]
fn v8_extends_edges_check_and_adds_feedback_provenance() {
    let tmp = tempdir().expect("tmpdir");
    let db = tmp.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v8");

    // The new rel kind is accepted.
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
         VALUES('memory','m1','file','file:r:a.rs','co_activated',2,'2026-01-01T00:00:00Z')",
        [],
    )
    .expect("co_activated edge");

    // Every prior rel kind still inserts.
    for (i, rel) in PRE_V8_RELS.iter().enumerate() {
        conn.execute(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at)
             VALUES('memory', ?1, 'memory', ?2, ?3, '2026-01-01T00:00:00Z')",
            rusqlite::params![format!("s{i}"), format!("d{i}"), rel],
        )
        .unwrap_or_else(|e| panic!("prior rel {rel} must still insert: {e}"));
    }

    // An unknown rel still violates the CHECK.
    assert!(
        conn.execute(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at)
             VALUES('a','a','b','b','bogus','2026-01-01T00:00:00Z')",
            []
        )
        .is_err(),
        "unknown rel must violate the extended CHECK"
    );

    // feedback_events has provenance defaulting to 'manual'.
    assert_feedback_provenance_defaults(&conn);

    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("v");
    assert_eq!(v, migrate::CURRENT_VERSION);
    assert_eq!(migrate::CURRENT_VERSION, "9");
}

/// After v8 migration, `feedback_events.provenance` exists and defaults to
/// `'manual'` when a row is inserted without it.
fn assert_feedback_provenance_defaults(conn: &rusqlite::Connection) {
    let has_col: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('feedback_events')
              WHERE name='provenance'",
            [],
            |r| r.get(0),
        )
        .expect("provenance col probe");
    assert_eq!(has_col, 1, "feedback_events.provenance missing");

    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at)
         VALUES('q1','m1','used','2026-01-01T00:00:00Z')",
        [],
    )
    .expect("insert feedback omitting provenance");
    let prov: String = conn
        .query_row(
            "SELECT provenance FROM feedback_events WHERE query_id='q1'",
            [],
            |r| r.get(0),
        )
        .expect("provenance");
    assert_eq!(
        prov, "manual",
        "omitted provenance must default to 'manual'"
    );
}

/// Build a genuine v7 database by replaying the 0001..0007 SQL exactly as
/// a v7 binary would have, including the `schema_meta` keys it wrote
/// (apply markers, run-once simhash markers, version=7). The 0002 DDL
/// needs the process-global sqlite-vec auto-extension and the 0004 FTS
/// rebuild needs the `identifier` tokenizer on this raw connection. Seeds
/// one `edges` row per pre-v8 rel kind (the v8 rebuild must carry every
/// one across, preserving weight + created_at) and two `feedback_events`
/// rows (the new `provenance` column default must backfill them).
fn build_v7_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    conn.execute_batch(migrate::M_BOOTSTRAP).expect("0001");
    conn.execute_batch(migrate::M_V2).expect("0002");
    conn.execute_batch(migrate::M_V3).expect("0003");
    conn.execute_batch(migrate::M_V4).expect("0004");
    conn.execute_batch(migrate::M_V5).expect("0005");
    conn.execute_batch(migrate::M_V6).expect("0006");
    conn.execute_batch(migrate::M_V7).expect("0007");
    conn.execute_batch(
        "INSERT INTO schema_meta(key, value) VALUES
            ('0002_v2_tables','1'), ('0003_stats_tables','1'),
            ('0004_v4_rank','1'), ('0004_simhash_backfill','1'),
            ('0005_v5_learning','1'), ('0005_simhash_rehash','1'),
            ('0006_v6_code_graph','1'), ('0007_v7_repo_root','1'),
            ('version','7');",
    )
    .expect("seed v7 schema_meta");

    seed_pre_v8_edges(&conn);
    seed_v7_feedback(&conn);
}

/// Seed one `edges` row per pre-v8 rel kind, each with a distinct weight +
/// created_at so the v8 rebuild's column copy can be checked exactly.
fn seed_pre_v8_edges(conn: &Connection) {
    let mut insert = conn
        .prepare(
            "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,weight,created_at)
             VALUES('memory', ?1, 'memory', ?2, ?3, ?4, ?5)",
        )
        .expect("prepare edge seed");
    for (i, rel) in PRE_V8_RELS.iter().enumerate() {
        insert
            .execute(rusqlite::params![
                format!("src{i}"),
                format!("dst{i}"),
                rel,
                (i as i64) + 1,
                format!("2026-02-0{}T00:00:00Z", i % 9 + 1),
            ])
            .unwrap_or_else(|e| panic!("seed edge {rel}: {e}"));
    }
}

/// Seed two pre-0008 `feedback_events` rows (no `provenance` column yet) so
/// the v8 `provenance` default must backfill them to `'manual'`.
fn seed_v7_feedback(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, target_kind)
         VALUES ('q-old','aaaa1111','used','2026-02-01T00:00:00Z','memory'),
                ('q-old','bbbb2222','irrelevant','2026-02-01T00:00:00Z','memory');",
    )
    .expect("seed feedback_events");
}

#[test]
fn open_migrates_v7_db_to_v8_preserving_edges_and_indexes() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    build_v7_db(&db);

    let conn = connection::open(&db).expect("open migrates v7 -> v8");

    assert_edges_preserved(&conn);

    // The new rel kind is accepted post-upgrade.
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at)
         VALUES('memory','m9','file','file:r:z.rs','co_activated','2026-03-01T00:00:00Z')",
        [],
    )
    .expect("co_activated edge accepted after upgrade");

    assert_indexes_present(&conn);

    // Pre-0008 feedback rows now read provenance='manual'.
    let manual: i64 = conn
        .query_row(
            "SELECT count(*) FROM feedback_events
              WHERE query_id='q-old' AND provenance='manual'",
            [],
            |r| r.get(0),
        )
        .expect("provenance backfill probe");
    assert_eq!(
        manual, 2,
        "pre-0008 feedback rows must backfill to 'manual'"
    );

    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("version row");
    assert_eq!(v, migrate::CURRENT_VERSION);
}

/// Assert every seeded pre-v8 edge row survived the v8 rebuild: the total
/// count, the full `(rel, weight, created_at)` set verbatim, and a sampled
/// `supersedes` row intact with its weight.
fn assert_edges_preserved(conn: &Connection) {
    // Every seeded edge row survived the rebuild (count).
    let count: i64 = conn
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .expect("edge count");
    assert_eq!(
        count as usize,
        PRE_V8_RELS.len(),
        "all pre-v8 edge rows must survive the rebuild"
    );

    // The full (rel, weight, created_at) set carried across verbatim.
    let mut rows: Vec<(String, i64, String)> = conn
        .prepare("SELECT rel, weight, created_at FROM edges")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    rows.sort();
    let mut expected: Vec<(String, i64, String)> = PRE_V8_RELS
        .iter()
        .enumerate()
        .map(|(i, rel)| {
            (
                (*rel).to_string(),
                (i as i64) + 1,
                format!("2026-02-0{}T00:00:00Z", i % 9 + 1),
            )
        })
        .collect();
    expected.sort();
    assert_eq!(rows, expected, "edge columns must be preserved exactly");

    // A sampled row (the supersedes edge) is intact with its weight.
    let sampled: (String, String, i64) = conn
        .query_row(
            "SELECT src_id, dst_id, weight FROM edges WHERE rel='supersedes'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("sampled supersedes row");
    let idx = PRE_V8_RELS
        .iter()
        .position(|r| *r == "supersedes")
        .expect("idx");
    assert_eq!(
        sampled,
        (format!("src{idx}"), format!("dst{idx}"), (idx as i64) + 1)
    );
}

/// Assert both edge indexes (`idx_edges_src`, `idx_edges_dst`) were
/// recreated by the v8 rebuild, via both `sqlite_master` and `index_list`.
fn assert_indexes_present(conn: &Connection) {
    // Both edge indexes were recreated by the rebuild.
    let idx_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index'
              AND name IN ('idx_edges_src','idx_edges_dst')",
            [],
            |r| r.get(0),
        )
        .expect("index probe");
    assert_eq!(idx_count, 2, "both edge indexes missing after v8 rebuild");

    // PRAGMA index_list also reports both indexes on the rebuilt table.
    let listed: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_index_list('edges')
              WHERE name IN ('idx_edges_src','idx_edges_dst')",
            [],
            |r| r.get(0),
        )
        .expect("index_list probe");
    assert_eq!(listed, 2, "index_list must report both edge indexes");
}

/// Seed an edge using the new rel + a provenance-tagged feedback row so a
/// second migrate run would surface any non-idempotent re-application (a
/// re-run table rebuild would lose the edge; a re-ADD COLUMN would error).
fn seed_idempotency_probe_rows(conn: &Connection) {
    conn.execute(
        "INSERT INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at)
         VALUES('memory','keep','file','file:r:k.rs','co_activated','2026-04-01T00:00:00Z')",
        [],
    )
    .expect("seed co_activated edge");
    conn.execute(
        "INSERT INTO feedback_events(query_id, memory_id, verdict, at, provenance)
         VALUES('q-keep','keep','used','2026-04-01T00:00:00Z','reinforce')",
        [],
    )
    .expect("seed provenance-tagged feedback");
}

#[test]
fn v8_migration_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("comemory.db");
    let mut conn = connection::open(&db).expect("open runs v8");

    seed_idempotency_probe_rows(&conn);

    migrate::run(&mut conn).expect("second migrate run is a no-op");

    // The seeded edge survived (the rebuild did not re-run).
    let edge: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_id='keep' AND rel='co_activated'",
            [],
            |r| r.get(0),
        )
        .expect("edge survives");
    assert_eq!(edge, 1, "idempotent re-run must not rebuild edges");

    // The explicit provenance was not clobbered back to 'manual'.
    let prov: String = conn
        .query_row(
            "SELECT provenance FROM feedback_events WHERE query_id='q-keep'",
            [],
            |r| r.get(0),
        )
        .expect("provenance preserved");
    assert_eq!(
        prov, "reinforce",
        "idempotent re-run must not reset provenance"
    );

    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("version row");
    assert_eq!(v, migrate::CURRENT_VERSION);
}
