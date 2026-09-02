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

    // A second plain install must refuse to clobber the hooks that are
    // still there; `--force` overwrites them, which re-installs the one
    // just disabled.
    let refused = home
        .bin()
        .args(["install-hooks", "--repo", repo_s])
        .output()
        .expect("run install-hooks");
    assert!(
        !refused.status.success(),
        "install-hooks over existing hooks must fail without --force"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("--force"),
        "stderr must point at --force: {}",
        String::from_utf8_lossy(&refused.stderr)
    );
    home.run_ok(&["install-hooks", "--repo", repo_s, "--force"]);
    let after_force = home.run_json(&["hooks", "--repo", repo_s]);
    assert!(
        installed_by_name(&after_force)["post-commit"],
        "--force must re-install the disabled hook: {after_force}"
    );

    home.run_ok(&["index-code", "--repo", "repo", "--path", repo_s]);
    let search = home.run_json(&["search-code", "hook_probe", "--repo", "repo"]);
    assert!(
        !search["hits"].as_array().expect("hits").is_empty(),
        "search-code after hook toggle: {search}"
    );
}
