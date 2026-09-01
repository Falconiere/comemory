#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory eval --history`: real `eval_runs` rows
//! written by `comemory eval`/`tune`/`bandit` through the real binary
//! (AC-37, AC-38, AC-39).

use std::fmt::Write as _;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Lexically distinct memory bodies; each doubles as its own golden query
/// (mirrors `tests/cli__eval.rs` / `tests/cli__tune.rs`'s fixtures).
const TOPICS: &[&str] = &[
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

/// Save the first `n` [`TOPICS`] through the real binary and write a golden
/// YAML pairing each body with its saved id. Returns the golden path.
fn corpus_with_golden(home: &TempDir, n: usize) -> std::path::PathBuf {
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

/// AC-37: running `comemory eval` twice against a real golden set writes
/// two `eval_runs` rows; `eval --history --json` returns them newest-first
/// with the real recall/MRR each run itself printed.
#[test]
fn running_eval_twice_writes_two_rows_readable_newest_first() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 10);
    let golden_arg = golden.to_string_lossy().into_owned();
    let eval_args = ["eval", "--golden", &golden_arg, "--golden-only", "--k", "3"];

    let first_report = run_json(&home, &eval_args);
    let second_report = run_json(&home, &eval_args);

    let history = run_json(&home, &["eval", "--history"]);
    let rows = history.as_array().expect("history is a JSON array");
    assert_eq!(rows.len(), 2, "two eval runs must write two eval_runs rows");

    // Newest-first: the second run's row comes back before the first's.
    assert!(
        rows[0]["at"].as_str().expect("at") >= rows[1]["at"].as_str().expect("at"),
        "history must be newest-first, got {history}"
    );
    for row in rows {
        assert_eq!(row["kind"].as_str(), Some("eval"));
        assert_eq!(row["applied"].as_bool(), Some(false));
        assert!(row["knobs"].is_object());
    }

    // Cross-check the row's real recall/MRR against what the run itself
    // printed (byte-identical corpus, so both eval runs score identically).
    assert_eq!(
        rows[1]["recall"].as_f64(),
        first_report["recall_at_k"].as_f64(),
        "first row's recall must match the first eval run's own report"
    );
    assert_eq!(
        rows[1]["mrr"].as_f64(),
        first_report["mrr"].as_f64(),
        "first row's mrr must match the first eval run's own report"
    );
    assert_eq!(
        rows[0]["recall"].as_f64(),
        second_report["recall_at_k"].as_f64()
    );
    assert_eq!(rows[0]["mrr"].as_f64(), second_report["mrr"].as_f64());
}

/// AC-38: `comemory tune` writes an `eval_runs` row with `kind == "tune"`
/// and the winning knobs JSON; `--apply` sets `applied == 1`.
#[test]
fn tune_writes_an_eval_runs_row_with_winning_knobs_and_applied_flag() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());
    let golden_arg = golden.to_string_lossy().into_owned();

    let tune_resp = run_json(
        &home,
        &["tune", "--golden", &golden_arg, "--golden-only", "--apply"],
    );
    let applied = tune_resp["applied"].as_bool().expect("applied bool");

    let history = run_json(&home, &["eval", "--history"]);
    let rows = history.as_array().expect("history array");
    assert_eq!(rows.len(), 1, "one row for the one tune run");
    let row = &rows[0];
    assert_eq!(row["kind"].as_str(), Some("tune"));
    assert_eq!(
        row["applied"].as_bool(),
        Some(applied),
        "row's applied must mirror whether --apply actually rewrote config.toml"
    );
    let winner = &tune_resp["report"]["ranked"][0]["candidate"];
    assert_eq!(
        row["knobs"]["rrf_k"].as_f64(),
        winner["rrf_k"].as_f64(),
        "row's knobs must carry the winning candidate: {row}"
    );
    assert_eq!(
        row["knobs"]["graph_hops"].as_u64(),
        winner["graph_hops"].as_u64()
    );
}

/// AC-39: `eval --history --limit N` returns at most `N` rows, and on an
/// empty table returns an empty array with exit 0.
#[test]
fn history_limit_caps_rows_and_empty_table_exits_zero() {
    let home = TempDir::new().expect("tempdir");

    // Empty table first: no eval/tune/bandit run has ever happened here.
    let empty = run_json(&home, &["eval", "--history"]);
    assert_eq!(
        empty.as_array().expect("array"),
        &Vec::<Value>::new(),
        "an empty table must return an empty array, not an error"
    );
    bin(&home).args(["eval", "--history"]).assert().success();

    let golden = corpus_with_golden(&home, 10);
    let golden_arg = golden.to_string_lossy().into_owned();
    for _ in 0..3 {
        run_json(
            &home,
            &["eval", "--golden", &golden_arg, "--golden-only", "--k", "3"],
        );
    }

    let capped = run_json(&home, &["eval", "--history", "--limit", "2"]);
    assert_eq!(
        capped.as_array().expect("array").len(),
        2,
        "--limit 2 must cap the returned rows even though 3 runs happened"
    );
}

/// `--history` is a clap conflict with the scoring-mode flags, not a
/// silent precedence rule.
#[test]
fn history_conflicts_with_the_golden_set_flags() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["eval", "--history", "--golden", "golden.yaml"])
        .assert()
        .failure();
    bin(&home)
        .args(["eval", "--history", "--k", "5"])
        .assert()
        .failure();
}

/// `--limit` without `--history` is a clap usage error (`requires`).
#[test]
fn limit_without_history_is_a_usage_error() {
    let home = TempDir::new().expect("tempdir");
    bin(&home).args(["eval", "--limit", "5"]).assert().failure();
}
