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

/// Like [`open_db`] but routes through `connection::open` so the
/// `sqlite-vec` extension is loaded. Use this when the test needs to
/// SELECT against `code_vec` or `memory_vec`.
fn open_db_with_vec(home: &TempDir) -> Connection {
    comemory::store::connection::open(home.path().join("comemory.db")).expect("open with vec0")
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

/// Ingest a code symbol row via `comemory ingest-code` and verify it survives
/// a `comemory rebuild`. The code index tables must be preserved by copying
/// them from the old DB into the newly-built DB.
#[test]
fn rebuild_preserves_code_index() {
    let home = tempdir().expect("tempdir");

    // Save a memory so the DB exists before the ingest.
    run_save(&home, &["--kind", "note", "code index preservation body"]);

    // Ingest one code symbol via ingest-code.
    let embedding = super::vectors::vector("rebuild-code-seed", 768);
    let row = serde_json::json!({
        "repo": "myrepo",
        "path": "src/lib.rs",
        "blob_oid": "aaaa000000000000000000000000000000000000",
        "symbol": "preserved_fn",
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 2_u32,
        "snippet": "fn preserved_fn() {}",
        "simhash": 0_i64,
        "embedding": embedding,
    });
    let payload = format!("{row}\n");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .success();

    // Confirm baseline counts.
    {
        let conn = open_db(&home);
        assert_eq!(
            count(&conn, "SELECT count(*) FROM code_symbols"),
            1,
            "pre-rebuild code_symbols"
        );
    }

    // Rebuild: must preserve the code index.
    run_rebuild(&home);

    let conn = open_db_with_vec(&home);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM memories"),
        1,
        "rebuild must restore the memory row"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM code_symbols"),
        1,
        "rebuild must preserve code_symbols rows"
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM indexed_files"),
        1,
        "rebuild must preserve indexed_files rows"
    );
    // Regression for PR #3 xhigh review: vec0 cross-DB INSERT-SELECT must
    // actually carry the embedding rows over, not silently drop them.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM code_vec"),
        1,
        "rebuild must preserve code_vec rows"
    );
}

/// Regression for PR #3 review thread: after a successful rebuild, neither
/// the live DB path nor the tmp path should leave orphaned `*-wal` /
/// `*-shm` sidecars in the data dir. SQLite leaves these next to the main
/// file after a WAL-mode close, and the tmp connection's sidecars used to
/// linger forever because the cleanup loop only iterated against `db`.
#[test]
fn rebuild_cleans_up_wal_shm_sidecars() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "sidecar cleanup body"]);

    run_rebuild(&home);

    let data_dir = home.path();
    let live = data_dir.join("comemory.db");
    let tmp = data_dir.join("comemory.db.rebuild.tmp");
    for path in [&live, &tmp] {
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.clone().into_os_string();
            sidecar.push(suffix);
            let sidecar = std::path::PathBuf::from(sidecar);
            assert!(
                !sidecar.exists(),
                "rebuild must not leave a sidecar at {}",
                sidecar.display()
            );
        }
    }
    assert!(
        !tmp.exists(),
        "rebuild must not leave the tmp DB at {}",
        tmp.display()
    );
}

/// When the rebuild cannot complete (e.g. the tmp DB path is blocked by a
/// pre-existing directory), the original `comemory.db` must be left intact
/// and the tmp artefact must be cleaned up.
///
/// We trigger the failure by pre-creating a *directory* at the tmp path
/// before running rebuild. SQLite cannot open a directory as a database, so
/// `connection::open(tmp_path)` returns an error immediately — well before
/// the rename that would clobber the live DB.
#[test]
fn rebuild_does_not_destroy_on_error() {
    let home = tempdir().expect("tempdir");
    run_save(&home, &["--kind", "note", "original body"]);

    // Record the original DB's memory count to compare after the failed rebuild.
    let original_count = {
        let conn = open_db(&home);
        count(&conn, "SELECT count(*) FROM memories")
    };

    // Block the tmp path with a directory so `connection::open` cannot
    // create the tmp DB — rebuild must return a non-zero exit code without
    // touching the live `comemory.db`.
    let tmp_path = home.path().join("comemory.db.rebuild.tmp");
    std::fs::create_dir_all(&tmp_path).expect("create dir at tmp path");

    // Rebuild must fail.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["rebuild"])
        .assert()
        .failure();

    // The original DB must still be intact: it must open and have the same
    // memory count as before.
    let conn = open_db(&home);
    let after_count = count(&conn, "SELECT count(*) FROM memories");
    assert_eq!(
        after_count, original_count,
        "original DB must be intact after a failed rebuild"
    );
}
