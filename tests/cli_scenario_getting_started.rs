#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Getting-started journey: `docs/getting-started.md` steps 2–5 against
//! a throwaway store and a real git fixture. Compositional only — per-command
//! JSON contracts live in `tests/cli__*.rs`.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use cli_bin::CliHome;

/// Mentions a qualified symbol so `save` writes a `references_symbol`
/// edge that `comemory edges` can find by words.
const SAVE_BODY: &str = "Use Postgres for analytics, not ClickHouse — see ADR-14. \
The frontmatter parser is `demo:src/lib.rs:parse_frontmatter`.";

#[test]
fn getting_started_save_search_index_context_round_trip() {
    let home = CliHome::new();
    let doctor = home.run_json(&["doctor"]);
    assert_eq!(doctor["db_writable"].as_bool(), Some(true), "{doctor}");

    let saved = home.run_json(&[
        "save",
        SAVE_BODY,
        "--kind",
        "decision",
        "--repo",
        "demo",
        "--tags",
        "db,analytics",
    ]);
    let id = saved["id"].as_str().expect("save id").to_string();

    let search = home.run_json(&["search", "postgres analytics"]);
    let hits = search["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|h| h["memory_id"].as_str() == Some(id.as_str())),
        "search must return the saved memory: {search}"
    );

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
    home.run_ok(&["index-code", "--repo", "demo", "--path", repo_s]);

    let code = home.run_json(&["search-code", "parse frontmatter", "--repo", "demo"]);
    let code_hits = code["hits"].as_array().expect("code hits");
    assert!(
        !code_hits.is_empty(),
        "search-code must hit parse_frontmatter: {code}"
    );

    let ctx = home.run_json(&["context", "frontmatter", "--repo", "demo"]);
    assert!(
        ctx.get("memories").and_then(|m| m.as_array()).is_some(),
        "context bundle must carry memories: {ctx}"
    );

    let edges = home.run_json(&["edges", "parse_frontmatter"]);
    assert!(
        edges["items"]
            .as_array()
            .expect("edges items")
            .iter()
            .any(|e| e["rel"] == "references_symbol" && e["src_id"].as_str() == Some(id.as_str())),
        "edges must find the memory's symbol reference by words: {edges}"
    );

    let stats = home.run_json(&["stats"]);
    assert!(
        stats["memories"].as_u64().expect("memories") >= 1,
        "{stats}"
    );
    assert!(
        stats["code_symbols"].as_u64().expect("code_symbols") >= 1,
        "{stats}"
    );

    let inventory = home.run_json(&["repos"]);
    let listed = inventory["repos"].as_array().expect("repos array");
    assert!(
        listed.iter().any(|r| r["repo"].as_str() == Some("demo")),
        "repos must list demo: {inventory}"
    );

    let shown = home.run_json(&["show", &id]);
    assert_eq!(shown["body"].as_str(), Some(SAVE_BODY), "{shown}");
}
