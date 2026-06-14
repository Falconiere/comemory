//! Integration tests for `comemory eval`: a real save → search → feedback
//! harvest scored through the real binary, plus the empty-set failure mode.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Run a `--json` subcommand to success and parse its stdout envelope.
fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

/// Extract a required string field from a JSON envelope.
fn json_str(v: &Value, field: &str) -> String {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("envelope field {field:?} missing in {v}"))
        .to_string()
}

#[test]
fn eval_scores_harvested_feedback_through_real_binary() {
    let home = TempDir::new().expect("tempdir");
    let save = run_json(
        &home,
        &[
            "save",
            "postgres advisory locks for migration ordering",
            "--kind",
            "decision",
        ],
    );
    let memory_id = json_str(&save, "id");
    run_json(
        &home,
        &["save", "tokio shutdown ordering bug", "--kind", "bug"],
    );

    let search = run_json(&home, &["search", "advisory lock"]);
    let query_id = json_str(&search, "query_id");
    run_json(&home, &["feedback", &query_id, "--used", &memory_id]);

    let report = run_json(&home, &["eval"]);
    assert_eq!(report["k"].as_u64(), Some(3), "default k is 3");
    assert_eq!(report["queries"].as_u64(), Some(1));
    assert_eq!(
        report["recall_at_k"].as_f64(),
        Some(1.0),
        "harvested pair must score perfectly: {report}"
    );
    assert_eq!(report["mrr"].as_f64(), Some(1.0));
    let results = report["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["rank_of_first_hit"].as_u64(), Some(1));
    assert_eq!(results[0]["query"].as_str(), Some("advisory lock"));
}

#[test]
fn eval_empty_data_dir_exits_unavailable() {
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home).args(["eval"]).assert().failure().code(69);
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("no golden pairs"),
        "stderr should explain the empty golden set, got: {stderr:?}"
    );
}

#[test]
fn eval_golden_only_requires_golden_file_flag() {
    // clap's `requires` guard: --golden-only without --golden is a usage
    // error at parse time, before any data dir is touched.
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["eval", "--golden-only"])
        .assert()
        .failure();
    let db_path = home.path().join(".comemory").join("comemory.db");
    assert!(!db_path.exists(), "usage error must not create the db");
}

#[test]
fn eval_golden_file_tty_summary_line() {
    let home = TempDir::new().expect("tempdir");
    let save = run_json(
        &home,
        &[
            "save",
            "postgres advisory locks for migration ordering",
            "--kind",
            "decision",
        ],
    );
    let memory_id = json_str(&save, "id");
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: advisory lock\n  relevant: [{memory_id}]\n"),
    )
    .expect("write golden file");

    let assert = bin(&home)
        .args(["eval", "--golden"])
        .arg(&golden)
        .args(["--golden-only", "--k", "5"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        stdout.contains("recall@5: 1.000") && stdout.contains("mrr: 1.000"),
        "TTY summary should report perfect scores, got: {stdout:?}"
    );
}
