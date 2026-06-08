//! Task 12: `comemory ingest-code` reads pre-embedded JSONL rows on stdin
//! and inserts them into `code_symbols` + `code_fts` + `code_vec`. The
//! caller-supplied embedding's dim must match `schema_meta.code_vector_dim`
//! (768 by default); a mismatch is the only documented failure mode.

use assert_cmd::Command;
use comemory::store::connection;
use tempfile::tempdir;

use super::vectors;

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
