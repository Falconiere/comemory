#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Getting-started journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_getting_started.rs`: doctor → save → search →
//! index-code (job) → search-code → context → edges → stats → repos →
//! show, against a real `comemory serve` and a real git fixture.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/serve_bin.rs"]
mod serve_bin;

use serde_json::json;
use serve_bin::ServeHome;

/// Mentions a qualified symbol so the save writes a `references_symbol`
/// edge that `GET /edges` can find by words.
const SAVE_BODY: &str = "Use Postgres for analytics, not ClickHouse — see ADR-14. \
The frontmatter parser is `demo:src/lib.rs:parse_frontmatter`.";

#[test]
fn getting_started_save_search_index_context_round_trip() {
    let tmp = tempfile::TempDir::new().expect("workspace");
    let workspace = tmp.path().to_str().expect("utf8").to_string();
    let srv = ServeHome::with_args(&["--allow-path", &workspace]);

    let doctor = srv.get("/doctor");
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");

    let saved = srv.post(
        "/memories",
        &json!({
            "body": SAVE_BODY,
            "kind": "decision",
            "repo": "demo",
            "tags": ["db", "analytics"],
        }),
    );
    let id = saved["id"].as_str().expect("save id").to_string();

    let search = srv.get_q("/memories/search", &[("query", "postgres analytics")]);
    let hits = search["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["memory_id"].as_str() == Some(id.as_str())),
        "search must return the saved memory: {search}"
    );

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
    let indexed = srv.job(
        "/code/index",
        &json!({ "repo": "demo", "path": repo.to_str().expect("utf8") }),
    );
    assert!(
        indexed["files_indexed"].as_u64().is_some_and(|n| n >= 1),
        "index-code job must report indexed files: {indexed}"
    );

    let code = srv.get_q(
        "/code/search",
        &[("query", "parse frontmatter"), ("repo", "demo")],
    );
    assert!(
        !code["hits"].as_array().expect("code hits").is_empty(),
        "search-code must hit parse_frontmatter: {code}"
    );

    let ctx = srv.get_q("/context", &[("query", "frontmatter"), ("repo", "demo")]);
    assert!(
        ctx.get("memories").and_then(|m| m.as_array()).is_some(),
        "context bundle must carry memories: {ctx}"
    );

    let edges = srv.get_q("/edges", &[("query", "parse_frontmatter")]);
    assert!(
        edges["items"]
            .as_array()
            .expect("edges items")
            .iter()
            .any(|e| e["rel"] == "references_symbol" && e["src_id"].as_str() == Some(id.as_str())),
        "edges must find the memory's symbol reference by words: {edges}"
    );

    let stats = srv.get("/stats");
    assert!(
        stats["memories"].as_u64().expect("memories") >= 1,
        "{stats}"
    );
    assert!(
        stats["code_symbols"].as_u64().expect("code_symbols") >= 1,
        "{stats}"
    );

    let inventory = srv.get("/repos");
    assert!(
        inventory["repos"]
            .as_array()
            .expect("repos array")
            .iter()
            .any(|r| r["repo"].as_str() == Some("demo")),
        "repos must list demo: {inventory}"
    );

    let shown = srv.get(&format!("/memories/{id}"));
    assert_eq!(shown["body"].as_str(), Some(SAVE_BODY), "{shown}");
}
