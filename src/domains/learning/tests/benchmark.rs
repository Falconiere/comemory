#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage for the `comemory benchmark` core against a real
//! corpus: memories written through the production save path, a real SQLite
//! database, and the real retrieval legs. Nothing here is mocked.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use comemory::config::Config;
use comemory::config::paths::Paths;
use comemory::domains::learning::benchmark::{self, Request};
use comemory::domains::learning::evaluation::benchmark_arm_report::Verdict;
use comemory::store::connection;
use comemory::utilities::context::Ctx;

use crate::test_common::benchmark_corpus::seeded_home;

/// Write `text` into `dir` and hand back the path.
fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(text.as_bytes()).expect("write");
    path
}

/// A memory-only set over the seeded corpus, with `budgets.min_tasks` chosen by
/// the caller so the inconclusive rule can be exercised.
fn memory_set(min_tasks: usize) -> String {
    format!(
        "version: 1\nname: core-memory\nranking:\n  rrf_k: 60.0\n  decay: 0.0\n  \
         mmr_lambda: 0.7\n  bm25_weights: [1.0, 3.0]\n  graph_hops: 2\n  graph_seeds: 8\n\
         defaults:\n  k: 1\n  max_text_bytes: 64\n\
         budgets:\n  min_tasks: {min_tasks}\n  min_ndcg_gain: 0.02\n  \
         max_ndcg_regression: 0.01\n  max_p95_task_ms: 60000\ntasks:\n\
         \x20 - id: t-paging\n    domain: memory\n    query: paged search tied results\n    \
         judgments:\n      - relevance: 3\n        target:\n          domain: memory\n          \
         id: 3f116117\n\
         \x20 - id: t-decay\n    domain: memory\n    query: activation decay wall clock\n    \
         judgments:\n      - relevance: 3\n        target:\n          domain: memory\n          \
         id: 0568bffe\n"
    )
}

/// Run the core against the seeded corpus with the given set text.
fn run_set(
    paths: &Paths,
    cfg: &Config,
    set_text: &str,
    dir: &Path,
) -> comemory::prelude::Result<
    comemory::domains::learning::evaluation::benchmark_report::BenchmarkReport,
> {
    let set_path = write(dir, "set.yaml", set_text);
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let mut ctx = Ctx::borrowed(paths, cfg, &mut conn);
    benchmark::run(
        &mut ctx,
        &Request {
            set: set_path,
            scores: Vec::new(),
            k: None,
        },
    )
}

#[test]
fn the_fixture_bodies_still_hash_to_the_ids_the_shipped_set_names() {
    let (tmp, paths, cfg) = seeded_home();
    let report = run_set(&paths, &cfg, &memory_set(1), tmp.path()).expect("run");
    let matched: usize = report.tasks.iter().map(|t| t.judgments_matched).sum();
    assert_eq!(
        matched,
        2,
        "both content-derived ids must still resolve; if this fails the fixture \
         bodies moved and tests/common/fixtures/benchmark-mixed-v1.yaml needs its \
         ids recomputed. Unmatched: {:?}",
        report
            .tasks
            .iter()
            .flat_map(|t| t.unmatched_targets.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_run_records_the_contract_on_every_candidate() {
    let (tmp, paths, cfg) = seeded_home();
    let report = run_set(&paths, &cfg, &memory_set(1), tmp.path()).expect("run");
    assert_eq!(report.artifact_version, 1);
    assert_eq!(report.observation_version, 1);
    assert_eq!(report.set_size.tasks, 2);
    assert_eq!(report.set_size.judgments, 2);
    assert!(report.decay_frozen, "the set pinned decay to 0.0");
    assert_eq!(report.retrieval.knobs.decay, 0.0);
    assert_eq!(report.retrieval.knobs_hash.len(), 64);
    assert_eq!(report.retrieval.corpus.memories, 3);
    assert_eq!(report.retrieval.corpus.digest.len(), 64);
    assert!(!report.reference_time.is_empty());

    let task = report.tasks.first().expect("a task row");
    assert!(
        !task.observation.candidates.is_empty(),
        "the pool is non-empty"
    );
    assert_eq!(task.observation.page_limit, 1);
    assert_eq!(task.observation.page_offset, 0);
    assert_eq!(task.observation.filters.domains, vec!["memory".to_string()]);
    for (index, candidate) in task.observation.candidates.iter().enumerate() {
        assert_eq!(
            candidate.pool_position,
            index + 1,
            "pool order is the capture order"
        );
        assert_eq!(
            candidate.returned_position,
            (index == 0).then_some(1),
            "only the first candidate is on a k=1 page"
        );
        assert_eq!(candidate.text.sha256.len(), 64);
        assert!(
            candidate.text.text.len() <= 64,
            "the set's byte bound holds"
        );
        assert!(
            candidate.text.truncated,
            "the fixture bodies exceed 64 bytes"
        );
        assert_eq!(
            comemory::domains::learning::evaluation::candidate_identity::parse_ref(
                &candidate.candidate_ref
            )
            .expect("every emitted ref must parse"),
            candidate.identity
        );
    }
}

#[test]
fn a_run_writes_no_telemetry_and_repeats_exactly() {
    let (tmp, paths, cfg) = seeded_home();
    let first = run_set(&paths, &cfg, &memory_set(1), tmp.path()).expect("first run");
    let second = run_set(&paths, &cfg, &memory_set(1), tmp.path()).expect("second run");

    let conn = connection::open(paths.db_path()).expect("open db");
    let logged: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log");
    assert_eq!(
        logged, 0,
        "measurement must not feed the signals it measures"
    );
    let bumps: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(access_count), 0) FROM memories",
            [],
            |r| r.get(0),
        )
        .expect("sum access_count");
    assert_eq!(bumps, 0, "no access counter may move during a benchmark");

    let a = first.arms.first().expect("baseline arm");
    let b = second.arms.first().expect("baseline arm");
    assert_eq!(a.overall.recall_at_k, b.overall.recall_at_k);
    assert_eq!(a.overall.mrr, b.overall.mrr);
    assert_eq!(a.overall.ndcg_at_k, b.overall.ndcg_at_k);
    assert_eq!(a.overall.pool_recall, b.overall.pool_recall);
    assert_eq!(a.overall.recall_at_k_ci, b.overall.recall_at_k_ci);
}

#[test]
fn too_few_judged_tasks_is_inconclusive_whatever_the_numbers_say() {
    let (tmp, paths, cfg) = seeded_home();
    let report = run_set(&paths, &cfg, &memory_set(99), tmp.path()).expect("run");
    let baseline = report.arms.first().expect("baseline arm");
    assert!(baseline.overall.judged_tasks < report.budgets.min_tasks);
    assert_eq!(
        baseline.verdict,
        Verdict::Inconclusive,
        "insufficient data outranks a favourable point estimate"
    );
}

#[test]
fn the_environment_block_records_size_latency_hardware_and_memory() {
    let (tmp, paths, cfg) = seeded_home();
    let report = run_set(&paths, &cfg, &memory_set(1), tmp.path()).expect("run");
    let env = &report.environment;
    assert!(!env.os.is_empty() && !env.arch.is_empty());
    assert!(
        env.observation_bytes > 0,
        "candidate text was held and measured"
    );
    assert!(env.peak_task_observation_bytes <= env.observation_bytes);
    let baseline = report.arms.first().expect("baseline arm");
    assert!(baseline.latency_ms.p50 <= baseline.latency_ms.p95);
    assert!(baseline.latency_ms.p95 <= baseline.latency_ms.max);
    assert!(
        baseline.latency_within_budget,
        "60s is ample for a 3-row corpus"
    );
}

#[test]
fn the_summary_drops_candidates_and_keeps_every_other_field() {
    let (tmp, paths, cfg) = seeded_home();
    let report = run_set(&paths, &cfg, &memory_set(1), tmp.path()).expect("run");
    let summary = report.summary();
    assert!(
        summary
            .tasks
            .iter()
            .all(|t| t.observation.candidates.is_empty())
    );
    assert!(
        report
            .tasks
            .iter()
            .any(|t| !t.observation.candidates.is_empty())
    );
    assert_eq!(summary.arms.len(), report.arms.len());
    assert_eq!(summary.retrieval.knobs_hash, report.retrieval.knobs_hash);
    assert_eq!(summary.set_size.judgments, report.set_size.judgments);
}

#[test]
fn a_zero_k_override_is_a_usage_error_rather_than_an_empty_page() {
    let (tmp, paths, cfg) = seeded_home();
    let set_path = write(tmp.path(), "set.yaml", &memory_set(1));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = benchmark::run(
        &mut ctx,
        &Request {
            set: set_path,
            scores: Vec::new(),
            k: Some(0),
        },
    )
    .expect_err("k=0 must be refused");
    assert!(err.to_string().contains("--k must be at least 1"), "{err}");
}

#[test]
fn two_scores_files_declaring_one_arm_name_are_refused() {
    let (tmp, paths, cfg) = seeded_home();
    let set_path = write(tmp.path(), "set.yaml", &memory_set(1));
    let a = write(tmp.path(), "a.json", r#"{"arm":"dup","scores":{}}"#);
    let b = write(tmp.path(), "b.json", r#"{"arm":"dup","scores":{}}"#);
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = benchmark::run(
        &mut ctx,
        &Request {
            set: set_path,
            scores: vec![a, b],
            k: None,
        },
    )
    .expect_err("a duplicate arm name must be refused");
    assert!(err.to_string().contains("already declared"), "{err}");
}
