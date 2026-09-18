#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the versioned benchmark set: the shipped reviewed fixture must
//! load, and every shape that would silently score something other than what
//! the file says must fail to load naming the task.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use comemory::config::Config;
use comemory::domains::learning::evaluation::benchmark_set::{
    BenchmarkSet, SET_VERSION, TaskDomain,
};
use tempfile::TempDir;

/// The reviewed fixture set shipped with the suite.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/common/fixtures/benchmark-mixed-v1.yaml"
);

/// Write `yaml` into a tempdir and return its path plus the guard.
fn write_set(yaml: &str) -> (PathBuf, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("set.yaml");
    let mut file = std::fs::File::create(&path).expect("create set file");
    file.write_all(yaml.as_bytes()).expect("write set file");
    (path, tmp)
}

/// The shipped fixture's text, with `find` replaced by `replace` once.
fn fixture_with(find: &str, replace: &str) -> String {
    let raw = std::fs::read_to_string(FIXTURE).expect("read fixture");
    assert!(raw.contains(find), "fixture must contain {find:?}");
    raw.replacen(find, replace, 1)
}

/// Load a mutated fixture and return the error message it must produce.
fn load_err(find: &str, replace: &str) -> String {
    let (path, _tmp) = write_set(&fixture_with(find, replace));
    BenchmarkSet::load(&path)
        .expect_err("a malformed set must not load")
        .to_string()
}

#[test]
fn the_shipped_reviewed_fixture_loads_and_covers_every_domain() {
    let set = BenchmarkSet::load(Path::new(FIXTURE)).expect("the shipped fixture must load");
    assert_eq!(set.version, SET_VERSION);
    assert_eq!(set.name, "comemory-mixed-v1");
    assert_eq!(set.defaults.k, 3);
    assert_eq!(set.defaults.max_text_bytes, 2048);
    assert!(
        set.vectors.is_none(),
        "the shipped set is the lexical scenario"
    );
    assert_eq!(
        set.ranking.decay, 0.0,
        "the shipped set pins decay so runs are reproducible across days"
    );
    let domains: Vec<TaskDomain> = set.tasks.iter().map(|t| t.domain).collect();
    for wanted in [
        TaskDomain::Memory,
        TaskDomain::Code,
        TaskDomain::Document,
        TaskDomain::All,
    ] {
        assert!(domains.contains(&wanted), "{wanted:?} must be covered");
    }
    for task in &set.tasks {
        task.target_keys()
            .expect("every judgment target must resolve");
    }
}

#[test]
fn the_pinned_ranking_becomes_the_effective_config() {
    let set = BenchmarkSet::load(Path::new(FIXTURE)).expect("load");
    let base = Config::defaults();
    assert_eq!(base.rank.decay, 0.5, "the live default is not frozen");
    let effective = set
        .effective_config(&base)
        .expect("pinned ranking validates");
    assert_eq!(effective.rank.decay, 0.0, "the set's pin wins");
    assert_eq!(effective.retrieval.rrf_k, 60.0);
}

#[test]
fn an_out_of_range_ranking_knob_is_reported_against_the_set_not_as_a_bad_request() {
    // The bound belongs to `Config::validate`, so it is enforced when the set's
    // pinned knobs become the effective config — and the BadRequest that
    // validation raises is converted, because the fault is in the file.
    let (path, _tmp) = write_set(&fixture_with("  mmr_lambda: 0.7", "  mmr_lambda: 9.9"));
    let set = BenchmarkSet::load(&path).expect("the file itself is well formed");
    let msg = set
        .effective_config(&Config::defaults())
        .expect_err("an out-of-range knob must not be scored")
        .to_string();
    assert!(
        msg.starts_with("config:"),
        "must be a config fault, not a bad request: {msg}"
    );
    assert!(
        msg.contains("comemory-mixed-v1"),
        "must name the set: {msg}"
    );
}

#[test]
fn an_unsupported_version_is_refused_by_number() {
    let msg = load_err("version: 1", "version: 2");
    assert!(msg.contains("version 2 is not supported"), "{msg}");
}

#[test]
fn a_duplicate_task_id_is_refused_because_ids_key_the_scores_files() {
    let msg = load_err("  - id: code-ranking-03", "  - id: mem-paging-01");
    assert!(msg.contains("duplicate task id `mem-paging-01`"), "{msg}");
}

#[test]
fn a_relevance_above_the_grade_ceiling_is_refused() {
    let msg = load_err("      - relevance: 2\n", "      - relevance: 7\n");
    assert!(msg.contains("relevance 7"), "{msg}");
    assert!(msg.contains("grade ceiling"), "{msg}");
}

#[test]
fn a_filter_whose_leg_the_task_does_not_run_is_refused_naming_the_task() {
    // `kind` narrows the memory leg only, and this task is code-only.
    let msg = load_err(
        "    filters:\n      repo: demo\n      lang: rust",
        "    filters:\n      repo: demo\n      lang: rust\n      kind: decision",
    );
    assert!(msg.contains("code-ranking-03"), "must name the task: {msg}");
    assert!(
        msg.contains("`kind`") && msg.contains("memory leg only"),
        "{msg}"
    );

    // `lang` narrows the code leg only, and this task is memory-only.
    let msg = load_err(
        "      repo: demo\n      kind: bug",
        "      repo: demo\n      kind: bug\n      lang: rust",
    );
    assert!(
        msg.contains("mem-paging-01") && msg.contains("`lang`"),
        "{msg}"
    );

    // A time bound narrows the memory leg only, and this task is document-only.
    let msg = load_err(
        "    filters:\n      path:\n        - \"**/*.md\"",
        "    filters:\n      since: 2026-01-01\n      path:\n        - \"**/*.md\"",
    );
    assert!(
        msg.contains("doc-chunking-04") && msg.contains("`since`"),
        "{msg}"
    );
}

#[test]
fn a_judgment_for_a_domain_the_task_does_not_run_is_refused() {
    let msg = load_err(
        "        target:\n          domain: code\n          repo: demo\n          path: src/ranking.rs\n          symbol: activation_boost\n\n  - id: doc-chunking-04",
        "        target:\n          domain: memory\n          id: 3f116117\n\n  - id: doc-chunking-04",
    );
    assert!(msg.contains("code-ranking-03"), "{msg}");
    assert!(msg.contains("cannot match under `domain: code`"), "{msg}");
}

#[test]
fn a_task_without_judgments_carries_no_signal_and_is_refused() {
    let msg = load_err(
        "    judgments:\n      - relevance: 3\n        target:\n          domain: memory\n          id: 3f116117\n",
        "    judgments: []\n",
    );
    assert!(
        msg.contains("mem-paging-01") && msg.contains("no judgments"),
        "{msg}"
    );
}

#[test]
fn absent_budgets_are_refused_because_they_must_predate_the_score() {
    let raw = std::fs::read_to_string(FIXTURE).expect("read fixture");
    let start = raw.find("budgets:").expect("budgets block");
    let end = raw.find("\ntasks:").expect("tasks block") + 1;
    let (path, _tmp) = write_set(&format!("{}{}", &raw[..start], &raw[end..]));
    let msg = BenchmarkSet::load(&path)
        .expect_err("budgets are required")
        .to_string();
    assert!(msg.contains("budgets"), "{msg}");
}

#[test]
fn a_zero_min_tasks_budget_is_refused() {
    let msg = load_err("  min_tasks: 3", "  min_tasks: 0");
    assert!(
        msg.contains("budgets.min_tasks must be at least 1"),
        "{msg}"
    );
}

#[test]
fn a_vector_without_a_declared_model_is_refused_as_a_mixed_scenario() {
    let msg = load_err(
        "    query: paged search tied results",
        "    query: paged search tied results\n    vector: [0.1, 0.2]",
    );
    assert!(msg.contains("mem-paging-01"), "{msg}");
    assert!(msg.contains("separate scenarios"), "{msg}");
}

#[test]
fn a_declared_model_requires_every_task_to_supply_a_matching_vector() {
    let yaml = fixture_with(
        "defaults:",
        "vectors:\n  model: test:unit\n  dim: 4\n\ndefaults:",
    );
    let (path, _tmp) = write_set(&yaml);
    let msg = BenchmarkSet::load(&path)
        .expect_err("a BYO-vector set needs vectors")
        .to_string();
    assert!(
        msg.contains("mem-paging-01") && msg.contains("must supply one"),
        "{msg}"
    );

    let yaml = yaml.replacen(
        "    query: paged search tied results",
        "    query: paged search tied results\n    vector: [0.1, 0.2]",
        1,
    );
    let (path, _tmp2) = write_set(&yaml);
    let msg = BenchmarkSet::load(&path)
        .expect_err("a wrong-length vector is not the declared model's")
        .to_string();
    assert!(
        msg.contains("2 values") && msg.contains("dim is 4"),
        "{msg}"
    );
}

#[test]
fn a_missing_file_names_the_path() {
    let msg = BenchmarkSet::load(Path::new("/nonexistent/benchmark-set.yaml"))
        .expect_err("a missing set file must not be an empty set")
        .to_string();
    assert!(msg.contains("/nonexistent/benchmark-set.yaml"), "{msg}");
}

#[test]
fn an_unknown_top_level_key_is_refused_rather_than_ignored() {
    let msg = load_err(
        "name: comemory-mixed-v1",
        "name: comemory-mixed-v1\nbudget: 3",
    );
    assert!(msg.contains("budget"), "{msg}");
}
