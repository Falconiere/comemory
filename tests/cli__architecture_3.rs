#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Drift checking, the `learn` wrapper against real agent scripts, and the
//! recall regression that keeps a stored model out of everyday results.

use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;

#[path = "common/architecture_repo.rs"]
mod architecture_repo;

use architecture_repo::{
    architecture_json, bin, commit_files, index_repo, reindex_full, scaffold_and_save,
};

/// Write an executable shell script acting as the agent and return its path.
fn agent_script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    path.to_string_lossy().to_string()
}

/// Top-`n` hit ids of `comemory find <query>`.
fn find_ids(home: &TempDir, query: &str, n: usize) -> Vec<String> {
    let out = bin(home).args(["find", query, "--json"]).assert().success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("json");
    v["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .take(n)
        .map(|h| h["id"].as_str().expect("id").to_string())
        .collect()
}

#[test]
fn a_freshly_saved_scaffold_reports_no_drift() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    scaffold_and_save(&home, "r", ws.path());

    let drift = architecture_json(&home, &["check", "--repo", "r", "--json"]);
    assert_eq!(drift["drift_count"], 0, "{drift}");
    assert_eq!(drift["repo"], "r");
    assert!(drift["model_id"].as_str().is_some(), "{drift}");
}

#[test]
fn a_moved_file_shows_up_as_a_stale_member_and_an_unmapped_directory() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    let repo = index_repo(&home, ws.path(), "r");
    scaffold_and_save(&home, "r", ws.path());

    std::fs::remove_file(repo.join("src/b/two.rs")).expect("remove");
    commit_files(
        &repo,
        &[("src/c/three.rs", "pub fn gamma() {}\n")],
        "move the module",
    );
    reindex_full(&home, &repo, "r");

    let drift = architecture_json(&home, &["check", "--repo", "r", "--json"]);
    let stale: Vec<&str> = drift["stale_members"]
        .as_array()
        .expect("stale_members")
        .iter()
        .map(|s| s["member"].as_str().expect("member"))
        .collect();
    assert!(stale.contains(&"src/b"), "{drift}");
    let unmapped: Vec<&str> = drift["unmapped"]
        .as_array()
        .expect("unmapped")
        .iter()
        .map(|u| u["path"].as_str().expect("path"))
        .collect();
    assert!(unmapped.contains(&"src/c"), "{drift}");
    assert!(
        drift["drift_count"].as_u64().expect("count") >= 2,
        "{drift}"
    );
}

#[test]
fn two_mined_kinds_between_one_pair_are_one_missing_edge() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    // The fixture mines BOTH an `imports` and a `co_changed` edge between the
    // two directories. A model that keeps the components but drops the edges
    // has omitted one relation, not two.
    let mut model = architecture_json(&home, &["scaffold", "--repo", "r", "--json"]);
    model["edges"] = Value::Array(Vec::new());
    let path = ws.path().join("edgeless.json");
    std::fs::write(&path, serde_json::to_string_pretty(&model).expect("json")).expect("write");
    bin(&home)
        .args(["architecture", "save"])
        .arg(&path)
        .args(["--repo", "r"])
        .assert()
        .success();

    let drift = architecture_json(&home, &["check", "--repo", "r", "--json"]);
    let missing = drift["missing_edges"].as_array().expect("missing_edges");
    assert_eq!(missing.len(), 1, "{drift}");
    assert_eq!(missing[0]["from"], "src_a");
    assert_eq!(missing[0]["to"], "src_b");
    // One omission, both mined kinds named on it.
    assert_eq!(
        missing[0]["kinds"],
        serde_json::json!(["imports", "co_changed"])
    );
    assert_eq!(missing[0]["weight"], 1);
}

#[test]
fn learn_hands_the_scaffold_to_the_agent_and_saves_what_it_prints() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    // The agent reads the prompt file, renames one component, and prints the
    // model — the same shape a real `claude -p` run is asked for.
    let script = agent_script(
        ws.path(),
        "agent.sh",
        "set -e\n\
         grep -q '\"schema\": 1' \"$1\" || { echo 'no scaffold in prompt' >&2; exit 9; }\n\
         sed -n '/^```json$/,/^```$/p' \"$1\" | sed '1d;$d' \\\n\
           | sed 's/\"source\": \"scaffold\"/\"source\": \"agent\"/' \\\n\
           | sed 's/\"name\": \"a\"/\"name\": \"Alpha layer\"/'\n",
    );
    let learned = architecture_json(
        &home,
        &[
            "learn",
            "--repo",
            "r",
            "--command",
            &format!("sh {script} {{prompt_file}}"),
            "--json",
        ],
    );
    assert!(learned["saved"]["id"].as_str().is_some(), "{learned}");

    let prompt = std::fs::read_to_string(learned["prompt_path"].as_str().expect("prompt_path"))
        .expect("prompt file");
    assert!(
        prompt.contains("\"schema\": 1"),
        "scaffold must be in the prompt"
    );
    assert!(
        prompt.contains("Describe the architecture of `r`"),
        "{prompt}"
    );

    let stored = architecture_json(&home, &["show", "--repo", "r", "--json"]);
    assert_eq!(stored["source"], "agent", "{stored}");
    let names: Vec<&str> = stored["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert!(names.contains(&"Alpha layer"), "{names:?}");
}

#[test]
fn a_dry_run_writes_the_prompt_and_starts_nothing() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let marker = ws.path().join("agent-ran");
    let script = agent_script(
        ws.path(),
        "marker.sh",
        &format!("touch {}\necho '{{}}'\n", marker.display()),
    );
    let learned = architecture_json(
        &home,
        &[
            "learn",
            "--repo",
            "r",
            "--dry-run",
            "--command",
            &format!("sh {script} {{prompt_file}}"),
            "--json",
        ],
    );
    let prompt = std::fs::read_to_string(learned["prompt_path"].as_str().expect("path"))
        .expect("prompt written");
    assert!(prompt.contains("\"schema\": 1"), "{prompt}");
    assert!(!marker.exists(), "the agent must not have been started");
    assert!(learned.get("saved").is_none(), "{learned}");
}

#[test]
fn learn_refuses_a_template_without_a_placeholder_before_spawning() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let marker = ws.path().join("agent-ran");
    let script = agent_script(
        ws.path(),
        "marker2.sh",
        &format!("touch {}\necho '{{}}'\n", marker.display()),
    );
    let out = bin(&home)
        .args([
            "architecture",
            "learn",
            "--repo",
            "r",
            "--command",
            &format!("sh {script}"),
        ])
        .assert()
        .failure()
        .code(64);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(stderr.contains("{prompt_file}"), "{stderr}");
    assert!(!marker.exists(), "nothing may be spawned");
}

#[test]
fn a_failing_agent_surfaces_its_status_and_stderr_and_prose_is_a_data_error() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let failing = agent_script(
        ws.path(),
        "fail.sh",
        "echo 'model service is down' >&2\nexit 3\n",
    );
    let out = bin(&home)
        .args([
            "architecture",
            "learn",
            "--repo",
            "r",
            "--command",
            &format!("sh {failing} {{prompt_file}}"),
        ])
        .assert()
        .failure()
        .code(70);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(stderr.contains("model service is down"), "{stderr}");
    assert!(stderr.contains("agent command failed"), "{stderr}");

    let prose = agent_script(ws.path(), "prose.sh", "echo 'I could not read the repo.'\n");
    let out = bin(&home)
        .args([
            "architecture",
            "learn",
            "--repo",
            "r",
            "--command",
            &format!("sh {prose} {{prompt_file}}"),
        ])
        .assert()
        .failure()
        .code(65);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(stderr.starts_with("error: json:"), "{stderr}");
    assert!(
        stderr.contains("expected value at line 1 column 1"),
        "the parse error must name what failed: {stderr}"
    );
}

#[test]
fn storing_a_model_does_not_displace_everyday_recall() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let before = find_ids(&home, "two", 3);
    scaffold_and_save(&home, "r", ws.path());
    let after = find_ids(&home, "two", 3);

    assert_eq!(before, after, "the architecture memory displaced a hit");
    let stored = architecture_json(&home, &["show", "--repo", "r", "--json"]);
    assert!(
        !stored["components"]
            .as_array()
            .expect("components")
            .is_empty()
    );
}
