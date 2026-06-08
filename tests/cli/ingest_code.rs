//! Task 12: `comemory ingest-code` reads pre-embedded JSONL rows on stdin
//! and inserts them into `code_symbols` + `code_fts` + `code_vec`. The
//! caller-supplied embedding's dim must match `schema_meta.code_vector_dim`
//! (768 by default); a mismatch is the only documented failure mode.

use assert_cmd::Command;
use comemory::store::connection;
use tempfile::tempdir;

use super::vectors;

/// Build a minimal valid JSONL row string. `seed` drives the embedding so
/// every call produces a distinct (but deterministic) vector.
fn make_row(seed: &str, repo: &str, path: &str, blob_oid: &str) -> String {
    let embedding = vectors::vector(seed, 768);
    serde_json::to_string(&serde_json::json!({
        "repo": repo,
        "path": path,
        "blob_oid": blob_oid,
        "symbol": format!("sym_{seed}"),
        "kind": "function",
        "lang": "rust",
        "line_start": 1_u32,
        "line_end": 3_u32,
        "snippet": format!("fn {seed}() {{}}"),
        "simhash": 0_i64,
        "embedding": embedding,
    }))
    .expect("row json")
}

#[test]
fn ingest_code_inserts_row_with_supplied_embedding() {
    let home = tempdir().expect("tempdir");
    let embedding = vectors::vector("ingest-code-seed", 768);
    let row = serde_json::json!({
        "repo": "sample",
        "path": "src/lib.rs",
        "blob_oid": "0000000000000000000000000000000000000000",
        "symbol": "run_migration",
        "kind": "function",
        "lang": "rust",
        "line_start": 1,
        "line_end": 3,
        "snippet": "fn run_migration() {}\n",
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

    let db = home.path().join("comemory.db");
    let conn = connection::open(&db).expect("open db");
    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(symbols, 1);
    let vecs: i64 = conn
        .query_row("SELECT count(*) FROM code_vec", [], |r| r.get(0))
        .expect("count code_vec");
    assert_eq!(vecs, 1);
}

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
