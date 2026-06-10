//! Task 9: `comemory save` must write through the v0.2 store layer —
//! atomic markdown plus a SQLite mirror that includes FTS5 (always) and
//! `memory_vec` (only when a caller-supplied vector is provided).
//!
//! The dim guard fires before any DB write so a wrong-dim vector is a
//! hard failure on stderr instead of a silently dropped row. The fixup
//! commit also reorders the guard ahead of the markdown write, so a
//! wrong-dim payload must leave the `memories/` directory empty.

use assert_cmd::Command;
use comemory::store::connection;
use std::fs;
use tempfile::tempdir;

use super::vectors;

/// Count `.md` files at the top of `<data_dir>/memories/`, ignoring the
/// `.trash/` subdir and any hidden tmp files. Returns 0 when the directory
/// does not exist yet (the wrong-dim path is allowed to skip `ensure_dirs`
/// in the future without breaking this assertion).
fn count_md_files(data_dir: &std::path::Path) -> usize {
    let mem_dir = data_dir.join("memories");
    let Ok(read) = fs::read_dir(&mem_dir) else {
        return 0;
    };
    read.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".md") && !n.starts_with('.'))
                .unwrap_or(false)
        })
        .count()
}

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

/// Body A for the near-duplicate tests. Measured: simhash Hamming(A, B) = 5
/// (within NEAR_DUP_HAMMING = 8), Hamming(A, C) = 37, Hamming(B, C) = 36.
const DUP_BODY_A: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to fifty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";
/// Body B: A with one word changed (`fifty` → `eighty`).
const DUP_BODY_B: &str = "postgres connection pool exhausts under load spikes raise \
     max_connections to eighty and add pgbouncer in transaction mode for the api workers \
     during peak traffic hours";
/// Body C: a genuinely distinct topic.
const DUP_BODY_C: &str =
    "ast-grep pattern matching finds unwrap calls across the rust codebase quickly";

/// Run `comemory --json save <body> [extra...]` under `home` and parse the
/// JSON output. `extra` is appended after the body so tests can exercise
/// flags like `--supersedes`.
fn save_json_args(home: &tempfile::TempDir, body: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["--json", "save", "--kind", "note", body];
    args.extend_from_slice(extra);
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(&args)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    serde_json::from_str(stdout.trim()).expect("save --json emits one JSON object")
}

/// Run `comemory --json save <body>` under `home` and parse the JSON output.
fn save_json(home: &tempfile::TempDir, body: &str) -> serde_json::Value {
    save_json_args(home, body, &[])
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
fn near_duplicate_save_tty_emits_warning_line() {
    let home = tempdir().expect("tempdir");
    let first = save_json(&home, DUP_BODY_A);
    let first_id = first["id"].as_str().expect("id string").to_string();

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--kind", "note", DUP_BODY_B])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains(&format!("similar memory {first_id} exists")),
        "stderr should carry the duplicate warning, got: {stderr:?}",
    );
    assert!(
        stderr.contains("supersedes"),
        "warning should hint at supersedes: {stderr:?}",
    );
    // The saved-id line still lands on stdout, untouched by the warning.
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    assert!(stdout.starts_with("saved "), "stdout: {stdout:?}");
}

#[test]
fn distinct_save_tty_emits_no_warning() {
    let home = tempdir().expect("tempdir");
    save_json(&home, DUP_BODY_A);

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--kind", "note", DUP_BODY_C])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        !stderr.contains("similar memory"),
        "distinct topic must not warn: {stderr:?}",
    );
}

#[test]
fn identical_resave_reports_second_closest_near_dup() {
    // Fix 8 regression: the near-dup scan excludes the body's own
    // content-derived id BEFORE the closest-hit selection, so an identical
    // re-save surfaces the second-closest live near-dup instead of
    // self-matching (which the old post-save filter silently dropped,
    // yielding no duplicate_of at all).
    let home = tempdir().expect("tempdir");
    let first = save_json(&home, DUP_BODY_A);
    let first_id = first["id"].as_str().expect("id string").to_string();

    let second = save_json(&home, DUP_BODY_B);
    let second_id = second["id"].as_str().expect("id string").to_string();
    assert_eq!(second["duplicate_of"].as_str(), Some(first_id.as_str()));

    // Identical re-save of A: its own row is excluded, so B (the real
    // near-dup) must surface.
    let resave = save_json(&home, DUP_BODY_A);
    assert_eq!(resave["id"].as_str(), Some(first_id.as_str()));
    assert_eq!(
        resave["duplicate_of"].as_str(),
        Some(second_id.as_str()),
        "identical re-save must report the second-closest live near-dup: {resave}",
    );
}

#[test]
fn save_rejects_wrong_dim_vector() {
    let home = tempdir().expect("tempdir");
    let bad = vectors::vector("seed", 16);
    let payload = serde_json::to_string(&serde_json::json!({ "embedding": bad })).expect("json");

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", "--vector-stdin", "--kind", "note", "body"])
        .write_stdin(payload)
        .assert()
        .failure();
    let out = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(out.contains("vector dim mismatch"), "stderr: {out}");

    // Major-finding fix: the dim guard now runs BEFORE `MemoryStore::save`,
    // so a wrong-dim payload must leave the `memories/` directory empty —
    // no orphan markdown to garbage-collect.
    assert_eq!(
        count_md_files(home.path()),
        0,
        "dim guard must abort before the markdown write — orphan .md detected",
    );
}

// ─── --supersedes flag + edge materialization ────────────────────────────

/// Old memory body for the supersede tests. Shares query tokens with
/// `SUP_BODY_NEW` (advisory/locks/postgres/migrations) so a single search
/// returns both, while staying far apart in simhash space so the diversify
/// stage's near-dup collapse cannot fold one into the other.
const SUP_BODY_OLD: &str = "advisory locks serialize concurrent migrations in postgres";
/// Replacement memory body saved with `--supersedes <old id>`.
const SUP_BODY_NEW: &str = "advisory locks guidance update: prefer a migrations table with \
     select for update row locking instead of advisory locks in postgres";

/// Save the old + new supersede fixture bodies and return `(old_id, new_id,
/// new_md_path)`.
fn save_supersede_pair(home: &tempfile::TempDir) -> (String, String, String) {
    let old = save_json(home, SUP_BODY_OLD);
    let old_id = old["id"].as_str().expect("old id").to_string();
    let new = save_json_args(home, SUP_BODY_NEW, &["--supersedes", &old_id]);
    let new_id = new["id"].as_str().expect("new id").to_string();
    let new_path = new["path"].as_str().expect("new path").to_string();
    (old_id, new_id, new_path)
}

/// Count `supersedes` edges from `src` to `dst` in the live DB under `home`.
fn supersedes_edge_count(home: &tempfile::TempDir, src: &str, dst: &str) -> i64 {
    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    conn.query_row(
        "SELECT count(*) FROM edges WHERE src_kind = 'memory' AND src_id = ?1 \
           AND rel = 'supersedes' AND dst_kind = 'memory' AND dst_id = ?2",
        rusqlite::params![src, dst],
        |r| r.get(0),
    )
    .expect("count edges")
}

/// Run `comemory --json search <query>` under `home` and return the `hits`
/// array.
fn search_hits(home: &tempfile::TempDir, query: &str) -> Vec<serde_json::Value> {
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["--json", "search", query])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("search --json emits one JSON object");
    v["hits"].as_array().expect("hits array").clone()
}

/// Find the hit for `id` in `hits`, panicking with the full hit list when
/// absent so failures are debuggable.
fn hit_for<'a>(hits: &'a [serde_json::Value], id: &str) -> &'a serde_json::Value {
    hits.iter()
        .find(|h| h["memory_id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("no hit for {id} in {hits:?}"))
}

/// Assert the search-visible supersede contract for the fixture pair: the
/// old memory is annotated `superseded_by = new_id` with the 0.2 penalty,
/// while the new memory is unpenalized and unannotated. Shared between the
/// save-path test and the rebuild-parity test.
fn assert_supersede_search_contract(home: &tempfile::TempDir, old_id: &str, new_id: &str) {
    let hits = search_hits(home, "advisory locks postgres migrations");
    let old_hit = hit_for(&hits, old_id);
    assert_eq!(
        old_hit["superseded_by"].as_str(),
        Some(new_id),
        "old memory must carry superseded_by: {old_hit}",
    );
    assert_eq!(
        old_hit["score_parts"]["supersede"].as_f64(),
        Some(0.2),
        "old memory must take the supersede penalty: {old_hit}",
    );
    let new_hit = hit_for(&hits, new_id);
    assert!(
        new_hit.get("superseded_by").is_none(),
        "new memory must not be annotated: {new_hit}",
    );
    assert_eq!(
        new_hit["score_parts"]["supersede"].as_f64(),
        Some(1.0),
        "new memory must be unpenalized: {new_hit}",
    );
}

#[test]
fn save_supersedes_writes_edge_frontmatter_and_penalizes_ranking() {
    let home = tempdir().expect("tempdir");
    let (old_id, new_id, new_path) = save_supersede_pair(&home);

    // (a) The supersedes edge exists, directed new -> old.
    assert_eq!(
        supersedes_edge_count(&home, &new_id, &old_id),
        1,
        "expected exactly one supersedes edge {new_id} -> {old_id}",
    );

    // (b) Markdown stays the source of truth: the new memory's frontmatter
    // carries relations.supersedes = [old_id].
    let raw = fs::read_to_string(&new_path).expect("read new memory markdown");
    let (fm, _) = comemory::memory::Frontmatter::split(&raw).expect("parse frontmatter");
    assert_eq!(
        fm.relations.supersedes,
        vec![old_id.clone()],
        "frontmatter must record the supersedes relation",
    );

    // (c) Ranking: old memory penalized + annotated, new memory untouched.
    assert_supersede_search_contract(&home, &old_id, &new_id);
}

#[test]
fn rebuild_rematerializes_supersedes_edge_from_markdown() {
    let home = tempdir().expect("tempdir");
    let (old_id, new_id, _) = save_supersede_pair(&home);

    // Throw the derived DB away entirely — rebuild must reconstruct the
    // relation edge from frontmatter alone.
    fs::remove_file(home.path().join("comemory.db")).expect("drop db");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .arg("rebuild")
        .assert()
        .success();

    assert_eq!(
        supersedes_edge_count(&home, &new_id, &old_id),
        1,
        "rebuild must rematerialize the supersedes edge from frontmatter",
    );
    assert_supersede_search_contract(&home, &old_id, &new_id);
}

#[test]
fn save_rejects_self_supersede() {
    // Fix 2 regression: re-saving an identical body with --supersedes set
    // to its own content-hash id used to create a self-edge A→A that
    // permanently 0.2x-penalized the memory and flagged it for prune. The
    // save must now abort before any write.
    let home = tempdir().expect("tempdir");
    let first = save_json(&home, SUP_BODY_OLD);
    let own_id = first["id"].as_str().expect("id string").to_string();

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "note",
            "--supersedes",
            &own_id,
            SUP_BODY_OLD,
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("cannot supersede itself"),
        "stderr should explain the self-supersede rejection, got: {stderr}",
    );
    // No self-edge was written.
    assert_eq!(
        supersedes_edge_count(&home, &own_id, &own_id),
        0,
        "self-supersede must not write an edge",
    );
}

#[test]
fn save_rejects_malformed_supersedes_id() {
    let home = tempdir().expect("tempdir");
    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "--kind",
            "note",
            "--supersedes",
            "NOT-HEX!",
            "body that must never land on disk",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("--supersedes") && stderr.contains("NOT-HEX!"),
        "stderr should name the flag and the bad id, got: {stderr}",
    );
    // Validation runs before the markdown write — nothing saved.
    assert_eq!(
        count_md_files(home.path()),
        0,
        "invalid --supersedes must not leave a markdown file",
    );
}
