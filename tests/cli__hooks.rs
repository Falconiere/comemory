#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory hooks` driven as a real subprocess against a real git repo
//! (spec AC-35, AC-36) — no mocks (Binding Rule 9).

#[path = "common/git_repo.rs"]
mod git_repo;

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

/// Run `comemory` with the temp data dir, returning stdout on success.
fn comemory(data_dir: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args(args)
        .env("COMEMORY_DATA_DIR", data_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "comemory {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn hooks_json(data_dir: &Path, repo: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "hooks",
        "--repo",
        repo.to_str().expect("utf8 path"),
        "--json",
    ];
    args.extend_from_slice(extra);
    let out = comemory(data_dir, &args);
    serde_json::from_str(&out).expect("hooks --json parses")
}

/// `{name: installed}` map from a `hooks --json` payload, for easy per-row
/// assertions.
fn installed_by_name(v: &serde_json::Value) -> HashMap<String, bool> {
    v["hooks"]
        .as_array()
        .expect("hooks array")
        .iter()
        .map(|h| {
            (
                h["name"].as_str().expect("name").to_string(),
                h["installed"].as_bool().expect("installed"),
            )
        })
        .collect()
}

#[test]
fn ac35_fresh_repo_then_install_hooks_then_disable_one() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = ws.path().join("repo");
    git_repo::init_repo(&repo);

    let before = hooks_json(home.path(), &repo, &[]);
    let before_map = installed_by_name(&before);
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(!before_map[hook], "{hook} must start uninstalled");
    }

    comemory(
        home.path(),
        &["install-hooks", "--repo", repo.to_str().expect("utf8 path")],
    );
    let after_install = hooks_json(home.path(), &repo, &[]);
    let after_install_map = installed_by_name(&after_install);
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(after_install_map[hook], "{hook} must be installed");
    }

    let hooks_dir = repo.join(".git").join("hooks");
    let commit_before = std::fs::read(hooks_dir.join("post-commit")).expect("read post-commit");
    let merge_before = std::fs::read(hooks_dir.join("post-merge")).expect("read post-merge");

    let after_disable = hooks_json(home.path(), &repo, &["--disable", "post-checkout"]);
    let after_disable_map = installed_by_name(&after_disable);
    assert!(!after_disable_map["post-checkout"], "only this hook flips");
    assert!(after_disable_map["post-commit"]);
    assert!(after_disable_map["post-merge"]);

    assert_eq!(
        std::fs::read(hooks_dir.join("post-commit")).expect("read post-commit"),
        commit_before,
        "post-commit file must be byte-identical after disabling post-checkout"
    );
    assert_eq!(
        std::fs::read(hooks_dir.join("post-merge")).expect("read post-merge"),
        merge_before,
        "post-merge file must be byte-identical after disabling post-checkout"
    );
}

#[test]
fn ac36_search_edit_reinforcement_row_is_config_backed_and_round_trips() {
    let home = TempDir::new().expect("home");
    let ws = TempDir::new().expect("workspace");
    let repo = ws.path().join("repo");
    git_repo::init_repo(&repo);

    let before = hooks_json(home.path(), &repo, &[]);
    let before_hooks = before["hooks"].as_array().expect("hooks array");
    let reinforce_before = before_hooks
        .iter()
        .find(|h| h["name"] == "search-edit-reinforcement")
        .expect("reinforce row present");
    assert_eq!(reinforce_before["source"], "config");
    assert_eq!(reinforce_before["installed"], serde_json::json!(true));
    assert!(
        !home.path().join("config.toml").exists(),
        "reporting default state must not create config.toml"
    );

    let disabled = hooks_json(
        home.path(),
        &repo,
        &["--disable", "search-edit-reinforcement"],
    );
    let row = disabled["hooks"]
        .as_array()
        .expect("hooks array")
        .iter()
        .find(|h| h["name"] == "search-edit-reinforcement")
        .expect("reinforce row present");
    assert_eq!(row["installed"], serde_json::json!(false));

    let config_toml =
        std::fs::read_to_string(home.path().join("config.toml")).expect("read config.toml");
    let parsed: toml::Value = toml::from_str(&config_toml).expect("config.toml parses");
    assert_eq!(
        parsed["reinforce"]["enabled"].as_bool(),
        Some(false),
        "config.toml must persist the disable: {config_toml}"
    );

    let re_enabled = hooks_json(
        home.path(),
        &repo,
        &["--enable", "search-edit-reinforcement"],
    );
    let row = re_enabled["hooks"]
        .as_array()
        .expect("hooks array")
        .iter()
        .find(|h| h["name"] == "search-edit-reinforcement")
        .expect("reinforce row present");
    assert_eq!(row["installed"], serde_json::json!(true));
    let config_toml =
        std::fs::read_to_string(home.path().join("config.toml")).expect("read config.toml");
    let parsed: toml::Value = toml::from_str(&config_toml).expect("config.toml parses");
    assert_eq!(parsed["reinforce"]["enabled"].as_bool(), Some(true));
}
