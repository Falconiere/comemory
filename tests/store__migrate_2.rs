//! Behavior tests for `crate::store::migrate` (part 2 — v6 migration
//! and v5→v6 upgrade path).

use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn v6_extends_edges_adds_code_graph_columns() {
    let tmp = tempdir().expect("tmpdir");
    let db = tmp.path().join("comemory.db");
    let conn = connection::open(&db).expect("open migrates to v6");

    // edges accepts the new rel kinds + weight, old kinds still work.
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

    // new columns + tables exist
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
    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("v");
    assert_eq!(v, "7");
    let fv: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='code_format_version'",
            [],
            |r| r.get(0),
        )
        .expect("fv");
    assert_eq!(fv, "2");
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

    let v: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='version'",
            [],
            |r| r.get(0),
        )
        .expect("version row");
    assert_eq!(v, "7");

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
