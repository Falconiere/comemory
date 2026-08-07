#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory eval`: a real save → search → feedback
//! harvest scored through the real binary, plus the empty-set failure mode.

use std::fmt::Write as _;

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
        stdout.contains("recall@5: 1.000 [1.000, 1.000]")
            && stdout.contains("mrr: 1.000 [1.000, 1.000]"),
        "TTY summary should bracket perfect scores with their CIs, got: {stdout:?}"
    );
}

/// Save four memories and a golden file that hits two of them and misses
/// two, so the report has real spread for the bootstrap to bracket.
fn mixed_golden_home() -> (TempDir, std::path::PathBuf) {
    let home = TempDir::new().expect("tempdir");
    let bodies = [
        "postgres advisory locks for migration ordering",
        "tokio runtime shutdown ordering bug in the worker pool",
        "clap derive places global flags before the subcommand",
        "sqlite wal checkpoint starvation under long readers",
    ];
    let ids: Vec<String> = bodies
        .iter()
        .map(|b| json_str(&run_json(&home, &["save", b, "--kind", "note"]), "id"))
        .collect();
    let golden = home.path().join("golden.yaml");
    let mut yaml = String::new();
    for (query, id) in [
        ("advisory lock migration", &ids[0]),
        ("tokio shutdown worker pool", &ids[1]),
        ("zebra quantum nonsense", &ids[2]),
        ("entirely unrelated gibberish phrase", &ids[3]),
    ] {
        let _ = writeln!(yaml, "- query: {query}\n  relevant: [{id}]");
    }
    std::fs::write(&golden, yaml).expect("write golden file");
    (home, golden)
}

#[test]
fn eval_json_adds_confidence_intervals_without_changing_point_estimates() {
    let (home, golden) = mixed_golden_home();
    let golden_arg = golden.to_string_lossy().to_string();
    let args = ["eval", "--golden", &golden_arg, "--golden-only", "--k", "3"];
    let report = run_json(&home, &args);

    // The pre-CI shape is untouched: eval-check.sh's jq paths and floors
    // still read plain numbers here.
    assert!(
        report["recall_at_k"].is_number() && report["mrr"].is_number(),
        "point estimates must stay scalars: {report}"
    );
    assert_eq!(report["queries"].as_u64(), Some(4));

    for (field, point) in [
        ("recall_ci", report["recall_at_k"].as_f64()),
        ("mrr_ci", report["mrr"].as_f64()),
    ] {
        let ci = report[field].as_array().expect("CI serializes as an array");
        assert_eq!(ci.len(), 2, "{field} is a (lo, hi) pair: {report}");
        let lo = ci[0].as_f64().expect("lo is a number");
        let hi = ci[1].as_f64().expect("hi is a number");
        let point = point.expect("point estimate is a number");
        assert!(
            lo <= point && point <= hi,
            "{field} [{lo}, {hi}] must contain {point}"
        );
        assert!(lo < hi, "{field} must have width on a mixed hit/miss set");
    }
}

/// `(point, lo, hi)` for one `label: 0.500 [0.250, 0.750]` group of the TTY
/// summary line.
fn tty_metric(line: &str, label: &str) -> (f64, f64, f64) {
    let rest = line
        .split_once(&format!("{label}: "))
        .expect("summary line carries the label")
        .1;
    let (point, rest) = rest.split_once(" [").expect("point precedes its CI");
    let (bounds, _) = rest.split_once(']').expect("CI closes its bracket");
    let (lo, hi) = bounds.split_once(", ").expect("CI renders as 'lo, hi'");
    let num = |s: &str| s.trim().parse::<f64>().expect("summary numbers parse");
    (num(point), num(lo), num(hi))
}

#[test]
fn eval_tty_brackets_widen_on_a_mixed_hit_miss_set() {
    let (home, golden) = mixed_golden_home();
    let assert = bin(&home)
        .args(["eval", "--golden"])
        .arg(&golden)
        .args(["--golden-only", "--k", "3"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let summary = stdout.lines().next().expect("summary line").to_string();
    assert!(
        summary.contains("(4 queries)"),
        "summary counts the golden pairs, got: {summary:?}"
    );

    // Two hits out of four: unlike the perfect-score case, both bootstrap
    // intervals must render distinct lo/hi bounds around their point.
    for label in ["recall@3", "mrr"] {
        let (point, lo, hi) = tty_metric(&summary, label);
        assert!(
            lo < hi,
            "{label} must bracket a mixed set with distinct bounds, got [{lo}, {hi}] in {summary:?}"
        );
        assert!(
            lo <= point && point <= hi,
            "{label} CI [{lo}, {hi}] must contain {point} in {summary:?}"
        );
    }
}

#[test]
fn eval_reruns_are_byte_identical() {
    let (home, golden) = mixed_golden_home();
    let golden_arg = golden.to_string_lossy().to_string();
    let run = || {
        let assert = bin(&home)
            .args(["--json", "eval", "--golden", &golden_arg])
            .args(["--golden-only", "--k", "3"])
            .assert()
            .success();
        assert.get_output().stdout.clone()
    };
    assert_eq!(
        run(),
        run(),
        "eval is measurement only — two runs on one corpus must match byte for byte"
    );
}
