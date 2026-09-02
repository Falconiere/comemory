#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Code-index journey: two-file import + co-change fixture → index-code →
//! search-code → feedback --used-code → graph → extract JSONL → ast.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use cli_bin::CliHome;
use serde_json::Value;

fn index_pair(home: &CliHome) -> std::path::PathBuf {
    let repo = home.data_dir().parent().expect("parent").join("r");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "couple once",
    );
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() { let _x = 1; }\n"),
            ("b.rs", "fn beta() { let _y = 2; }\n"),
        ],
        "couple twice",
    );
    let repo_s = repo.to_str().expect("utf8").to_string();
    home.run_ok(&["index-code", "--repo", "r", "--path", &repo_s]);
    repo
}

#[test]
fn index_search_feedback_graph_extract_ast() {
    let home = CliHome::new();
    let repo = index_pair(&home);
    let repo_s = repo.to_str().expect("utf8");

    let search = home.run_json(&["search-code", "alpha", "--repo", "r", "--lang", "rust"]);
    let hits = search["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "search-code must hit alpha: {search}");
    let symbol_id = hits[0]["symbol_id"]
        .as_i64()
        .expect("symbol_id")
        .to_string();
    let query_id = search["query_id"].as_str().expect("query_id");

    home.run_ok(&["feedback", query_id, "--used-code", &symbol_id]);

    let graph = home.run_json(&["graph", "--repo", "r"]);
    let edges = graph["edges"].as_array().expect("edges");
    assert!(
        edges.iter().any(|e| e["rel"] == "imports"),
        "imports edge: {graph}"
    );
    let dot = home.run_ok(&["graph", "--repo", "r", "--format", "dot"]);
    assert!(
        dot.contains("digraph") || dot.contains("->"),
        "dot graph: {dot}"
    );

    let inventory = home.run_json(&["repos"]);
    assert!(
        inventory["repos"]
            .as_array()
            .expect("repos")
            .iter()
            .any(|r| r["repo"].as_str() == Some("r")),
        "{inventory}"
    );

    let extract = home.run_ok(&["index-code", "--repo", "r", "--extract", "--path", repo_s]);
    let first = extract
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("extract JSONL");
    let row: Value = serde_json::from_str(first).expect("extract row");
    assert_eq!(row["repo"].as_str(), Some("r"));
    assert!(row["path"].as_str().is_some(), "{row}");
    assert!(row["symbol"].as_str().is_some(), "{row}");

    let a_rs = repo.join("a.rs");
    let ast = home
        .bin()
        .args([
            "--json",
            "ast",
            "fn $NAME($$$) { $$$ }",
            "--lang",
            "rs",
            "--file",
        ])
        .arg(&a_rs)
        .assert()
        .success();
    let ast_json: Value = serde_json::from_slice(&ast.get_output().stdout).expect("ast json");
    let items = ast_json["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|i| i["text"].as_str().is_some_and(|t| t.contains("alpha"))),
        "ast must find alpha: {ast_json}"
    );
}
