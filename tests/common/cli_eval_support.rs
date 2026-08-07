#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Shared corpus + golden-file builder for `src/api/tests/eval.rs`,
//! `src/api/tests/tune.rs`, and `src/api/tests/bandit.rs`.

use assert_cmd::Command;
use serde_json::Value;
use std::fmt::Write as _;
use tempfile::TempDir;

/// Lexically distinct memory bodies; each doubles as its own golden query
/// (mirrors `tests/cli__tune.rs`'s fixture so the two suites stay
/// comparable).
pub const TOPICS: &[&str] = &[
    "postgres advisory lock migration ordering",
    "tokio runtime shutdown sequencing bug",
    "clap derive global flag placement",
    "sqlite fts5 tokenizer unicode normalization",
    "docker compose volume mount permissions",
    "kubernetes ingress certificate renewal",
    "redis cache eviction policy tuning",
    "graphql federation gateway timeout",
    "webpack chunk splitting heuristics",
    "terraform state locking dynamodb",
];

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
pub fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Run a `--json` subcommand to success and parse its stdout envelope.
pub fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

/// Save the first `n` [`TOPICS`] through the real binary and write a golden
/// YAML pairing each body with its saved id. Returns the golden path.
pub fn corpus_with_golden(home: &TempDir, n: usize) -> std::path::PathBuf {
    let mut yaml = String::new();
    for topic in &TOPICS[..n] {
        let save = run_json(home, &["save", topic, "--kind", "note"]);
        let id = save["id"].as_str().expect("save id").to_string();
        let _ = writeln!(yaml, "- query: {topic}\n  relevant: [{id}]");
    }
    let golden = home.path().join("golden.yaml");
    std::fs::write(&golden, yaml).expect("write golden file");
    golden
}
