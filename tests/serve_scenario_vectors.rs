#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! BYO-vector journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_vectors.rs`: `index-code --extract` (CLI-only, no
//! HTTP equivalent) → splice embeddings → `POST /code/ingest` (job) →
//! `POST /code/search` vector search, plus `POST /memories` /
//! `POST /memories/search` vector save+search, against a real
//! `comemory serve`.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/serve_bin.rs"]
mod serve_bin;
#[path = "common/vectors.rs"]
mod vectors;

use assert_cmd::Command;
use serde_json::{Value, json};
use serve_bin::ServeHome;

#[test]
fn extract_embed_ingest_search_code_vector() {
    let tmp = tempfile::TempDir::new().expect("workspace");
    let workspace = tmp.path().to_str().expect("utf8").to_string();
    let srv = ServeHome::with_args(&["--allow-path", &workspace]);

    let repo = tmp.path().join("demo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "src/lib.rs",
            "pub fn parse_frontmatter(input: &str) -> Option<&str> {\n    input.strip_prefix(\"---\")\n}\n",
        )],
        "fixture",
    );
    let repo_s = repo.to_str().expect("utf8");

    // No HTTP `--extract` route (spec Non-Goal): the real CLI, pointed at
    // the server's own data dir, produces the JSONL. `--extract` writes no
    // DB rows, so running it alongside the live server touches no shared
    // SQLite state.
    let extract = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .env("COMEMORY_DATA_DIR", srv.data_dir())
        .args([
            "index-code",
            "--repo",
            "demo",
            "--extract",
            "--path",
            repo_s,
        ])
        .output()
        .expect("run index-code --extract");
    assert!(
        extract.status.success(),
        "index-code --extract failed: {}",
        String::from_utf8_lossy(&extract.stderr)
    );
    let extract_out = String::from_utf8(extract.stdout).expect("extract stdout utf8");

    let mut payload = String::new();
    let mut query_vec: Option<Vec<f32>> = None;
    for line in extract_out.lines().filter(|l| !l.trim().is_empty()) {
        let mut row: Value = serde_json::from_str(line).expect("extract row");
        let symbol = row["symbol"].as_str().expect("symbol").to_string();
        let embedding = vectors::vector(&symbol, 768);
        if query_vec.is_none() {
            query_vec = Some(embedding.clone());
        }
        row["embedding"] = json!(embedding);
        payload.push_str(&serde_json::to_string(&row).expect("row json"));
        payload.push('\n');
    }
    assert!(!payload.is_empty(), "extract must emit at least one row");

    let ingested = srv.job_text("/code/ingest", payload);
    assert_eq!(
        ingested["rows"].as_u64(),
        Some(1),
        "ingest job must report the one ingested row: {ingested}"
    );

    let search = srv.post(
        "/code/search",
        &json!({
            "query": "parse frontmatter",
            "vector": query_vec.expect("query vec"),
        }),
    );
    assert!(
        !search["hits"].as_array().expect("hits").is_empty(),
        "vector search-code must hit the ingested symbol: {search}"
    );

    let doctor = srv.get("/doctor");
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");
}

#[test]
fn save_and_search_memory_vector() {
    let srv = ServeHome::new();
    let embedding = vectors::vector("mem-knn", 1024);

    let saved = srv.post(
        "/memories",
        &json!({
            "body": "unique vector memory body about knn dim guard",
            "kind": "note",
            "vector": embedding,
        }),
    );
    let id = saved["id"].as_str().expect("save id").to_string();

    let search = srv.post(
        "/memories/search",
        &json!({
            "query": "knn dim guard",
            "vector": embedding,
        }),
    );
    let hits = search["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["memory_id"].as_str() == Some(id.as_str())),
        "vector search must return the saved memory: {search}"
    );

    let doctor = srv.get("/doctor");
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");
}
