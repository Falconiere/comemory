#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Code journey over `/api/v1` — the HTTP twin of `tests/cli_scenario_code.rs`:
//! index-code (job) → search-code → feedback --used-code → graph → repos →
//! ast, against a real `comemory serve` and a real two-commit git fixture.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/serve_bin.rs"]
mod serve_bin;

use serde_json::json;
use serve_bin::ServeHome;

#[test]
fn index_search_feedback_graph_repos_ast_round_trip() {
    let tmp = tempfile::TempDir::new().expect("workspace");
    let workspace = tmp.path().to_str().expect("utf8").to_string();
    let srv = ServeHome::with_args(&["--allow-path", &workspace]);

    let repo = tmp.path().join("r");
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

    let indexed = srv.job("/code/index", &json!({ "repo": "r", "path": repo_s }));
    assert!(
        indexed["files_indexed"].as_u64().is_some_and(|n| n >= 1),
        "index-code job must report indexed files: {indexed}"
    );

    let search = srv.get_q(
        "/code/search",
        &[("query", "alpha"), ("repo", "r"), ("lang", "rust")],
    );
    let hits = search["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "search-code must hit alpha: {search}");
    let symbol_id = hits[0]["symbol_id"]
        .as_i64()
        .expect("symbol_id")
        .to_string();
    let query_id = search["query_id"].as_str().expect("query_id").to_string();

    srv.post(
        "/feedback",
        &json!({ "query_id": query_id, "used_code": [symbol_id] }),
    );

    let graph = srv.get_q("/graph", &[("repo", "r")]);
    let edges = graph["edges"].as_array().expect("edges");
    assert!(
        edges.iter().any(|e| e["rel"] == "imports"),
        "imports edge: {graph}"
    );

    let inventory = srv.get("/repos");
    assert!(
        inventory["repos"]
            .as_array()
            .expect("repos")
            .iter()
            .any(|r| r["repo"].as_str() == Some("r")),
        "{inventory}"
    );

    let a_rs = repo.join("a.rs").to_str().expect("utf8").to_string();
    let ast = srv.post(
        "/code/ast",
        &json!({
            "pattern": "fn $NAME($$$) { $$$ }",
            "lang": "rs",
            "file": a_rs,
        }),
    );
    let items = ast["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|i| i["text"].as_str().is_some_and(|t| t.contains("alpha"))),
        "ast must find alpha: {ast}"
    );
}
