//! Task 14: `comemory rebuild` drops `comemory.db` and repopulates the
//! SQLite mirror from the on-disk markdown files. Markdown remains the
//! source of truth; the DB is a rebuildable derived cache.
//!
//! Part 1: basic reconstruction, tags/FTS/edges, staging-file skipping,
//! code-index preservation, and WAL/SHM sidecar cleanup.

#[path = "common/cli_rebuild_support.rs"]
mod support;
#[path = "common/vectors.rs"]
mod vectors;

use rusqlite::Connection;
use tempfile::{TempDir, tempdir};

use support::{count, open_db, open_db_with_vec, run_rebuild, run_save};

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

#[test]
fn rebuild_restores_tags_fts_and_edges() {
    let home = tempdir().expect("tempdir");
    save_rich_memory(&home);
    std::fs::remove_file(home.path().join("comemory.db")).expect("rm db");
    run_rebuild(&home);
    let conn = open_db_with_vec(&home);
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

/// Ingest one code symbol row (with a 768-d embedding) via `comemory
/// ingest-code` into `home`'s DB.
fn ingest_code_symbol(home: &TempDir) {
    let embedding = vectors::vector("rebuild-code-seed", 768);
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
    assert_cmd::Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .success();
}

/// Ingest a code symbol row via `comemory ingest-code` and verify it survives
/// a `comemory rebuild`. The code index tables must be preserved by copying
/// them from the old DB into the newly-built DB.
#[test]
fn rebuild_preserves_code_index() {
    let home = tempdir().expect("tempdir");

    // Save a memory so the DB exists before the ingest.
    run_save(&home, &["--kind", "note", "code index preservation body"]);
    ingest_code_symbol(&home);

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
    assert_cmd::Command::cargo_bin("comemory")
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

/// Snapshot of the persisted code references for a memory: the `code_ref`
/// anchor rows plus the count of each reference-edge kind. Captured before and
/// after a DB-delete + rebuild to prove anchors survive markdown round-trip.
#[derive(Debug, PartialEq, Eq)]
struct RefSnapshot {
    rows: Vec<comemory::store::code_ref::CodeRefRow>,
    file_edges: i64,
    symbol_edges: i64,
}

/// Capture the [`RefSnapshot`] for `memory_id` from `home`'s DB.
fn ref_snapshot(home: &TempDir, memory_id: &str) -> RefSnapshot {
    let conn = open_db(home);
    let rows = comemory::store::code_ref::for_memory(&conn, memory_id).expect("code_ref rows");
    let edge_count = |rel: &str| {
        conn.query_row(
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1 AND rel = ?2",
            rusqlite::params![memory_id, rel],
            |r| r.get::<_, i64>(0),
        )
        .expect("edge count")
    };
    RefSnapshot {
        rows,
        file_edges: edge_count("references_file"),
        symbol_edges: edge_count("references_symbol"),
    }
}

/// Save a memory anchored to one file + one symbol ref, returning its id.
/// Run from the crate-root cwd (a real git checkout) so the tracked refs
/// capture a real HEAD anchor (blob + commit + branch).
fn save_anchored_memory(home: &TempDir) -> String {
    run_save(
        home,
        &[
            "--repo",
            "comemory",
            "--kind",
            "note",
            "--ref-file",
            "src/cli/save.rs",
            "--ref-symbol",
            "src/cli/save.rs:run",
            "rebuild must replay anchored code refs from markdown",
        ],
    );
    let conn = open_db(home);
    conn.query_row("SELECT DISTINCT memory_id FROM code_ref", [], |r| r.get(0))
        .expect("a code_ref row exists after an anchored save")
}

/// Remove the DB and its WAL/SHM sidecars so the next open is a fresh rebuild.
fn delete_db(home: &TempDir) {
    std::fs::remove_file(home.path().join("comemory.db")).expect("remove db");
    let _ = std::fs::remove_file(home.path().join("comemory.db-wal"));
    let _ = std::fs::remove_file(home.path().join("comemory.db-shm"));
}

#[test]
fn rebuild_restores_code_ref_anchors_and_edges_from_frontmatter() {
    let home = tempdir().expect("tempdir");
    let memory_id = save_anchored_memory(&home);

    let before = ref_snapshot(&home, &memory_id);
    assert_eq!(
        before.rows.len(),
        2,
        "one file + one symbol anchor expected"
    );
    assert!(
        before.rows.iter().any(|r| r.pinned_blob.is_some()),
        "a tracked ref must capture a HEAD blob anchor: {before:?}"
    );
    assert_eq!(before.file_edges, 1);
    assert_eq!(before.symbol_edges, 1);

    delete_db(&home);
    run_rebuild(&home);

    let after = ref_snapshot(&home, &memory_id);
    assert_eq!(
        before, after,
        "rebuild must reconstruct identical code_ref anchors + reference edges from frontmatter"
    );
}
