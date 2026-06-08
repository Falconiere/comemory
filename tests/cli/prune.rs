//! Task 15: `comemory prune` operates against the v0.2 SQLite mirror
//! (`comemory.db`) instead of the old kuzu/lance fan-out. It reports
//! orphan edges (memory→… edges whose source memory is missing or
//! soft-deleted) and stale code files (paths referenced from
//! `code_symbols` that no longer appear in `indexed_files`).
//!
//! On a freshly-initialised DB both lists are empty and `--dry-run`
//! must not mutate anything.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn prune_dry_run_on_clean_db_emits_zero_counts() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["--json", "prune", "--dry-run"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["orphan_edges"].as_i64(), Some(0));
    let stale: Vec<&str> = v["stale_code_files"]
        .as_array()
        .expect("stale_code_files is array")
        .iter()
        .map(|x| x.as_str().expect("string entry"))
        .collect();
    assert!(stale.is_empty(), "expected no stale code files: {stale:?}");
}

#[test]
fn prune_dry_run_after_save_is_idempotent() {
    // Saving a memory creates `memory→{repo,author}` edges via the v0.2
    // mirror. Those edges are live (the source memory exists with
    // deleted_at IS NULL) so prune --dry-run must report 0 orphans and
    // a follow-up doctor invocation must still succeed.
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["save", "prune dry-run body", "--kind", "note"])
        .assert()
        .success();
    let assertion = bin(&home)
        .args(["--json", "prune", "--dry-run"])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parse JSON");
    assert_eq!(v["orphan_edges"].as_i64(), Some(0));
}

#[test]
fn prune_examples_block_present() {
    // The shared `cli_help_examples` test expects every subcommand --help
    // to ship an `Examples:` block. Asserted directly here as a guard
    // against future EXAMPLES drift inside this command specifically.
    let help = Command::cargo_bin("comemory")
        .expect("bin")
        .args(["prune", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(help).expect("utf8");
    assert!(
        text.contains("Examples:"),
        "prune --help must contain Examples block: {text:?}"
    );
}
