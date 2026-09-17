#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Hooks journey: install-hooks → hooks --json → disable one → index-code
//! + search-code still work in that repo.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use cli_bin::CliHome;
use std::collections::HashMap;

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
fn install_hooks_toggle_then_search_code() {
    let home = CliHome::new();
    let repo = home.data_dir().parent().expect("parent").join("repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("lib.rs", "fn hook_probe() {}\n")], "init");
    let repo_s = repo.to_str().expect("utf8");

    home.run_ok(&["install-hooks", "--repo", repo_s]);
    let after_install = home.run_json(&["hooks", "--repo", repo_s]);
    let map = installed_by_name(&after_install);
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(map[hook], "{hook} must be installed: {after_install}");
    }

    let after_disable = home.run_json(&["hooks", "--repo", repo_s, "--disable", "post-commit"]);
    let map = installed_by_name(&after_disable);
    assert!(!map["post-commit"], "post-commit flipped: {after_disable}");
    assert!(map["post-merge"]);
    assert!(map["post-checkout"]);

    // A second plain install refreshes the hooks comemory itself wrote — that
    // is how a repo whose hooks predate the worktree-label rule stops
    // registering every `git worktree add` as a repository — and re-installs
    // the one just disabled. No `--force` involved.
    home.run_ok(&["install-hooks", "--repo", repo_s]);
    let after_reinstall = home.run_json(&["hooks", "--repo", repo_s]);
    assert!(
        installed_by_name(&after_reinstall)["post-commit"],
        "a plain re-install must restore the disabled hook: {after_reinstall}"
    );

    // `--force` keeps its one job: clobbering a hook comemory did NOT write.
    let hooks_dir = repo.join(".git").join("hooks");
    std::fs::write(
        hooks_dir.join("post-commit"),
        "#!/bin/sh\necho hand-written\n",
    )
    .expect("write a foreign hook");
    let refused = home
        .bin()
        .args(["install-hooks", "--repo", repo_s])
        .output()
        .expect("run install-hooks");
    assert!(
        !refused.status.success(),
        "install-hooks over a foreign hook must fail without --force"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("--force"),
        "stderr must point at --force: {}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(
        std::fs::read_to_string(hooks_dir.join("post-commit"))
            .expect("read foreign hook")
            .contains("hand-written"),
        "a refused install must leave the foreign hook untouched"
    );
    home.run_ok(&["install-hooks", "--repo", repo_s, "--force"]);
    let after_force = home.run_json(&["hooks", "--repo", repo_s]);
    assert!(
        installed_by_name(&after_force)["post-commit"],
        "--force must replace the foreign hook with comemory's: {after_force}"
    );

    home.run_ok(&["index-code", "--repo", "repo", "--path", repo_s]);
    let search = home.run_json(&["search-code", "hook_probe", "--repo", "repo"]);
    assert!(
        !search["hits"].as_array().expect("hits").is_empty(),
        "search-code after hook toggle: {search}"
    );
}
