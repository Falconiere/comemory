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
fn tune_json_ranks_the_sampled_candidates_and_is_deterministic() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let first = tune_stdout(&home, &golden, &[]);
    let v: Value = serde_json::from_str(first.trim()).expect("parse tune JSON");
    let ranked = v["report"]["ranked"].as_array().expect("ranked array");
    assert_eq!(
        ranked.len(),
        64,
        "the default [tune] samples budget draws 64 of the 729 grid points"
    );
    assert_eq!(v["report"]["golden_pairs"].as_u64(), Some(10));
    assert_eq!(v["applied"].as_bool(), Some(false), "no --apply, no write");
    let c = &ranked[0]["candidate"];
    assert!(
        c["graph_hops"].is_u64() && c["graph_seeds"].is_u64(),
        "candidates must carry the graph knobs, got: {c}"
    );

    // No --seed: the seed is derived from unchanged inputs, so the second
    // run draws the same candidates and scores them the same way.
    let second = tune_stdout(&home, &golden, &[]);
    assert_eq!(first, second, "two tune runs must be byte-identical");
}

#[test]
fn tune_seed_flag_pins_the_candidate_draw() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let first = tune_stdout(&home, &golden, &["--seed", "42"]);
    let second = tune_stdout(&home, &golden, &["--seed", "42"]);
    assert_eq!(first, second, "--seed 42 must reproduce the report exactly");

    let other = tune_stdout(&home, &golden, &["--seed", "7"]);
    assert_ne!(
        first, other,
        "a different --seed must draw a different candidate set"
    );
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

/// Save one memory through the binary and return its id.
fn save(home: &TempDir, body: &str, tags: &[&str]) -> String {
    let mut args = vec!["save", body, "--kind", "note"];
    for t in tags {
        args.push("--tags");
        args.push(t);
    }
    run_json(home, &args)["id"]
        .as_str()
        .expect("save id")
        .to_string()
}

/// The smallest corpus where a tuned knob actually moves the score:
/// "postgres" appears only in the tagged memory's tags, so a baseline
/// weighting the tags column 0 ranks the body match ahead and misses,
/// while the grid's (1.0, 3.0) finds the tagged one. Returns the golden
/// file and the config.toml (pre-seeded with an unrelated key).
fn tag_discriminated_fixture(home: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    save(home, "postgres advisory lock migration ordering", &[]);
    let tagged = save(
        home,
        "kubernetes ingress certificate renewal notes",
        &["postgres"],
    );
    let plain = save(home, "terraform state locking dynamodb backend", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!(
            "- query: postgres\n  relevant: [{tagged}]\n\
             - query: terraform state locking\n  relevant: [{plain}]\n"
        ),
    )
    .expect("write golden file");

    let config = home.path().join(".comemory").join("config.toml");
    std::fs::write(
        &config,
        "embed_hint = \"keep-me\"\n\
         [retrieval]\n\
         bm25_weights = [1.0, 0.0]\n",
    )
    .expect("write starting config.toml");
    (golden, config)
}

#[test]
fn tune_apply_writes_the_graph_knobs_and_keeps_unrelated_keys() {
    let home = TempDir::new().expect("tempdir");
    let (golden, config) = tag_discriminated_fixture(&home);

    let assert = bin(&home)
        .env("COMEMORY_TUNE_MIN_GOLDEN", "2")
        .args(["--json", "tune", "--golden"])
        .arg(&golden)
        .args(["--golden-only", "--k", "1", "--seed", "42", "--apply"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse tune JSON");
    assert_eq!(
        v["applied"].as_bool(),
        Some(true),
        "a zero-weight tags column must be beatable, report: {v}"
    );

    let raw = std::fs::read_to_string(&config).expect("read applied config.toml");
    let winner = &v["report"]["ranked"][0]["candidate"];
    let hops = winner["graph_hops"].as_u64().expect("winner graph_hops");
    let seeds = winner["graph_seeds"].as_u64().expect("winner graph_seeds");
    assert!(
        raw.contains(&format!("graph_hops = {hops}")),
        "winner's graph_hops must land in [retrieval] as an integer, got: {raw:?}"
    );
    assert!(
        raw.contains(&format!("graph_seeds = {seeds}")),
        "winner's graph_seeds must land in [retrieval] as an integer, got: {raw:?}"
    );
    assert!(
        raw.contains("keep-me"),
        "unrelated keys must survive the rewrite, got: {raw:?}"
    );

    // The rewritten file must reload and pass Config::validate — a float
    // where a u32 belongs, or an out-of-range hop count, would fail here.
    bin(&home).args(["search", "postgres"]).assert().success();
}
