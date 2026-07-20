//! Integration tests for `comemory bandit`: thin golden set exits Unavailable
//! (sysexits 69), matching `comemory tune`.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const TOPICS: &[&str] = &[
    "postgres advisory lock migration ordering",
    "tokio runtime shutdown sequencing bug",
    "clap derive global flag placement",
];

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

fn corpus_with_golden(home: &TempDir, n: usize) -> std::path::PathBuf {
    let mut yaml = String::new();
    for topic in &TOPICS[..n] {
        let save = run_json(home, &["save", topic, "--kind", "note"]);
        let id = save["id"].as_str().expect("save id").to_string();
        yaml.push_str(&format!("- query: {topic}\n  relevant: [{id}]\n"));
    }
    let golden = home.path().join("golden.yaml");
    std::fs::write(&golden, yaml).expect("write golden file");
    golden
}

#[test]
fn bandit_thin_golden_set_exits_unavailable() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 3);
    let assertion = bin(&home)
        .args(["bandit", "--golden"])
        .arg(&golden)
        .arg("--golden-only")
        .assert()
        .failure()
        .code(69);
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("golden pairs"),
        "stderr should explain the thin golden set, got: {stderr:?}"
    );
}
