//! Task 12: `comemory ingest-code` — part 2.
//!
//! Covers: rollback on malformed mid-stream rows, conflicting blob_oid
//! rejection, wrong-dim vector rejection, and the extract→ingest→index
//! blob-gate round trip that must preserve BYO embeddings.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/cli_ingest_code_support.rs"]
mod support;
#[path = "common/vectors.rs"]
mod vectors;

use assert_cmd::Command;
use comemory::store::connection;
use support::make_row;
use tempfile::tempdir;

/// A malformed row mid-stream must roll back the entire batch; no rows should
/// land in `code_symbols` after a partial stream that fails on the second row.
#[test]
fn ingest_code_rolls_back_on_malformed_mid_stream() {
    let home = tempdir().expect("tempdir");
    let oid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let good_row = make_row("row1", "sample", "src/a.rs", oid);
    let bad_row = r#"{"not_valid_json_at_all": true, "missing_fields": "yes"}"#;
    let payload = format!("{good_row}\n{bad_row}\n");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .failure(); // malformed row must cause a non-zero exit

    let db = home.path().join("comemory.db");
    let conn = connection::open(&db).expect("open db");
    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(
        symbols, 0,
        "transaction must roll back on malformed mid-stream row; got {symbols} rows"
    );
    let files: i64 = conn
        .query_row("SELECT count(*) FROM indexed_files", [], |r| r.get(0))
        .expect("count indexed_files");
    assert_eq!(
        files, 0,
        "indexed_files must also be empty after rollback; got {files} rows"
    );
}

/// Two rows with the same (repo, path) but different blob_oid should be rejected
/// with a non-zero exit and must not commit any rows to `code_symbols`.
#[test]
fn ingest_code_rejects_conflicting_blob_oid_for_same_path() {
    let home = tempdir().expect("tempdir");
    let row1 = make_row(
        "conflict1",
        "myrepo",
        "src/lib.rs",
        "aaa0000000000000000000000000000000000000",
    );
    let row2 = make_row(
        "conflict2",
        "myrepo",
        "src/lib.rs",
        "bbb0000000000000000000000000000000000000",
    );
    let payload = format!("{row1}\n{row2}\n");

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("conflicting blob_oid"),
        "stderr should mention conflicting blob_oid; got: {stderr}"
    );

    let db = home.path().join("comemory.db");
    let conn = connection::open(&db).expect("open db");
    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(
        symbols, 0,
        "conflicting blob_oid must roll back all rows; got {symbols}"
    );
}

/// A row whose embedding length does not match the schema's code_vector_dim
/// (768) must fail with a VecDimMismatch error and no rows inserted.
#[test]
fn ingest_code_rejects_wrong_dim_vector() {
    let home = tempdir().expect("tempdir");
    let wrong_dim_embedding = vectors::vector("wrong-dim", 32); // should be 768
    let row = serde_json::json!({
        "repo": "sample",
        "path": "src/lib.rs",
        "blob_oid": "0000000000000000000000000000000000000000",
        "symbol": "bad_fn",
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 2_u32,
        "snippet": "fn bad_fn() {}",
        "simhash": 0_i64,
        "embedding": wrong_dim_embedding,
    });
    let payload = format!("{row}\n");

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("vector dim mismatch"),
        "stderr should mention vector dim mismatch; got: {stderr}"
    );

    let db = home.path().join("comemory.db");
    let conn = connection::open(&db).expect("open db");
    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(
        symbols, 0,
        "wrong-dim vector must cause rollback; got {symbols} rows"
    );
}

/// Regression for the M3 final-integration review: `ingest-code` must
/// stamp the per-repo code format (`schema_meta` key `code_format:<repo>`)
/// exactly like `index-code` does. Without the stamp, the next
/// `index-code` run on the same tree saw an unstamped repo, dropped every
/// `indexed_files` cursor, and the full re-walk purged ALL the BYO
/// `code_vec` embeddings the ingest had just landed.
#[test]
fn ingest_then_index_code_honors_blob_gate_and_keeps_embeddings() {
    let home = tempdir().expect("tempdir");
    let repo = home.path().join("fixture-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("src/lib.rs", "fn ingested_fn() {}\n")], "fixture");

    // extract → (caller embeds) → ingest: the documented BYO pipeline.
    let extract = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "fixture", "--path"])
        .arg(&repo)
        .arg("--extract")
        .assert()
        .success();
    let jsonl = String::from_utf8(extract.get_output().stdout.clone()).expect("utf8 jsonl");
    assert!(
        !jsonl.trim().is_empty(),
        "extract must emit at least one row"
    );
    let embedded: String = jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut row: serde_json::Value = serde_json::from_str(l).expect("row json");
            row["embedding"] = serde_json::json!(vectors::vector(l, 768));
            format!("{row}\n")
        })
        .collect();
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(embedded)
        .assert()
        .success();

    let snapshot = |conn: &rusqlite::Connection| -> (Vec<(String, String)>, i64) {
        let mut stmt = conn
            .prepare("SELECT path, indexed_at FROM indexed_files ORDER BY path")
            .expect("prepare");
        let cursors = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query")
            .collect::<Result<Vec<(String, String)>, _>>()
            .expect("rows");
        let vecs: i64 = conn
            .query_row("SELECT count(*) FROM code_vec", [], |r| r.get(0))
            .expect("count code_vec");
        (cursors, vecs)
    };
    let before = {
        let conn = connection::open(home.path().join("comemory.db")).expect("open db");
        snapshot(&conn)
    };
    assert!(before.1 > 0, "ingest must have landed code_vec rows");

    // A follow-up index-code over the unchanged tree must be gated by the
    // per-file blob OIDs (untouched indexed_at) and must NOT purge the
    // ingested embeddings.
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "fixture", "--path"])
        .arg(&repo)
        .assert()
        .success();

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let after = snapshot(&conn);
    assert_eq!(
        after.0, before.0,
        "indexed_at cursors must be untouched (blob gate honored)"
    );
    assert_eq!(
        after.1, before.1,
        "code_vec embeddings must survive the follow-up index-code"
    );
}
