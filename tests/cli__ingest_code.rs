//! Task 12: `comemory ingest-code` reads pre-embedded JSONL rows on stdin
//! and inserts them into `code_symbols` + `code_fts` + `code_vec`. The
//! caller-supplied embedding's dim must match `schema_meta.code_vector_dim`
//! (768 by default); a mismatch is the only documented failure mode.
//!
//! Part 1: happy-path insert, camelCase path tokenization, chunk parent_id
//! resolution, and the missing/half-specified chunk-field guards.

#[path = "common/cli_ingest_code_support.rs"]
mod support;
#[path = "common/vectors.rs"]
mod vectors;

use assert_cmd::Command;
use comemory::store::connection;
use support::make_row;
use tempfile::tempdir;

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
