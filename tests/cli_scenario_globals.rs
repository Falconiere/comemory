#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Global CLI contract: `--data-dir` (the flag, not just the env var),
//! `--json` before vs after the subcommand, and clap usage exits.

#[path = "common/cli_bin.rs"]
mod cli_bin;

use assert_cmd::Command;
use cli_bin::CliHome;
use serde_json::Value;

#[test]
fn data_dir_flag_wins_over_env_and_isolates_the_store() {
    let home = CliHome::new();
    let other = tempfile::TempDir::new().expect("other data dir");
    let data = home.data_dir();
    let data_s = data.to_str().expect("utf8");

    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", other.path())
        .args([
            "--data-dir",
            data_s,
            "--json",
            "save",
            "flag-isolated body unique token flagdir",
            "--kind",
            "note",
        ])
        .assert()
        .success();

    let listed = home.run_json(&["list"]);
    assert_eq!(listed["total"].as_u64(), Some(1), "flag store: {listed}");

    let leaked = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", other.path())
        .args(["--json", "list"])
        .assert()
        .success();
    let leaked_json: Value =
        serde_json::from_slice(&leaked.get_output().stdout).expect("list json");
    assert_eq!(
        leaked_json["total"].as_u64(),
        Some(0),
        "env-only store must not see the flag save: {leaked_json}"
    );
}

#[test]
fn json_flag_before_and_after_subcommand_match() {
    let home = CliHome::new();
    home.run_ok(&["save", "json placement body", "--kind", "note"]);

    let before = home.bin().args(["--json", "list"]).assert().success();
    let after = home.bin().args(["list", "--json"]).assert().success();
    let left: Value = serde_json::from_slice(&before.get_output().stdout).expect("before");
    let right: Value = serde_json::from_slice(&after.get_output().stdout).expect("after");
    assert_eq!(left, right, "--json before vs after the subcommand");
}

#[test]
fn unknown_subcommand_and_missing_args_are_usage_errors() {
    let home = CliHome::new();
    home.run_ok(&["doctor"]);

    let unknown = home.bin().args(["not-a-command"]).output().expect("run");
    assert_eq!(
        unknown.status.code(),
        Some(2),
        "unknown subcommand → clap 2"
    );

    let missing = home.bin().args(["search"]).output().expect("run");
    assert_eq!(missing.status.code(), Some(2), "missing query → clap 2");
}
