//! Task 14: `comemory rebuild` drops `comemory.db` and repopulates the
//! SQLite mirror from the on-disk markdown files. Markdown remains the
//! source of truth; the DB is a rebuildable derived cache.
//!
//! These tests exercise more than the row count: tags reappear in
//! `memory_tags`, the FTS index is repopulated, the v0.2 edges
//! (`in_repo` / `authored_by` / `tagged` plus cross-link references
//! parsed from a backticked `repo:path` reference) come back, and hidden
//! staging files in `memories/` are ignored.

use assert_cmd::Command;
use rusqlite::Connection;
use tempfile::{tempdir, TempDir};

fn run_save(home: &TempDir, args: &[&str]) {
    let mut cmd = Command::cargo_bin("comemory").expect("bin");
    cmd.env("COMEMORY_DATA_DIR", home.path());
    cmd.arg("save");
    for a in args {
        cmd.arg(a);
    }
    cmd.assert().success();
}

fn run_rebuild(home: &TempDir) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .success();
}

fn open_db(home: &TempDir) -> Connection {
    Connection::open(home.path().join("comemory.db")).expect("open")
}

#[test]
fn rebuild_reconstructs_memories_from_markdown() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "body one"]);
    run_save(&home, &["--kind", "note", "body two"]);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);

    let conn = open_db(&home);
    let cnt: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .expect("count");
    assert_eq!(cnt, 2);
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn assert_edge(conn: &Connection, rel: &str, dst_kind: &str, dst_id: &str) {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND rel = ?1 \
               AND dst_kind = ?2 AND dst_id = ?3",
            rusqlite::params![rel, dst_kind, dst_id],
            |r| r.get(0),
        )
        .expect("count edges");
    assert_eq!(n, 1, "expected edge {rel} -> {dst_kind}:{dst_id}");
}

fn save_rich_memory(home: &TempDir) {
    run_save(
        home,
        &[
            "--kind",
            "decision",
            "--repo",
            "qwick",
            "--author",
            "alice",
            "--tags",
            "db,postgres",
            "see `qwick:src/lib.rs` for the rationale",
        ],
    );
}

fn assert_rebuilt_state(conn: &Connection) {
    assert_eq!(count(conn, "SELECT count(*) FROM memories"), 1);
    assert_eq!(count(conn, "SELECT count(*) FROM memory_tags"), 2);
    let fts_hits: i64 = count(
        conn,
        "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'rationale'",
    );
    assert!(fts_hits >= 1, "expected FTS to find 'rationale'");
    assert_edge(conn, "in_repo", "repo", "qwick");
    assert_edge(conn, "authored_by", "author", "alice");
    assert_edge(conn, "tagged", "tag", "db");
    assert_edge(conn, "tagged", "tag", "postgres");
    assert_edge(conn, "references_file", "file", "qwick:src/lib.rs");
}

#[test]
fn rebuild_restores_tags_fts_and_edges() {
    let home = tempdir().expect("tempdir");
    save_rich_memory(&home);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);
    let conn = open_db(&home);
    assert_rebuilt_state(&conn);
}

#[test]
fn rebuild_skips_hidden_staging_files() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "kept body"]);
    // Drop a stale staging file in `memories/` — rebuild must skip it
    // rather than try to parse it as YAML frontmatter.
    let stale = home.path().join("memories").join(".abc12345.tmp");
    std::fs::write(&stale, "garbage not yaml").expect("write stale");
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);

    let conn = open_db(&home);
    let cnt: i64 = count(&conn, "SELECT count(*) FROM memories");
    assert_eq!(cnt, 1);
}
