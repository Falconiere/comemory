//! Integration tests for `comemory tune`: a real corpus saved through the
//! binary, grid-search determinism, the min-golden floor (and its env
//! hook), and the opt-in `--apply` write.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Lexically distinct memory bodies; each doubles as its own golden query.
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

/// Save the first `n` TOPICS through the real binary and write a golden
/// YAML pairing each body with its saved id. Returns the golden path.
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

/// Run `comemory tune --golden <file> --golden-only --json` to success and
/// return raw stdout (for byte-identity checks) plus extra args.
fn tune_stdout(home: &TempDir, golden: &std::path::Path, extra: &[&str]) -> String {
    let assert = bin(home)
        .args(["--json", "tune", "--golden"])
        .arg(golden)
        .arg("--golden-only")
        .args(extra)
        .assert()
        .success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout")
}

#[test]
fn tune_json_ranks_81_candidates_and_is_deterministic() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let first = tune_stdout(&home, &golden, &[]);
    let v: Value = serde_json::from_str(first.trim()).expect("parse tune JSON");
    let ranked = v["report"]["ranked"].as_array().expect("ranked array");
    assert_eq!(ranked.len(), 81, "grid must score all 81 candidates");
    assert_eq!(v["report"]["golden_pairs"].as_u64(), Some(10));
    assert_eq!(v["applied"].as_bool(), Some(false), "no --apply, no write");

    let second = tune_stdout(&home, &golden, &[]);
    assert_eq!(first, second, "two tune runs must be byte-identical");
}

#[test]
fn tune_env_min_golden_hook_lowers_the_floor() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 3);
    bin(&home)
        .env("COMEMORY_TUNE_MIN_GOLDEN", "3")
        .args(["tune", "--golden"])
        .arg(&golden)
        .arg("--golden-only")
        .assert()
        .success();
}

#[test]
fn tune_thin_golden_set_exits_unavailable() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 3);
    let assertion = bin(&home)
        .args(["tune", "--golden"])
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

#[test]
fn tune_apply_writes_config_only_when_winner_beats_baseline() {
    // The winner may legitimately tie the baseline (a perfect-scoring
    // corpus leaves no headroom), in which case --apply must NOT write.
    // We branch on the JSON `applied` field rather than forcing one
    // outcome: whichever branch fires, the on-disk state must agree.
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let stdout = tune_stdout(&home, &golden, &["--apply"]);
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse tune JSON");
    let applied = v["applied"].as_bool().expect("applied bool");
    let config = home.path().join(".comemory").join("config.toml");
    if applied {
        let raw = std::fs::read_to_string(&config).expect("read applied config.toml");
        assert!(
            raw.contains("[retrieval]") && raw.contains("rrf_k"),
            "applied config.toml must carry the winner knobs, got: {raw:?}"
        );
        assert!(
            raw.contains("[rank]") && raw.contains("mmr_lambda"),
            "applied config.toml must carry the rank knobs, got: {raw:?}"
        );
    } else {
        assert!(
            !config.exists(),
            "baseline won or tied: --apply must not create config.toml"
        );
    }
}
