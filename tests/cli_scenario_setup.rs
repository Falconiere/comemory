#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `comemory setup` journey: one real repo taken from nothing to
//! searchable, then the partial-failure and re-run shapes — all against the
//! real binary, a real `git init` tree, and real Rust files (Binding Rule 9).

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::path::Path;
use std::process::{Command, Stdio};

use assert_cmd::prelude::*;
use tempfile::TempDir;

/// Four real Rust files — enough for a genuine symbol walk without indexing
/// a whole crate on every run.
const SOURCES: &[(&str, &str)] = &[
    ("src/alpha.rs", "pub fn alpha() -> u8 { 1 }\n"),
    ("src/beta.rs", "pub fn beta(n: u8) -> u8 { n + 1 }\n"),
    (
        "src/gamma.rs",
        "pub struct Gamma { pub n: u8 }\nimpl Gamma { pub fn new() -> Self { Self { n: 0 } } }\n",
    ),
    ("src/delta.rs", "pub const DELTA: &str = \"delta\";\n"),
];

/// Run `comemory` with stdin closed, returning `(code, stdout, stderr)`.
fn comemory(data_dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::cargo_bin("comemory")
        .unwrap()
        .args(args)
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

/// A real git working tree with [`SOURCES`] committed.
fn repo_with_sources() -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    git_repo::init_repo(repo.path());
    git_commit::commit_files(repo.path(), SOURCES, "seed");
    repo
}

/// The state of one step in a `--json` envelope.
fn state_of(envelope: &serde_json::Value, id: &str) -> String {
    envelope["steps"]
        .as_array()
        .expect("steps array")
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no step `{id}`"))["state"]
        .as_str()
        .expect("state string")
        .to_string()
}

/// setup-01: an empty data dir and a fresh repo become a working code index,
/// and a re-run has nothing left to do.
#[test]
fn setup_yes_indexes_a_fresh_repo_then_search_code_finds_a_symbol() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();
    let repo_arg = repo.path().to_str().unwrap();

    let (code, stdout, stderr) = comemory(
        data.path(),
        &[
            "--json",
            "setup",
            "--yes",
            "--repo",
            repo_arg,
            "--host",
            "codex",
            "--skip",
            "agent-host",
        ],
    );
    assert_eq!(code, 0, "setup --yes failed: {stderr}");
    let envelope: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(state_of(&envelope, "data-dir"), "applied");
    assert_eq!(state_of(&envelope, "git-hooks"), "applied");
    assert_eq!(state_of(&envelope, "index-code"), "applied");
    assert_eq!(envelope["failed"], 0);
    assert!(
        data.path().join("comemory.db").exists(),
        "applying creates the store"
    );
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(repo.path().join(".git/hooks").join(hook).exists());
    }

    // The index is real: a symbol from the committed sources is findable.
    let (code, stdout, stderr) = comemory(data.path(), &["search-code", "gamma", "--json"]);
    assert_eq!(code, 0, "search-code failed: {stderr}");
    assert!(
        stdout.contains("Gamma") || stdout.contains("gamma"),
        "the indexed symbol should be searchable, got {stdout}"
    );

    // Re-running settles: everything is satisfied, nothing is applied.
    let (code, stdout, stderr) = comemory(
        data.path(),
        &[
            "--json",
            "setup",
            "--yes",
            "--repo",
            repo_arg,
            "--host",
            "codex",
            "--skip",
            "agent-host",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let again: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(again["applied"], 0, "idempotent");
    assert_eq!(state_of(&again, "index-code"), "satisfied");
    assert_eq!(state_of(&again, "git-hooks"), "satisfied");
}

/// setup-02: a step that fails does not hide the others, and the command
/// still reports the whole plan before exiting non-zero.
///
/// Unix-only because it needs a genuinely unwritable directory to make a
/// step fail for a real reason rather than a simulated one.
#[cfg(unix)]
#[test]
fn a_failing_step_still_reports_every_step_and_exits_69() {
    use std::os::unix::fs::PermissionsExt as _;

    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    // A read-only data directory: `data-dir` cannot create its layout, so
    // that step fails while `git-hooks` — which writes into the repo, not
    // the data dir — still succeeds beside it.
    let mut perms = std::fs::metadata(data.path()).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(data.path(), perms).unwrap();

    let (code, stdout, stderr) = comemory(
        data.path(),
        &[
            "--json",
            "setup",
            "--yes",
            "--only",
            "data-dir,git-hooks",
            "--repo",
            repo.path().to_str().unwrap(),
            "--host",
            "codex",
        ],
    );

    // Restore write permission first so the TempDir can clean itself up
    // even if an assertion below panics.
    let mut restore = std::fs::metadata(data.path()).unwrap().permissions();
    restore.set_mode(0o755);
    std::fs::set_permissions(data.path(), restore).unwrap();

    let envelope: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must stay valid JSON ({e}): {stdout}\n{stderr}"));
    assert_eq!(
        envelope["steps"].as_array().unwrap().len(),
        7,
        "every step is reported even when one fails"
    );
    assert_eq!(
        state_of(&envelope, "data-dir"),
        "failed",
        "an unwritable data dir is a real apply-time failure"
    );
    assert_eq!(envelope["failed"], 1);
    assert_eq!(
        state_of(&envelope, "git-hooks"),
        "applied",
        "a neighbouring step still runs after a failure"
    );
    for hook in ["post-commit", "post-merge", "post-checkout"] {
        assert!(
            repo.path().join(".git/hooks").join(hook).exists(),
            "{hook} was written despite the other step failing"
        );
    }
    assert_eq!(code, 69, "EX_UNAVAILABLE when a step failed: {stderr}");
    assert!(
        stderr.contains("step(s) failed"),
        "stderr names the failure: {stderr}"
    );
    assert!(
        stderr.contains("data-dir"),
        "stderr names which step: {stderr}"
    );
}

/// setup-03: `--only` narrows the run to one step and leaves the rest alone.
#[test]
fn only_runs_exactly_one_step_and_skips_the_others() {
    let data = tempfile::tempdir().unwrap();
    let repo = repo_with_sources();

    let (code, stdout, stderr) = comemory(
        data.path(),
        &[
            "--json",
            "setup",
            "--yes",
            "--only",
            "git-hooks",
            "--repo",
            repo.path().to_str().unwrap(),
            "--host",
            "codex",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let envelope: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(envelope["applied"], 1);
    assert_eq!(state_of(&envelope, "git-hooks"), "applied");
    assert_eq!(state_of(&envelope, "index-code"), "skipped");
    assert_eq!(state_of(&envelope, "data-dir"), "skipped");
    assert!(
        !data.path().join("comemory.db").exists(),
        "a skipped data-dir step means no store is created"
    );
}
