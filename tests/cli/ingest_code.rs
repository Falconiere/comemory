//! Task 12: `comemory ingest-code` reads pre-embedded JSONL rows on stdin
//! and inserts them into `code_symbols` + `code_fts` + `code_vec`. The
//! caller-supplied embedding's dim must match `schema_meta.code_vector_dim`
//! (768 by default); a mismatch is the only documented failure mode.

use assert_cmd::Command;
use comemory::store::connection;
use tempfile::tempdir;

use super::git_setup;
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

/// Regression for the path-token lowercase divergence: the ingest path must
/// hand the RAW relative path to `code_fts.path_tokens` so the identifier
/// tokenizer can split camelCase segments — pre-lowercasing made
/// `MyComponent.tsx` unreachable from the query `component`.
#[test]
fn ingest_code_camel_case_path_is_searchable_by_subtoken() {
    let home = tempdir().expect("tempdir");
    let row = make_row(
        "camel",
        "webapp",
        "src/MyComponent.tsx",
        "cccc000000000000000000000000000000000000",
    );

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(format!("{row}\n"))
        .assert()
        .success();

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let hits =
        comemory::store::fts::search_code(&conn, "component", 10, None, None, (2.0, 1.0, 1.5))
            .expect("search code");
    assert_eq!(
        hits.len(),
        1,
        "camelCase path segment must be reachable via its subtoken"
    );
}

/// Build a cAST chunk-child JSONL row referencing `parent_symbol` with
/// `chunk_index` `idx` (one-based), matching the contract emitted by
/// `index-code --extract` for oversized symbols.
fn make_chunk_row(seed: &str, repo: &str, path: &str, oid: &str, parent: &str, idx: u32) -> String {
    let embedding = vectors::vector(seed, 768);
    serde_json::to_string(&serde_json::json!({
        "repo": repo,
        "path": path,
        "blob_oid": oid,
        "symbol": format!("{parent}#{idx}"),
        "kind": "function",
        "lang": "rust",
        "line_start": 1 + idx,
        "line_end": 2 + idx,
        "snippet": format!("chunk {idx} body"),
        "simhash": 0_i64,
        "embedding": embedding,
        "parent_symbol": parent,
        "chunk_index": idx,
    }))
    .expect("chunk row json")
}

/// Chunk rows resolve `parent_id` against the parent row inserted earlier
/// in the same stream — the wire contract `index-code --extract` emits.
#[test]
fn ingest_code_resolves_parent_id_for_chunk_rows() {
    let home = tempdir().expect("tempdir");
    let oid = "feed000000000000000000000000000000000000";
    let parent = make_row("parent", "myrepo", "src/big.rs", oid);
    // `make_row` names its symbol `sym_<seed>`.
    let chunk1 = make_chunk_row("c1", "myrepo", "src/big.rs", oid, "sym_parent", 1);
    let chunk2 = make_chunk_row("c2", "myrepo", "src/big.rs", oid, "sym_parent", 2);
    let payload = format!("{parent}\n{chunk1}\n{chunk2}\n");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .success();

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let parent_id: i64 = conn
        .query_row(
            "SELECT id FROM code_symbols WHERE symbol = 'sym_parent'",
            [],
            |r| r.get(0),
        )
        .expect("parent row");
    let children: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE parent_id = ?1",
            [parent_id],
            |r| r.get(0),
        )
        .expect("count children");
    assert_eq!(children, 2, "both chunk rows must point at the parent");
    let vecs: i64 = conn
        .query_row("SELECT count(*) FROM code_vec", [], |r| r.get(0))
        .expect("count code_vec");
    assert_eq!(vecs, 3, "parent + both chunks carry embeddings");
}

/// A chunk row whose parent never appeared earlier in the stream is a hard
/// error naming the row, and the whole batch rolls back.
#[test]
fn ingest_code_rejects_chunk_row_with_missing_parent() {
    let home = tempdir().expect("tempdir");
    let oid = "feed111111111111111111111111111111111111";
    let orphan = make_chunk_row("orphan", "myrepo", "src/big.rs", oid, "never_seen", 1);

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(format!("{orphan}\n"))
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("never_seen"),
        "error must name the missing parent; got: {stderr}"
    );

    let conn = connection::open(home.path().join("comemory.db")).expect("open db");
    let symbols: i64 = conn
        .query_row("SELECT count(*) FROM code_symbols", [], |r| r.get(0))
        .expect("count code_symbols");
    assert_eq!(symbols, 0, "orphan chunk row must roll back the batch");
}

/// `parent_symbol` without `chunk_index` (or vice versa) violates the wire
/// contract and must fail loudly.
#[test]
fn ingest_code_rejects_half_specified_chunk_fields() {
    let home = tempdir().expect("tempdir");
    let embedding = vectors::vector("half", 768);
    let row = serde_json::to_string(&serde_json::json!({
        "repo": "myrepo",
        "path": "src/big.rs",
        "blob_oid": "feed222222222222222222222222222222222222",
        "symbol": "lonely#1",
        "kind": "function",
        "lang": "rust",
        "line_start": 2_u32,
        "line_end": 3_u32,
        "snippet": "half-specified chunk",
        "simhash": 0_i64,
        "embedding": embedding,
        "parent_symbol": "lonely",
    }))
    .expect("row json");

    let assertion = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["ingest-code"])
        .write_stdin(format!("{row}\n"))
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("both parent_symbol and"),
        "error must explain the both-or-neither contract; got: {stderr}"
    );
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
    git_setup::init_repo(&repo);
    git_setup::commit_files(&repo, &[("src/lib.rs", "fn ingested_fn() {}\n")], "fixture");

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
