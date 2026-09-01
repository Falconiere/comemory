#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Upgrade + behavior tests for the 0014 migration (console history
//! tables). `eval_runs` and `gc_runs` are both new, additive tables, so
//! these tests assert three things against a REAL SQLite database:
//!
//!   * fresh DB — after the full migration chain both tables exist, accept
//!     a real row, reject an out-of-set `eval_runs.kind`, and default
//!     `applied` to 0;
//!   * upgrade path — a genuine v13 database (0001..0013 replayed
//!     verbatim, seeded with memory and edge rows) gains both tables while
//!     every pre-existing row survives, which is what makes the migration
//!     Additive rather than Destructive;
//!   * idempotency — running the chain twice is a no-op and the marker is
//!     written exactly once.

use comemory::store::migrate::list::MIGRATIONS;
use comemory::store::{connection, migrate};
use rusqlite::Connection;
use tempfile::tempdir;

/// Build a genuine v13 database by replaying `0001..=0013` verbatim, exactly
/// as the previous release's binary created one. Deliberately does not call
/// `migrate::run` — that would apply 0014 too and leave no pre-v14 state to
/// upgrade.
///
/// The scratch open is load-bearing and follows the `migrate_v8` precedent:
/// `sqlite-vec` is registered as a process-global SQLite auto-extension by
/// `store::connection::open`, and the identifier FTS5 tokenizer is
/// registered per connection. A raw `Connection` has neither, so 0002's
/// `vec0` virtual tables and 0004's tokenized FTS rebuild would both fail.
fn build_v13_db(path: &std::path::Path) {
    let scratch = path.with_file_name("scratch-vec-register.db");
    drop(connection::open(&scratch).expect("register sqlite-vec"));

    let conn = Connection::open(path).expect("open raw");
    comemory::store::tokenizer::ffi::register(&conn).expect("register identifier tokenizer");
    for m in MIGRATIONS.iter().take(13) {
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
        "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('version', '13')",
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
        "INSERT INTO edges(src_kind, src_id, rel, dst_kind, dst_id, weight, created_at)
         VALUES ('memory', '11223344', 'relates_to', 'memory', '55667788', 1,
                 '2026-08-01T00:00:00Z')",
        [],
    )
    .unwrap();
    assert!(
        conn.query_row("SELECT 1 FROM eval_runs LIMIT 1", [], |_| Ok(()))
            .is_err(),
        "a v13 database must not already carry eval_runs"
    );
}

#[test]
fn v14_creates_both_history_tables_on_a_fresh_database() {
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
        "INSERT INTO eval_runs(id, kind, at, golden_pairs, k, recall, mrr, knobs)
         VALUES ('a1b2c3d4e5f60718', 'eval', '2026-08-31T10:00:00Z', 96, 10, 0.78, 0.61,
                 '{\"rrf_k\":60.0}')",
        [],
    )
    .unwrap();

    let (recall, mrr, applied): (f64, f64, i64) = conn
        .query_row(
            "SELECT recall, mrr, applied FROM eval_runs WHERE id = 'a1b2c3d4e5f60718'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(recall, 0.78);
    assert_eq!(mrr, 0.61);
    assert_eq!(
        applied, 0,
        "applied defaults to 0 when the run only reported"
    );

    conn.execute(
        "INSERT INTO gc_runs(id, at, removed, log_rows, event_rows, bytes_freed)
         VALUES ('0f1e2d3c4b5a6978', '2026-08-31T10:05:00Z', 12, 340, 88, 7340032)",
        [],
    )
    .unwrap();
    let bytes: i64 = conn
        .query_row("SELECT bytes_freed FROM gc_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bytes, 7_340_032);
}

#[test]
fn v14_rejects_an_unknown_eval_run_kind() {
    let dir = tempdir().unwrap();
    let conn = connection::open(dir.path().join("comemory.db")).unwrap();

    let err = conn.execute(
        "INSERT INTO eval_runs(id, kind, at, golden_pairs, k, recall, mrr, knobs)
         VALUES ('deadbeefdeadbeef', 'guesswork', '2026-08-31T10:00:00Z', 1, 10, 0.1, 0.1, '{}')",
        [],
    );
    assert!(
        err.is_err(),
        "the kind CHECK must reject a value outside eval|tune|bandit"
    );
}

#[test]
fn v14_upgrades_a_real_v13_database_without_losing_rows() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    build_v13_db(&db);

    // The real upgrade path: `connection::open` runs preflight + the
    // migration chain exactly as a user's next command would.
    let conn = connection::open(&db).expect("open migrates v13 -> v14");

    let memories: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    let edges: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(memories, 1, "the upgrade preserved the seeded memory row");
    assert_eq!(edges, 1, "the upgrade preserved the seeded edge row");

    let eval_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM eval_runs", [], |r| r.get(0))
        .unwrap();
    let gc_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM gc_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(eval_runs, 0, "the new table exists and starts empty");
    assert_eq!(gc_runs, 0);

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
fn v14_is_idempotent() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("comemory.db");
    let mut conn = connection::open(&db).expect("open runs the chain");
    conn.execute(
        "INSERT INTO gc_runs(id, at, removed, log_rows, event_rows, bytes_freed)
         VALUES ('1111222233334444', '2026-08-31T11:00:00Z', 1, 2, 3, 4)",
        [],
    )
    .unwrap();

    migrate::run(&mut conn).unwrap();

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM gc_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "a second run must not drop or duplicate rows");

    let markers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_meta WHERE key = '0014_v14_console'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(markers, 1, "the marker is written exactly once");
}
