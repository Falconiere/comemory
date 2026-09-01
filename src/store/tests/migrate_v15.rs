#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Upgrade + behavior tests for the 0015 migration (console-API additions:
//! `index_runs`, `eval_runs.discarded`, `repo_marker.archived`). Same
//! three-way shape as the v14 suite, against a REAL SQLite database:
//!
//!   * fresh DB — the table and both columns exist, accept a row, and the
//!     new columns default to 0;
//!   * upgrade path — a genuine v14 database (0001..0014 replayed
//!     verbatim, seeded with a memory, an eval run and a repo marker) gains
//!     all three while every pre-existing row survives with its new column
//!     at the default (console-api spec AC-20);
//!   * idempotency — running the chain twice is a no-op and the marker is
//!     written exactly once.

use comemory::store::migrate::list::MIGRATIONS;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::tempdir;

/// Build a genuine v14 database by replaying `0001..=0014` verbatim, exactly
/// as the previous release's binary created one (see `migrate_v14.rs` for
/// why the scratch open and the tokenizer registration are load-bearing).
fn build_v14_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    for m in MIGRATIONS.iter().take(14) {
        conn.execute_batch(m.sql)
            .unwrap_or_else(|e| panic!("replay {}: {e}", m.key));
        for marker in m.markers {
            conn.execute(
                "INSERT OR IGNORE INTO schema_meta(key, value) VALUES (?1, '1')",
                [marker],
            )
            .unwrap();
        }
    }
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('version', '14')",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO memories(id, slug, kind, body, created_at, updated_at, content_hash,
                              schema, md_path)
         VALUES ('11223344', '11223344-keep-me', 'decision', 'keep me across the upgrade',
                 '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z', 'abc', 1,
                 'memories/11223344-keep-me.md')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO eval_runs(id, kind, at, golden_pairs, k, recall, mrr, knobs, applied)
         VALUES ('a1b2c3d4e5f60718', 'tune', '2026-08-31T10:00:00Z', 96, 10, 0.78, 0.61,
                 '{\"rrf_k\":60.0}', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repo_marker(repo, last_head, last_indexed_at, root_path)
         VALUES ('demo', 'abc123', '2026-08-31T10:00:00Z', '/tmp/demo')",
        [],
    )
    .unwrap();
    assert!(
        conn.query_row("SELECT 1 FROM index_runs LIMIT 1", [], |_| Ok(()))
            .is_err(),
        "a v14 database must not already carry index_runs"
    );
}

#[test]
fn v15_creates_index_runs_and_the_flag_columns_on_a_fresh_database() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();

    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        version,
        migrate::CURRENT_VERSION,
        "fresh DB reports this build's schema version"
    );

    conn.execute(
        "INSERT INTO index_runs(id, repo, root_path, mode, started_at, finished_at,
                                duration_ms, files_indexed, symbols, outcome, error)
         VALUES ('0f1e2d3c4b5a6978', 'demo', '/tmp/demo', 'full', '2026-09-01T10:00:00Z',
                 '2026-09-01T10:00:02Z', 2000, 12, 340, 'ok', NULL)",
        [],
    )
    .unwrap();
    let (files, symbols): (i64, i64) = conn
        .query_row(
            "SELECT files_indexed, symbols FROM index_runs WHERE id = '0f1e2d3c4b5a6978'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((files, symbols), (12, 340));

    conn.execute(
        "INSERT INTO eval_runs(id, kind, at, golden_pairs, k, recall, mrr, knobs)
         VALUES ('a1b2c3d4e5f60718', 'eval', '2026-08-31T10:00:00Z', 96, 10, 0.78, 0.61, '{}')",
        [],
    )
    .unwrap();
    let discarded: i64 = conn
        .query_row("SELECT discarded FROM eval_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(discarded, 0, "discarded defaults to 0");

    conn.execute("INSERT INTO repo_marker(repo) VALUES ('demo')", [])
        .unwrap();
    let archived: i64 = conn
        .query_row("SELECT archived FROM repo_marker", [], |r| r.get(0))
        .unwrap();
    assert_eq!(archived, 0, "archived defaults to 0");
}

#[test]
fn v15_rejects_an_unknown_mode_or_outcome() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();
    let bad_outcome = conn.execute(
        "INSERT INTO index_runs(id, repo, mode, started_at, finished_at, duration_ms,
                                files_indexed, symbols, outcome)
         VALUES ('deadbeefdeadbeef', 'demo', 'full', 't', 't', 0, 0, 0, 'meh')",
        [],
    );
    assert!(bad_outcome.is_err(), "outcome CHECK");
    let bad_mode = conn.execute(
        "INSERT INTO index_runs(id, repo, mode, started_at, finished_at, duration_ms,
                                files_indexed, symbols, outcome)
         VALUES ('deadbeefdeadbeef', 'demo', 'partial', 't', 't', 0, 0, 0, 'ok')",
        [],
    );
    assert!(bad_mode.is_err(), "mode CHECK");
}

#[test]
fn v15_upgrades_a_real_v14_database_without_losing_rows() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    build_v14_db(&db);

    let conn = connection::open(&db).expect("open migrates v14 -> v15");

    let memories: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(memories, 1, "the upgrade preserved the seeded memory row");

    let (applied, discarded): (i64, i64) = conn
        .query_row(
            "SELECT applied, discarded FROM eval_runs WHERE id = 'a1b2c3d4e5f60718'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (applied, discarded),
        (0, 0),
        "existing run gains discarded = 0"
    );

    let (root, archived): (String, i64) = conn
        .query_row(
            "SELECT root_path, archived FROM repo_marker WHERE repo = 'demo'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(root, "/tmp/demo");
    assert_eq!(archived, 0, "existing marker gains archived = 0");

    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM index_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runs, 0, "the new table exists and starts empty");

    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, migrate::CURRENT_VERSION);
}

#[test]
fn v15_is_idempotent() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    let mut conn = connection::open(&db).expect("open runs the chain");
    conn.execute(
        "INSERT INTO index_runs(id, repo, mode, started_at, finished_at, duration_ms,
                                files_indexed, symbols, outcome)
         VALUES ('1111222233334444', 'demo', 'incremental', 't', 't', 1, 2, 3, 'ok')",
        [],
    )
    .unwrap();

    migrate::run(&mut conn).unwrap();

    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM index_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        runs, 1,
        "a second run neither recreates nor truncates the table"
    );
    let markers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_meta WHERE key = '0015_v15_console_api'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(markers, 1, "the marker is written exactly once");
}
