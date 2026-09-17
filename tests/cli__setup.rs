#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory setup` driven as a real subprocess against a real temp data
//! directory and a real `git init` working tree — no mocks (Binding Rule 9).

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::path::Path;
use std::process::{Command, Stdio};

use assert_cmd::prelude::*;
use tempfile::TempDir;

/// Run `comemory setup` with stdin closed (so the binary never sees a TTY)
/// and return `(exit code, stdout, stderr)`.
fn setup(data_dir: &Path, args: &[&str]) -> (i32, String, String) {
    let mut full = vec!["setup"];
    full.extend_from_slice(args);
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args(&full)
        .env("COMEMORY_DATA_DIR", data_dir)
        .env_remove("COMEMORY_API_KEY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Parse a `--json` run's stdout.
fn setup_json(data_dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let (code, stdout, stderr) = setup(data_dir, &full);
    assert_eq!(code, 0, "setup {full:?} failed: {stderr}");
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {stdout}"))
}

/// The state string of one step in a response envelope.
fn state_of<'a>(envelope: &'a serde_json::Value, id: &str) -> &'a str {
    envelope["steps"]
        .as_array()
        .expect("steps is an array")
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no step `{id}` in {envelope}"))["state"]
        .as_str()
        .expect("state is a string")
}

/// Every step id in an envelope, in the order the plan emitted them.
fn step_ids(envelope: &serde_json::Value) -> Vec<&str> {
    envelope["steps"]
        .as_array()
        .expect("steps is an array")
        .iter()
        .map(|s| s["id"].as_str().expect("id is a string"))
        .collect()
}

/// A real git working tree with two real Rust files committed.
fn repo_with_sources() -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    git_repo::init_repo(repo.path());
    git_commit::commit_files(
        repo.path(),
        &[
            ("src/alpha.rs", "pub fn alpha() -> u8 { 1 }\n"),
            ("src/beta.rs", "pub fn beta(n: u8) -> u8 { n + 1 }\n"),
        ],
        "seed",
    );
    repo
}

#[test]
fn dry_run_json_reports_a_plan_and_creates_no_database() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    let envelope = setup_json(
        data.path(),
        &[
            "--dry-run",
            "--repo",
            repo.path().to_str().unwrap(),
            "--host",
            "codex",
        ],
    );

    assert!(
        envelope["dry_run"].as_bool().unwrap(),
        "dry_run must be a JSON bool"
    );
    assert_eq!(envelope["applied"].as_u64().unwrap(), 0);
    assert_eq!(envelope["failed"].as_u64().unwrap(), 0);
    assert_eq!(
        step_ids(&envelope),
        [
            "data-dir",
            "agent-host",
            "git-hooks",
            "index-code",
            "index-docs",
            "reinforce",
            "cloud-auth",
        ],
        "every published step id appears exactly once, in order"
    );
    assert_eq!(state_of(&envelope, "git-hooks"), "pending");
    assert_eq!(state_of(&envelope, "index-code"), "pending");
    assert!(
        !data.path().join("comemory.db").exists(),
        "a dry run must not create the database"
    );
}

#[test]
fn cloud_auth_is_reported_not_applied_and_issues_no_request() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    let envelope = setup_json(
        data.path(),
        &[
            "--yes",
            "--only",
            "cloud-auth",
            "--repo",
            repo.path().to_str().unwrap(),
            "--host",
            "codex",
        ],
    );

    assert_eq!(state_of(&envelope, "cloud-auth"), "unavailable");
    assert_eq!(
        envelope["failed"].as_u64().unwrap(),
        0,
        "unavailable is never a failure"
    );
    let reason = envelope["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "cloud-auth")
        .unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        reason.contains("comemory auth login"),
        "the reason names the remedy: {reason}"
    );
    assert_eq!(
        envelope["applied"].as_u64().unwrap(),
        0,
        "setup never runs the device flow itself"
    );
}

#[test]
fn a_non_git_directory_reports_unavailable_repo_steps_and_exits_zero() {
    let data = tempfile::tempdir().unwrap();
    let plain = tempfile::tempdir().unwrap();

    let envelope = setup_json(
        data.path(),
        &[
            "--dry-run",
            "--repo",
            plain.path().to_str().unwrap(),
            "--host",
            "codex",
        ],
    );

    for id in ["git-hooks", "index-code", "index-docs"] {
        assert_eq!(state_of(&envelope, id), "unavailable", "for {id}");
    }
    assert!(
        envelope["repo"].is_null(),
        "no repo context outside a worktree"
    );
}

#[test]
fn an_unknown_step_id_exits_64_and_names_it() {
    let data = tempfile::tempdir().unwrap();
    let (code, _, stderr) = setup(data.path(), &["--only", "bogus"]);

    assert_eq!(code, 64, "EX_USAGE for a bad argument value");
    assert!(stderr.contains("bogus"), "names the bad id: {stderr}");
    assert!(stderr.contains("--only"), "names the flag: {stderr}");
    assert!(stderr.contains("git-hooks"), "lists known ids: {stderr}");
    assert!(
        !data.path().join("comemory.db").exists(),
        "validation runs before any probing"
    );
}

#[test]
fn dry_run_and_yes_are_mutually_exclusive() {
    let data = tempfile::tempdir().unwrap();
    let (code, _, stderr) = setup(data.path(), &["--yes", "--dry-run"]);

    assert_eq!(code, 2, "clap rejects conflicting flags with its own code");
    assert!(
        stderr.contains("cannot be used with"),
        "clap explains the conflict: {stderr}"
    );
}

#[test]
fn a_closed_stdin_prints_the_plan_and_the_yes_command() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    // Neither --yes nor --dry-run nor --json: with no terminal this must
    // fall back to reporting rather than failing on a prompt it cannot show.
    let (code, stdout, stderr) = setup(
        data.path(),
        &["--repo", repo.path().to_str().unwrap(), "--host", "codex"],
    );

    assert_eq!(code, 0, "a non-TTY invocation is not an error: {stderr}");
    assert!(
        stdout.contains("comemory setup --yes"),
        "tells the operator how to proceed: {stdout}"
    );
    assert!(
        !data.path().join("comemory.db").exists(),
        "reporting changes nothing"
    );
}

#[test]
fn applying_git_hooks_writes_them_and_a_second_run_is_satisfied() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let args = [
        "--yes",
        "--only",
        "git-hooks",
        "--repo",
        repo.path().to_str().unwrap(),
        "--host",
        "codex",
    ];

    let first = setup_json(data.path(), &args);
    assert_eq!(state_of(&first, "git-hooks"), "applied");
    assert_eq!(first["applied"].as_u64().unwrap(), 1);
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(
            repo.path().join(".git/hooks").join(hook).exists(),
            "{hook} must exist on disk"
        );
    }

    let second = setup_json(data.path(), &args);
    assert_eq!(state_of(&second, "git-hooks"), "satisfied");
    assert_eq!(
        second["applied"].as_u64().unwrap(),
        0,
        "idempotent: nothing left to do"
    );
}

#[test]
fn a_hand_written_hook_is_never_clobbered() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let hook = repo.path().join(".git/hooks/post-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let original = "#!/bin/sh\necho mine\n";
    std::fs::write(&hook, original).unwrap();

    let envelope = setup_json(
        data.path(),
        &[
            "--yes",
            "--repo",
            repo.path().to_str().unwrap(),
            "--host",
            "codex",
        ],
    );

    assert_eq!(state_of(&envelope, "git-hooks"), "unavailable");
    assert_eq!(
        std::fs::read_to_string(&hook).unwrap(),
        original,
        "the operator's own hook is left byte-identical"
    );
}

#[test]
fn the_tty_summary_is_stable() {
    let data = tempfile::tempdir().unwrap();
    let plain = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();

    // Deterministic by construction: a non-git target (so every repo-scoped
    // step is unavailable) and an empty PATH (so no agent-host CLI can be
    // found). Without the empty PATH this snapshot would record whether the
    // developer happens to have `claude`/`codex` installed and disagree with
    // CI.
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args([
            "setup",
            "--dry-run",
            "--repo",
            plain.path().to_str().unwrap(),
            "--host",
            "codex",
        ])
        .env("COMEMORY_DATA_DIR", data.path())
        .env("PATH", empty_path.path())
        .env_remove("COMEMORY_API_KEY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(code, 0, "{stderr}");

    // The data dir is a random tempdir; replace it so the snapshot is stable.
    let scrubbed = stdout.replace(&data.path().display().to_string(), "<DATA_DIR>");
    // Strip ANSI styling so the snapshot reads as plain text.
    let plain_text = strip_ansi(&scrubbed);
    insta::assert_snapshot!(plain_text);
}

/// Remove ANSI SGR sequences so a snapshot shows the text, not the styling.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // Skip "[...m" — every sequence this renderer emits is an SGR.
        for next in chars.by_ref() {
            if next == 'm' {
                break;
            }
        }
    }
    out
}
