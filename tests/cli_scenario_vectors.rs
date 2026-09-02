#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! BYO-vector journey: `index-code --extract` → splice embeddings →
//! `ingest-code` → `search-code --vector-stdin`, plus memory
//! save/search `--vector-stdin`.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/vectors.rs"]
mod vectors;

use cli_bin::CliHome;
use serde_json::{Value, json};

#[test]
fn extract_embed_ingest_search_code_vector() {
    let home = CliHome::new();
    let repo = home.data_dir().parent().expect("parent").join("demo");
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
    let extract = home.run_ok(&[
        "index-code",
        "--repo",
        "demo",
        "--extract",
        "--path",
        repo_s,
    ]);

    let mut payload = String::new();
    let mut query_vec: Option<Vec<f32>> = None;
    for line in extract.lines().filter(|l| !l.trim().is_empty()) {
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

    home.bin()
        .args(["ingest-code"])
        .write_stdin(payload)
        .assert()
        .success();

    let q = json!({ "embedding": query_vec.expect("query vec") });
    let search = home
        .bin()
        .args([
            "--json",
            "search-code",
            "parse frontmatter",
            "--vector-stdin",
        ])
        .write_stdin(serde_json::to_string(&q).expect("payload"))
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&search.get_output().stdout).expect("search json");
    assert!(
        !v["hits"].as_array().expect("hits").is_empty(),
        "vector search-code must hit the ingested symbol: {v}"
    );
    let doctor = home.run_json(&["doctor"]);
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");
}

#[test]
fn save_and_search_memory_vector_stdin() {
    let home = CliHome::new();
    let embedding = vectors::vector("mem-knn", 1024);
    let payload = serde_json::to_string(&json!({ "embedding": embedding })).expect("payload");

    home.bin()
        .args([
            "--json",
            "save",
            "unique vector memory body about knn dim guard",
            "--kind",
            "note",
            "--vector-stdin",
        ])
        .write_stdin(payload.as_str())
        .assert()
        .success();

    let search = home
        .bin()
        .args(["--json", "search", "knn dim guard", "--vector-stdin"])
        .write_stdin(payload)
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&search.get_output().stdout).expect("search json");
    assert!(
        !v["hits"].as_array().expect("hits").is_empty(),
        "vector search must return the saved memory: {v}"
    );
    let doctor = home.run_json(&["doctor"]);
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");
}
