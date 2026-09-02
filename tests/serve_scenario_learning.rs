#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Learning-loop journey over `/api/v1` — the HTTP twin of
//! `tests/cli_scenario_learning.rs`: save → search → feedback → mine → eval
//! → tune → bandit, against a real `comemory serve`.
//!
//! `config.toml`'s one-arm `[tune]` grid is seeded into `<root>/.comemory/`
//! BEFORE the server starts (`ServeHome::spawn_in`), since the server loads
//! config at startup — the cartesian product must stay at 1, same as the
//! CLI journey's `write_one_arm_grid`.

#[path = "common/serve_bin.rs"]
mod serve_bin;

use std::fmt::Write as _;

use serde_json::json;
use serve_bin::ServeHome;
use tempfile::TempDir;

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

#[test]
fn search_feedback_mine_eval_tune_bandit_over_http() {
    let root = TempDir::new().expect("root");
    let root_path = root.path().to_str().expect("utf8").to_string();
    let data_dir = root.path().join(".comemory");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::write(data_dir.join("config.toml"), ONE_ARM_GRID).expect("write config.toml");
    let srv = ServeHome::spawn_in(root, &["--allow-path", &root_path], &[]);

    let mut yaml = String::new();
    for topic in TOPICS {
        let saved = srv.post("/memories", &json!({ "body": topic, "kind": "note" }));
        let id = saved["id"].as_str().expect("save id").to_string();
        let _ = writeln!(yaml, "- query: {topic}\n  relevant: [{id}]");
    }
    let golden = srv.workspace().join("golden.yaml");
    std::fs::write(&golden, yaml).expect("write golden");
    let golden_s = golden.to_str().expect("utf8 golden path").to_string();

    let dim_saved = srv.post(
        "/memories",
        &json!({
            "body": "VecDimMismatch error raised by the dim guard",
            "kind": "bug",
        }),
    );
    let dim_id = dim_saved["id"].as_str().expect("save id").to_string();

    // A miss first (no reformulation partner yet), then the query that
    // actually finds the bug memory — the pair `mine` distills a mapping
    // from.
    let _miss = srv.get_q("/memories/search", &[("query", "embedding size error")]);
    let worked = srv.get_q("/memories/search", &[("query", "VecDimMismatch error")]);
    let query_id = worked["query_id"].as_str().expect("query_id").to_string();
    srv.post(
        "/feedback",
        &json!({ "query_id": query_id, "used": [dim_id] }),
    );

    let mined = srv.post("/mine", &json!({ "apply": true }));
    assert_eq!(mined["applied"].as_bool(), Some(true), "{mined}");
    assert!(
        mined["mappings"]
            .as_array()
            .expect("mappings")
            .iter()
            .any(|m| m["term"] == "embedding"),
        "mine --apply must persist a mapping: {mined}"
    );

    let evaled = srv.job(
        "/eval",
        &json!({ "golden": golden_s, "golden_only": true, "k": 3 }),
    );
    assert!(evaled["recall_at_k"].as_f64().is_some(), "{evaled}");
    assert!(evaled["mrr"].as_f64().is_some(), "{evaled}");
    assert_eq!(evaled["queries"].as_u64(), Some(10), "{evaled}");

    let history = srv.get("/eval/history");
    assert!(
        !history.as_array().expect("history array").is_empty(),
        "GET /eval/history must list the run: {history}"
    );

    let tuned = srv.job("/tune", &json!({ "golden": golden_s, "golden_only": true }));
    let ranked = tuned["report"]["ranked"].as_array().expect("ranked");
    assert_eq!(ranked.len(), 1, "one-arm grid: {tuned}");
    assert_eq!(tuned["applied"].as_bool(), Some(false), "{tuned}");

    // `api::bandit::run` returns `BanditReport` directly (unlike the CLI's
    // `--json` output, which wraps it as `{"report": ...}`) — the job
    // result's fields sit at the top level, not nested under `report`.
    let bandit = srv.job(
        "/bandit",
        &json!({ "golden": golden_s, "golden_only": true }),
    );
    assert_eq!(bandit["golden_pairs"].as_u64(), Some(10), "{bandit}");
    assert!(
        bandit["ranked"].as_array().is_some_and(|r| !r.is_empty()),
        "bandit happy path must rank at least one arm: {bandit}"
    );
    assert!(bandit.get("proposed").is_some(), "{bandit}");
}
