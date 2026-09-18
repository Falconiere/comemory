#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory benchmark` driven as a subprocess against a real multi-domain
//! corpus: three memories saved through the binary, one real git repo indexed
//! with `index-code`, and one markdown tree indexed with `index`.
//!
//! The set under test is the shipped, reviewed
//! `tests/common/fixtures/benchmark-mixed-v1.yaml`, so the fixture that ships
//! with the suite is the one the end-to-end behaviour is proved against.

#[path = "common/cli_bin.rs"]
mod cli_bin;
#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use std::path::{Path, PathBuf};

use cli_bin::CliHome;
use serde_json::Value;

/// The shipped reviewed benchmark set.
const SET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/benchmark-mixed-v1.yaml"
);

/// The three fixture memory bodies, in the order the set's ids expect them.
const MEMORIES: [(&str, &str); 3] = [
    (
        "bug",
        "Paged search reorders tied results.\n\nAccess tracking bumps only the returned page, so a gapped bump set floats over the rows it was behind.",
    ),
    (
        "decision",
        "Activation decay must be pinned for measurement.\n\nDisabling access tracking does not freeze activation decay; only a zero decay exponent removes the wall clock.",
    ),
    (
        "note",
        "Document chunking splits on paragraph boundaries.\n\nThe shared splitter keeps a heading breadcrumb for every passage it emits.",
    ),
];

/// A rust source file carrying the symbol the set's code task judges.
const RANKING_RS: &str = "\
/// Map activation to a bounded multiplier with a clamp.
pub fn activation_boost(activation: f64, clamp: (f64, f64)) -> f64 {
    let raw = (0.2 * activation).exp();
    raw.max(clamp.0).min(clamp.1)
}

/// Beta-smoothed feedback posterior mean.
pub fn beta_feedback(used: u64, irrelevant: u64) -> f64 {
    (used as f64 + 1.0) / (used + irrelevant) as f64
}
";

/// The markdown document the set's document task judges.
const CHUNKING_MD: &str = "\
# Chunking

The shared splitter breaks a document on a paragraph boundary and keeps the
heading breadcrumb for every passage it emits, so a chunk cites where it came
from.

## Boundaries

A paragraph boundary is preferred over a hard byte cut.
";

/// Seed the full corpus: memories, a real indexed git repo, an indexed
/// markdown tree. Returns the home and the workspace root.
fn seeded_home() -> (CliHome, PathBuf) {
    let home = CliHome::new();
    let root = home.data_dir().parent().expect("parent").to_path_buf();

    for (kind, body) in MEMORIES {
        home.run_ok(&["save", body, "--kind", kind, "--repo", "demo"]);
    }

    let repo = root.join("demo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(&repo, &[("src/ranking.rs", RANKING_RS)], "ranking");
    home.run_ok(&[
        "index-code",
        "--path",
        repo.to_str().expect("utf8"),
        "--repo",
        "demo",
    ]);

    let docs = root.join("guides");
    std::fs::create_dir_all(&docs).expect("create docs dir");
    std::fs::write(docs.join("chunking.md"), CHUNKING_MD).expect("write doc");
    home.run_ok(&["index", root.join("guides").to_str().expect("utf8")]);

    (home, root)
}

/// Read a written artifact back as JSON.
fn read_artifact(path: &Path) -> Value {
    let raw = std::fs::read_to_string(path).expect("read artifact");
    serde_json::from_str(&raw).expect("artifact is JSON")
}

/// Every task row of a report, by task id.
fn task<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|t| t["task_id"] == id)
        .unwrap_or_else(|| panic!("task {id} missing from {report}"))
}

#[test]
fn benchmark_emits_a_full_observation_artifact_over_every_domain() {
    let (home, root) = seeded_home();
    let artifact_path = root.join("artifact.json");
    let summary = home.run_json(&[
        "benchmark",
        "--set",
        SET,
        "--report",
        artifact_path.to_str().expect("utf8"),
    ]);

    assert_eq!(summary["artifact_version"], 1);
    assert_eq!(summary["observation_version"], 1);
    assert_eq!(summary["set_name"], "comemory-mixed-v1");
    assert_eq!(summary["set_size"]["tasks"], 5);
    assert_eq!(summary["decay_frozen"], true, "the set pins decay to 0.0");
    assert_eq!(summary["retrieval"]["corpus"]["memories"], 3);
    assert!(
        summary["retrieval"]["corpus"]["code_symbols"]
            .as_u64()
            .expect("count")
            > 0,
        "index-code must have produced symbols: {summary}"
    );
    assert!(
        summary["retrieval"]["corpus"]["documents"]
            .as_u64()
            .expect("count")
            > 0,
        "index must have produced documents: {summary}"
    );
    let repos = summary["retrieval"]["corpus"]["repos"]
        .as_array()
        .expect("repos");
    assert_eq!(repos.len(), 1, "{summary}");
    assert_eq!(repos[0]["repo"], "demo");
    assert!(
        repos[0]["last_head"]
            .as_str()
            .is_some_and(|h| !h.is_empty()),
        "the repository revision must be recorded: {summary}"
    );
    assert!(
        summary["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .all(|t| t["observation"]["candidates"]
                .as_array()
                .expect("array")
                .is_empty()),
        "--json prints the summary, with candidates dropped"
    );

    let artifact = read_artifact(&artifact_path);
    let mut domains_seen: Vec<String> = Vec::new();
    for row in artifact["tasks"].as_array().expect("tasks") {
        for candidate in row["observation"]["candidates"]
            .as_array()
            .expect("candidates")
        {
            let domain = candidate["identity"]["domain"].as_str().expect("domain");
            if !domains_seen.iter().any(|d| d == domain) {
                domains_seen.push(domain.to_string());
            }
            assert!(
                candidate["candidate_ref"]
                    .as_str()
                    .expect("ref")
                    .starts_with(domain),
                "every reference is domain-qualified: {candidate}"
            );
            assert_eq!(
                candidate["text"]["sha256"].as_str().expect("digest").len(),
                64,
                "every candidate carries a text hash: {candidate}"
            );
            assert!(candidate["pool_position"].as_u64().expect("pool position") >= 1);
        }
    }
    domains_seen.sort();
    assert_eq!(
        domains_seen,
        vec![
            "code".to_string(),
            "document".to_string(),
            "memory".to_string()
        ],
        "the mixed set must observe candidates from all three corpora"
    );
}

#[test]
fn benchmark_measures_the_pool_separately_from_the_page_and_reports_coverage() {
    let (home, root) = seeded_home();
    let artifact_path = root.join("artifact.json");
    home.run_json(&[
        "benchmark",
        "--set",
        SET,
        "--report",
        artifact_path.to_str().expect("utf8"),
    ]);
    let artifact = read_artifact(&artifact_path);

    let baseline = &artifact["arms"].as_array().expect("arms")[0];
    assert_eq!(baseline["name"], "deterministic");
    assert_eq!(baseline["verdict"], "baseline");
    let overall = &baseline["overall"];
    assert!(
        overall["pool_recall"].as_f64().expect("pool recall")
            >= overall["recall_at_k"].as_f64().expect("recall@k"),
        "the pool is a superset of the page, so its recall cannot be lower: {overall}"
    );
    assert!(overall["judged_tasks"].as_u64().expect("judged") >= 3);
    assert!(
        overall["unjudged_in_page"].as_u64().expect("unjudged") > 0,
        "a mixed corpus surfaces candidates nobody judged; coverage must say so: {overall}"
    );
    assert!(overall["judged_page_fraction"].as_f64().expect("fraction") < 1.0);

    let per_domain = baseline["per_domain"].as_array().expect("per_domain");
    assert_eq!(per_domain.len(), 3, "one block per corpus");

    let mixed = task(&artifact, "mixed-decay-05");
    assert_eq!(
        mixed["observation"]["filters"]["domains"]
            .as_array()
            .expect("domains")
            .len(),
        3,
        "an `all` task records all three legs: {mixed}"
    );
    let memory_only = task(&artifact, "mem-paging-01");
    assert_eq!(
        memory_only["observation"]["filters"]["domains"],
        serde_json::json!(["memory"])
    );
    assert_eq!(memory_only["observation"]["filters"]["repo"], "demo");
    assert_eq!(memory_only["observation"]["filters"]["kind"], "bug");
    assert_eq!(
        memory_only["observation"]["filters"]["lang"],
        Value::Null,
        "lang narrows the code leg only and this task does not run it"
    );
}

#[test]
fn benchmark_compares_two_arms_over_one_candidate_snapshot() {
    let (home, root) = seeded_home();
    let artifact_path = root.join("artifact.json");
    home.run_json(&[
        "benchmark",
        "--set",
        SET,
        "--report",
        artifact_path.to_str().expect("utf8"),
    ]);
    let artifact = read_artifact(&artifact_path);

    // Build an arm from the artifact itself: score every candidate by the
    // reverse of its pool position, so the arm is a deliberate inversion.
    let mut scores = serde_json::Map::new();
    for row in artifact["tasks"].as_array().expect("tasks") {
        let mut per_task = serde_json::Map::new();
        let candidates = row["observation"]["candidates"]
            .as_array()
            .expect("candidates");
        for candidate in candidates {
            let position = candidate["pool_position"].as_u64().expect("position");
            per_task.insert(
                candidate["candidate_ref"]
                    .as_str()
                    .expect("ref")
                    .to_string(),
                serde_json::json!(-(position as f64)),
            );
        }
        scores.insert(
            row["task_id"].as_str().expect("id").to_string(),
            Value::Object(per_task),
        );
    }
    let scores_path = root.join("inverted.json");
    std::fs::write(
        &scores_path,
        serde_json::to_vec(&serde_json::json!({
            "arm": "inverted",
            "scorer_version": "test:inversion@1",
            "scores": Value::Object(scores),
        }))
        .expect("serialize scores"),
    )
    .expect("write scores");

    let compared_path = root.join("compared.json");
    let summary = home.run_json(&[
        "benchmark",
        "--set",
        SET,
        "--scores",
        scores_path.to_str().expect("utf8"),
        "--report",
        compared_path.to_str().expect("utf8"),
    ]);

    let arms = summary["arms"].as_array().expect("arms");
    assert_eq!(arms.len(), 2, "baseline plus the scored arm: {summary}");
    assert_eq!(arms[1]["name"], "inverted");
    assert_eq!(arms[1]["scorer_version"], "test:inversion@1");
    assert_eq!(
        arms[1]["scored_fraction"], 1.0,
        "the arm scored every candidate"
    );
    let delta = &arms[1]["paired_vs_baseline"];
    assert_eq!(delta["metric"], "ndcg_at_k");
    assert_eq!(delta["ci"].as_array().expect("ci").len(), 2);
    assert!(
        delta["mean_delta"].as_f64().expect("delta") <= 0.0,
        "reversing a ranking cannot improve it: {delta}"
    );
    assert_eq!(
        arms[1]["overall"]["pool_recall"], arms[0]["overall"]["pool_recall"],
        "an arm reorders the pool, it cannot change what is in it"
    );

    let compared = read_artifact(&compared_path);
    let first = &compared["tasks"].as_array().expect("tasks")[0];
    let per_arm = first["arms"].as_array().expect("per-arm metrics");
    assert_eq!(per_arm.len(), 2, "every task carries both arms: {first}");
}

#[test]
fn benchmark_writes_no_telemetry_and_leaves_eval_untouched() {
    let (home, _root) = seeded_home();
    let before = home.run_json(&["eval", "--history"]);
    home.run_json(&["benchmark", "--set", SET]);
    home.run_json(&["benchmark", "--set", SET]);
    let after = home.run_json(&["eval", "--history"]);
    assert_eq!(
        before, after,
        "a benchmark run records no eval_runs row and does not disturb eval history"
    );

    let conn = comemory::store::connection::open(home.data_dir().join("comemory.db"))
        .expect("open the same database the runs used");
    let logged: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log");
    assert_eq!(
        logged, 0,
        "two benchmark runs must leave the query log empty: measurement never feeds \
         the signals it measures"
    );
    let memory_bumps: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(access_count), 0) FROM memories",
            [],
            |r| r.get(0),
        )
        .expect("sum memory access_count");
    let code_bumps: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(access_count), 0) FROM code_symbols",
            [],
            |r| r.get(0),
        )
        .expect("sum code access_count");
    assert_eq!(
        (memory_bumps, code_bumps),
        (0, 0),
        "no access counter may move"
    );
}

#[test]
fn a_repeated_run_over_an_unchanged_corpus_scores_identically() {
    let (home, _root) = seeded_home();
    let first = home.run_json(&["benchmark", "--set", SET]);
    let second = home.run_json(&["benchmark", "--set", SET]);
    let arm = |report: &Value| report["arms"].as_array().expect("arms")[0]["overall"].clone();
    assert_eq!(
        arm(&first),
        arm(&second),
        "a decay-frozen run over an unchanged corpus must reproduce exactly"
    );
    assert_eq!(
        first["retrieval"]["corpus"]["digest"],
        second["retrieval"]["corpus"]["digest"]
    );
    assert_eq!(
        first["retrieval"]["knobs_hash"],
        second["retrieval"]["knobs_hash"]
    );
}

#[test]
fn a_supplied_vector_scenario_records_its_model_identity_and_digest() {
    let (home, root) = seeded_home();
    // A deterministic unit-length-ish 1024-dim vector: the memory vec0 table's
    // dimension is baked into the DDL, so a BYO-vector run has to match it.
    let vector: Vec<f64> = (0..1024).map(|i| f64::from(i % 7) / 7.0).collect();
    let csv = vector
        .iter()
        .map(|v| format!("{v:.6}"))
        .collect::<Vec<_>>()
        .join(",");
    home.run_ok(&[
        "save",
        "Vector scenario memory about activation decay and ranking.",
        "--kind",
        "note",
        "--repo",
        "demo",
        "--vector",
        &csv,
    ]);

    let yaml = format!(
        "version: 1\nname: vector-scenario\nvectors:\n  model: test:unit\n  dim: 1024\n\
         ranking:\n  rrf_k: 60.0\n  decay: 0.0\n  mmr_lambda: 0.7\n  \
         bm25_weights: [1.0, 3.0]\n  graph_hops: 2\n  graph_seeds: 8\n\
         defaults:\n  k: 2\n  max_text_bytes: 512\n\
         budgets:\n  min_tasks: 1\n  min_ndcg_gain: 0.02\n  max_ndcg_regression: 0.01\n  \
         max_p95_task_ms: 60000\ntasks:\n  - id: vec-01\n    domain: memory\n    \
         query: activation decay ranking\n    vector: [{}]\n    judgments:\n      \
         - relevance: 3\n        target:\n          domain: memory\n          id: 0568bffe\n",
        vector
            .iter()
            .map(|v| format!("{v:.6}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let set_path = root.join("vector-set.yaml");
    std::fs::write(&set_path, yaml).expect("write vector set");

    let report = home.run_json(&["benchmark", "--set", set_path.to_str().expect("utf8")]);
    let filters = &task(&report, "vec-01")["observation"]["filters"];
    assert_eq!(filters["vector"]["kind"], "supplied");
    assert_eq!(filters["vector"]["model"], "test:unit");
    assert_eq!(filters["vector"]["dim"], 1024);
    assert_eq!(
        filters["vector"]["digest"].as_str().expect("digest").len(),
        64,
        "a replay must be able to prove it used the same numbers"
    );

    // A lexical set over the same corpus must not claim a vector scenario.
    let lexical = home.run_json(&["benchmark", "--set", SET]);
    assert_eq!(
        task(&lexical, "mem-paging-01")["observation"]["filters"]["vector"],
        serde_json::json!({"kind": "lexical"})
    );
}

#[test]
fn a_vector_set_missing_a_task_vector_is_refused_naming_the_task() {
    let (home, root) = seeded_home();
    let yaml = concat!(
        "version: 1\nname: vector-scenario\nvectors:\n  model: test:unit\n  dim: 4\n",
        "ranking:\n  rrf_k: 60.0\n  decay: 0.0\n  mmr_lambda: 0.7\n",
        "  bm25_weights: [1.0, 3.0]\n  graph_hops: 2\n  graph_seeds: 8\n",
        "budgets:\n  min_tasks: 1\n  min_ndcg_gain: 0.02\n  max_ndcg_regression: 0.01\n",
        "  max_p95_task_ms: 60000\ntasks:\n  - id: vec-missing\n    domain: memory\n",
        "    query: activation decay\n    judgments:\n      - relevance: 3\n",
        "        target:\n          domain: memory\n          id: 0568bffe\n",
    );
    let set_path = root.join("missing-vector.yaml");
    std::fs::write(&set_path, yaml).expect("write set");
    let out = home
        .bin()
        .args(["benchmark", "--set", set_path.to_str().expect("utf8")])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(78), "EX_CONFIG");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("vec-missing"),
        "must name the task: {stderr}"
    );
    assert!(stderr.contains("must supply one"), "{stderr}");
}

#[test]
fn a_k_override_replaces_the_set_default_cut() {
    let (home, _root) = seeded_home();
    let default_cut = home.run_json(&["benchmark", "--set", SET]);
    assert_eq!(
        default_cut["k"], 3,
        "the shipped set declares defaults.k: 3"
    );

    let narrowed = home.run_json(&["benchmark", "--set", SET, "--k", "1"]);
    assert_eq!(narrowed["k"], 1);
    assert_eq!(
        task(&narrowed, "mem-decay-02")["observation"]["page_limit"],
        1,
        "the override reaches the observation's own page record"
    );
    let wide = default_cut["arms"].as_array().expect("arms")[0]["overall"].clone();
    let tight = narrowed["arms"].as_array().expect("arms")[0]["overall"].clone();
    assert_eq!(
        wide["pool_recall"], tight["pool_recall"],
        "the cut moves the page, never the pool"
    );
    assert!(
        tight["recall_at_k"].as_f64().expect("recall")
            <= wide["recall_at_k"].as_f64().expect("recall"),
        "a narrower cut cannot find more: {tight} vs {wide}"
    );

    let zero = home
        .bin()
        .args(["benchmark", "--set", SET, "--k", "0"])
        .output()
        .expect("run");
    assert_eq!(zero.status.code(), Some(2), "clap refuses a zero cut");
}

#[test]
fn a_malformed_set_exits_with_the_config_code_and_names_the_file() {
    let (home, root) = seeded_home();
    let bad = root.join("bad.yaml");
    std::fs::write(&bad, "version: 9\nname: x\n").expect("write");
    let out = home
        .bin()
        .args(["benchmark", "--set", bad.to_str().expect("utf8")])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(78), "EX_CONFIG");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("bad.yaml"), "{stderr}");

    let missing = home
        .bin()
        .args(["benchmark", "--set", "/nonexistent/set.yaml"])
        .output()
        .expect("run");
    assert_eq!(missing.status.code(), Some(78));
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("/nonexistent/set.yaml"),
        "a missing set file must name the path"
    );
}

#[test]
fn the_tty_summary_states_the_verdict_the_budget_and_the_decay_pin() {
    let (home, _root) = seeded_home();
    let stdout = home.run_ok(&["benchmark", "--set", SET]);
    assert!(stdout.contains("comemory-mixed-v1 v1"), "{stdout}");
    assert!(stdout.contains("decay frozen"), "{stdout}");
    assert!(stdout.contains("deterministic [Baseline]"), "{stdout}");
    assert!(stdout.contains("pool_recall"), "{stdout}");
    assert!(stdout.contains("within budget"), "{stdout}");
}
