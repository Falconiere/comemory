#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Learning-loop journey from `docs/guides/ranking-and-eval.md`:
//! search → feedback → mine → eval → tune → bandit.
//!
//! Bandit runs against a one-arm `[tune]` grid so the cartesian product
//! stays at 1 (the default 729-arm grid is not acceptable in this suite).

#[path = "common/cli_bin.rs"]
mod cli_bin;

use cli_bin::CliHome;
use std::fmt::Write as _;
use std::path::PathBuf;

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

const ONE_ARM_GRID: &str = "\
[tune]
rrf_k_grid = [60.0]
decay_grid = [0.5]
mmr_lambda_grid = [0.7]
bm25_grid = [[1.0, 3.0]]
graph_hops_grid = [0]
graph_seeds_grid = [8]
samples = 0
";

fn corpus_with_golden(home: &CliHome) -> PathBuf {
    let mut yaml = String::new();
    for topic in TOPICS {
        let save = home.run_json(&["save", topic, "--kind", "note"]);
        let id = save["id"].as_str().expect("save id");
        let _ = writeln!(yaml, "- query: {topic}\n  relevant: [{id}]");
    }
    let golden = home
        .data_dir()
        .parent()
        .expect("parent")
        .join("golden.yaml");
    std::fs::write(&golden, yaml).expect("write golden");
    golden
}

fn write_one_arm_grid(home: &CliHome) {
    let dir = home.data_dir();
    std::fs::create_dir_all(&dir).expect("data dir");
    std::fs::write(dir.join("config.toml"), ONE_ARM_GRID).expect("write config.toml");
}

#[test]
fn search_feedback_mine_eval_tune_bandit() {
    let home = CliHome::new();
    let golden = corpus_with_golden(&home);
    let golden_s = golden.to_str().expect("utf8").to_string();
    write_one_arm_grid(&home);

    let dim_save = home.run_json(&[
        "save",
        "VecDimMismatch error raised by the dim guard",
        "--kind",
        "bug",
    ]);
    let dim_id = dim_save["id"].as_str().expect("id").to_string();
    home.run_ok(&["search", "embedding size error"]);
    let worked = home.run_json(&["search", "VecDimMismatch error"]);
    let query_id = worked["query_id"].as_str().expect("query_id");
    home.run_ok(&["feedback", query_id, "--used", &dim_id]);

    let mined = home.run_json(&["mine", "--apply"]);
    assert_eq!(mined["applied"].as_bool(), Some(true), "{mined}");
    assert!(
        mined["mappings"]
            .as_array()
            .expect("mappings")
            .iter()
            .any(|m| m["term"] == "embedding"),
        "mine --apply must persist a mapping: {mined}"
    );

    let evaled = home.run_json(&["eval", "--golden", &golden_s, "--golden-only", "--k", "3"]);
    assert!(evaled["recall_at_k"].as_f64().is_some(), "{evaled}");
    assert!(evaled["mrr"].as_f64().is_some(), "{evaled}");
    assert_eq!(evaled["queries"].as_u64(), Some(10), "{evaled}");

    let history = home.run_json(&["eval", "--history"]);
    assert!(
        !history.as_array().expect("history array").is_empty(),
        "eval --history must list the run: {history}"
    );

    let tuned = home.run_json(&["tune", "--golden", &golden_s, "--golden-only"]);
    let ranked = tuned["report"]["ranked"].as_array().expect("ranked");
    assert_eq!(ranked.len(), 1, "one-arm grid: {tuned}");
    assert_eq!(tuned["applied"].as_bool(), Some(false), "{tuned}");

    let bandit = home.run_json(&["bandit", "--golden", &golden_s, "--golden-only"]);
    let report = &bandit["report"];
    assert_eq!(report["golden_pairs"].as_u64(), Some(10), "{bandit}");
    assert!(
        report["ranked"].as_array().is_some_and(|r| !r.is_empty()),
        "bandit happy path must rank at least one arm: {bandit}"
    );
    assert!(report.get("proposed").is_some(), "{bandit}");
}
