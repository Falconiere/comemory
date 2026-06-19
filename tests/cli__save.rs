//! Task 9: `comemory save` must write through the v0.2 store layer —
//! atomic markdown plus a SQLite mirror that includes FTS5 (always) and
//! `memory_vec` (only when a caller-supplied vector is provided).
//!
//! Part 1: lexical-only mirror, the two BYO-vector branches (stdin JSON +
//! CSV flag), the CSV parse-error path, and the save-time near-dup advisory.

#[path = "common/cli_save_support.rs"]
mod support;
#[path = "common/vectors.rs"]
mod vectors;

use assert_cmd::Command;
use comemory::store::connection;
use support::{DUP_BODY_A, DUP_BODY_B, DUP_BODY_C, count_md_files, save_json};
use tempfile::tempdir;

/// Run a single `count(*)` query against `conn` and return the integer
/// result. Tests can chain several without a forest of `query_row` boilerplate.
fn count_query(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("query failed: {sql}: {e}"))
}

/// Assert the SQLite mirror tables for the
/// `save_writes_md_and_indexes_lexical_when_no_vector` test: one row in
/// `memories` (with the expected repo+author), exactly two tag rows,
/// `memory_fts` populated, `memory_vec` empty, and the 4 expected
/// memory→{repo,author,tag} edges.
fn assert_lexical_save_mirror(conn: &rusqlite::Connection) {
    assert_eq!(count_query(conn, "SELECT count(*) FROM memory_fts"), 1);
    assert_eq!(count_query(conn, "SELECT count(*) FROM memory_vec"), 0);
    assert_eq!(
        count_query(
            conn,
            "SELECT count(*) FROM memories WHERE repo = 'foo' AND author = 'alice'",
        ),
        1,
        "memories row missing or with wrong repo/author",
    );
    assert_eq!(count_query(conn, "SELECT count(*) FROM memory_tags"), 2);
    // 1 in_repo + 1 authored_by + 2 tagged = 4 edges.
    assert_eq!(
        count_query(
            conn,
            "SELECT count(*) FROM edges WHERE src_kind = 'memory' \
              AND rel IN ('in_repo','authored_by','tagged')",
        ),
        4,
        "expected 1 in_repo + 1 authored_by + 2 tagged",
    );
}

#[test]
fn save_writes_md_and_indexes_lexical_when_no_vector() {
    let home = tempdir().expect("tempdir");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "note",
            "--repo",
            "foo",
            "--tags",
            "db,postgres",
            "--author",
            "alice",
            "advisory locks for migration ordering",
        ])
        .assert()
        .success();

    assert_eq!(count_md_files(home.path()), 1);

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    assert_lexical_save_mirror(&conn);
}

#[test]
fn save_with_vector_stdin_writes_memory_vec_row() {
    let home = tempdir().expect("tempdir");
    let vector = vectors::vector("seed", 1024);
    let payload = serde_json::to_string(&serde_json::json!({ "embedding": vector })).expect("json");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--vector-stdin",
            "--kind",
            "note",
            "advisory locks for migration ordering",
        ])
        .write_stdin(payload)
        .assert()
        .success();

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    let vec_count: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count vec");
    assert_eq!(vec_count, 1);
}

#[test]
fn save_with_vector_csv_flag_writes_memory_vec_row() {
    // Exercise the CSV (`--vector`) branch of `read_optional_vector` end-
    // to-end: comma-split, f32::parse, then dim-guard. A 1024-d vector
    // rendered as CSV is rejected by some shells for being too long, but
    // `assert_cmd` passes the string straight through to `argv` so this
    // works fine in-process.
    let home = tempdir().expect("tempdir");
    let vector = vectors::vector("csv-seed", 1024);
    let csv = vector
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(",");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--vector",
            &csv,
            "--kind",
            "note",
            "advisory locks for migration ordering via csv",
        ])
        .assert()
        .success();

    let db_path = home.path().join("comemory.db");
    let conn = connection::open(&db_path).expect("open db");
    let vec_count: i64 = conn
        .query_row("SELECT count(*) FROM memory_vec", [], |r| r.get(0))
        .expect("count vec");
    assert_eq!(vec_count, 1, "CSV --vector flag should populate memory_vec");
}

#[test]
fn save_csv_vector_with_bad_token_fails_to_parse() {
    // The CSV branch maps `f32::parse` errors into `Error::Config("--vector
    // parse: ..")`. Feed a token that isn't a float to exercise that path.
    let home = tempdir().expect("tempdir");
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--vector",
            "0.1,not-a-float,0.3",
            "--kind",
            "note",
            "body",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("--vector parse"),
        "stderr should mention CSV parse error, got: {stderr}",
    );

    // Parse failure happens before the markdown write, so nothing on disk.
    assert_eq!(
        count_md_files(home.path()),
        0,
        "csv parse error must not leave an orphan markdown",
    );
}

#[test]
fn near_duplicate_save_warns_and_hints() {
    let home = tempdir().expect("tempdir");

    let first = save_json(&home, DUP_BODY_A);
    let first_id = first["id"].as_str().expect("id string").to_string();
    assert!(
        first.get("duplicate_of").is_none(),
        "first save has nothing to duplicate: {first}",
    );

    // One-word edit of A: Hamming 5 <= NEAR_DUP_HAMMING, so the save still
    // succeeds but reports the original id as `duplicate_of`.
    let second = save_json(&home, DUP_BODY_B);
    assert_eq!(
        second["duplicate_of"].as_str(),
        Some(first_id.as_str()),
        "near-dup save should point at the first id: {second}",
    );
    assert_ne!(second["id"].as_str(), Some(first_id.as_str()));

    // Distinct topic: the key must be ABSENT (skip_serializing_if), not null.
    let third = save_json(&home, DUP_BODY_C);
    assert!(
        third.get("duplicate_of").is_none(),
        "distinct save must omit duplicate_of entirely: {third}",
    );
}

#[test]
fn near_dup_radius_env_is_honored() {
    // Hamming(A, B) = 5: with COMEMORY_RANK_NEAR_DUP_HAMMING tightened to 4
    // the second save must NOT report a duplicate — the save-time check
    // reads cfg.rank.near_dup_hamming instead of the hardcoded constant.
    let home = tempdir().expect("tempdir");
    save_json(&home, DUP_BODY_A);

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .env("COMEMORY_RANK_NEAR_DUP_HAMMING", "4")
        .args(["--json", "save", "--kind", "note", DUP_BODY_B])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let second: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("save --json emits one JSON object");
    assert!(
        second.get("duplicate_of").is_none(),
        "radius 4 must not flag a Hamming-5 near-dup: {second}",
    );
}

/// Read the single saved `.md` body from a temp data dir's `memories/`.
fn read_only_memory(home: &std::path::Path) -> String {
    let mem_dir = home.join("memories");
    let entry = std::fs::read_dir(&mem_dir)
        .expect("memories dir")
        .flatten()
        .find(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".md") && !n.starts_with('.'))
                .unwrap_or(false)
        })
        .expect("one saved markdown");
    std::fs::read_to_string(entry.path()).expect("read md")
}

#[test]
fn save_ref_symbol_writes_frontmatter_anchor_edge_and_code_ref_row() {
    // Runs inside the real comemory git checkout (test cwd = crate root), so
    // `--repo comemory --ref-symbol src/cli/save.rs:run` pins a committed
    // anchor against the on-disk HEAD tree.
    let home = tempdir().expect("tempdir");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "note",
            "--repo",
            "comemory",
            "--ref-symbol",
            "src/cli/save.rs:run",
            "links the save run fn",
        ])
        .assert()
        .success();

    let md = read_only_memory(home.path());
    assert!(
        md.contains("comemory:src/cli/save.rs:run"),
        "frontmatter must carry the qualified symbol ref:\n{md}",
    );

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    assert_eq!(
        count_query(
            &conn,
            "SELECT count(*) FROM code_ref WHERE rel = 'references_symbol' \
              AND dst_id = 'comemory:src/cli/save.rs:run' AND pinned_blob IS NOT NULL",
        ),
        1,
        "a pinned code_ref row must exist for the symbol",
    );
    assert_eq!(
        count_query(
            &conn,
            "SELECT count(*) FROM edges WHERE rel = 'references_symbol' \
              AND dst_id = 'comemory:src/cli/save.rs:run'",
        ),
        1,
        "a references_symbol edge must exist for the symbol",
    );
}

#[test]
fn save_malformed_ref_symbol_exits_64() {
    // `foo.rs` has no `:symbol` segment → EX_USAGE (exit 64), pre-write.
    let home = tempdir().expect("tempdir");
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "note",
            "--repo",
            "comemory",
            "--ref-symbol",
            "foo.rs",
            "body",
        ])
        .assert()
        .code(64);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("foo.rs"),
        "exit-64 message must name the bad value, got: {stderr}",
    );
    assert!(
        !stderr.contains("memory not found"),
        "a malformed flag must surface as a usage error, not 'memory not found', got: {stderr}",
    );
    assert_eq!(
        count_md_files(home.path()),
        0,
        "a malformed ref must abort before the markdown write",
    );
}

#[test]
fn save_untracked_ref_exits_0_with_json_warning() {
    let home = tempdir().expect("tempdir");
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "--json",
            "save",
            "--kind",
            "note",
            "--repo",
            "comemory",
            "--ref-file",
            "src/does/not/exist/zzz.nope",
            "links a missing file",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let out: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("save --json emits one JSON object");
    let warnings = out["warnings"].as_array().expect("warnings array present");
    assert_eq!(warnings.len(), 1, "one unpinned-ref warning: {out}");
    assert!(
        warnings[0]
            .as_str()
            .map(|w| w.contains("zzz.nope") || w.contains("untracked"))
            .unwrap_or(false),
        "warning should describe the unpinned ref: {out}",
    );
}
