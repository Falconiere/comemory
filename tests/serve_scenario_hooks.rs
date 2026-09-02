#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Hooks journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_hooks.rs`: confirm-gated `POST /hooks/install` →
//! `GET /hooks` reports all three git hooks installed → `PUT
//! /hooks/{name}` flips one back off → `index-code` (job) + `search-code`
//! still work in that repo, against a real `comemory serve` and a real
//! git fixture.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/serve_bin.rs"]
mod serve_bin;

use std::collections::HashMap;

use serde_json::json;
use serve_bin::ServeHome;

/// Mirrors `tests/cli_scenario_hooks.rs::installed_by_name`: the per-hook
/// `installed` flag keyed by hook name, out of a `GET /hooks` (or `PUT
/// /hooks/{name}`) response's `hooks[]` rows.
fn installed_by_name(v: &serde_json::Value) -> HashMap<String, bool> {
    v["hooks"]
        .as_array()
        .expect("hooks array")
        .iter()
        .filter_map(|row| {
            Some((
                row["name"].as_str()?.to_string(),
                row["installed"].as_bool()?,
            ))
        })
        .collect()
}

#[test]
fn install_hooks_toggle_then_search_code_over_http() {
    let tmp = tempfile::TempDir::new().expect("workspace");
    let workspace = tmp.path().to_str().expect("utf8").to_string();
    let srv = ServeHome::with_args(&["--allow-path", &workspace]);

    let repo = tmp.path().join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("lib.rs", "fn hook_probe() {}\n")], "init");
    let repo_s = repo.to_str().expect("utf8").to_string();

    let (status, unconfirmed) = srv.post_raw("/hooks/install", &json!({ "repo": repo_s.clone() }));
    assert_eq!(status, 400, "{unconfirmed}");
    assert_eq!(
        unconfirmed["error"]["code"], "confirmation_required",
        "{unconfirmed}"
    );

    srv.post(
        "/hooks/install",
        &json!({ "repo": repo_s.clone(), "confirm": true }),
    );

    let after_install = srv.get_q("/hooks", &[("repo", repo_s.as_str())]);
    let map = installed_by_name(&after_install);
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(map[hook], "{hook} must be installed: {after_install}");
    }

    let after_disable = srv.put(
        &format!("/hooks/post-commit?repo={repo_s}"),
        &json!({ "enabled": false }),
    );
    let map = installed_by_name(&after_disable);
    assert!(!map["post-commit"], "post-commit flipped: {after_disable}");
    assert!(map["post-merge"]);
    assert!(map["post-checkout"]);

    let after_disable_reread = srv.get_q("/hooks", &[("repo", repo_s.as_str())]);
    let map = installed_by_name(&after_disable_reread);
    assert!(
        !map["post-commit"],
        "post-commit must read back flipped: {after_disable_reread}"
    );
    assert!(map["post-merge"]);
    assert!(map["post-checkout"]);

    let indexed = srv.job(
        "/code/index",
        &json!({ "repo": "repo", "path": repo_s.clone() }),
    );
    assert!(
        indexed["files_indexed"].as_u64().is_some_and(|n| n >= 1),
        "index-code job must report indexed files: {indexed}"
    );

    let search = srv.get_q("/code/search", &[("query", "hook_probe"), ("repo", "repo")]);
    assert!(
        !search["hits"].as_array().expect("hits").is_empty(),
        "search-code after hook toggle: {search}"
    );
}
