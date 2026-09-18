#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Coverage for the capture step against a real corpus: the pool retrieval
//! produced, the page a user would have seen, and what the reviewed judgments
//! matched. Real SQLite, real saves, real retrieval legs.

use comemory::config::Config;
use comemory::domains::learning::evaluation::benchmark_runner::{self, RunContext};
use comemory::domains::learning::evaluation::benchmark_set::{BenchmarkSet, BenchmarkTask};
use comemory::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity,
};
use comemory::domains::learning::evaluation::run_environment;
use comemory::domains::retrieval::scope::{Domain, Filters};
use comemory::domains::retrieval::unified::{self, DomainFilters};
use comemory::store::{Connection, connection};
use comemory::utilities::pagination::PageWindow;

use crate::test_common::benchmark_corpus::{MEMORY_IDS, REPO, seeded_home};

/// A memory-only set whose single task carries `judgments`, at cut `k`.
fn set_yaml(k: usize, judgments: &str) -> String {
    format!(
        "version: 1\nname: runner-fixture\nranking:\n  rrf_k: 60.0\n  decay: 0.0\n  \
         mmr_lambda: 0.7\n  bm25_weights: [1.0, 3.0]\n  graph_hops: 2\n  graph_seeds: 8\n\
         defaults:\n  k: {k}\n  max_text_bytes: 4096\n\
         budgets:\n  min_tasks: 1\n  min_ndcg_gain: 0.02\n  max_ndcg_regression: 0.01\n  \
         max_p95_task_ms: 60000\ntasks:\n  - id: t1\n    domain: memory\n    \
         query: activation decay paged search\n    filters:\n      repo: {REPO}\n    \
         judgments:\n{judgments}"
    )
}

/// Parse `yaml` into a set without touching the filesystem.
fn parse(yaml: &str) -> BenchmarkSet {
    serde_yaml::from_str(yaml).expect("set yaml")
}

/// Capture the set's first task against `conn`.
fn capture_first(
    cfg: &Config,
    conn: &Connection,
    set: &BenchmarkSet,
) -> comemory::domains::learning::evaluation::benchmark_runner::TaskCapture {
    let version = run_environment::retrieval_version(cfg, conn).expect("version");
    let task: &BenchmarkTask = set.tasks.first().expect("one task");
    benchmark_runner::capture(
        cfg,
        conn,
        set,
        task,
        &RunContext {
            k: set.defaults.k,
            version: &version,
            reference_time: "2026-09-18T00:00:00Z".to_string(),
        },
    )
    .expect("capture")
}

/// A judgment block naming `id` at `relevance`.
fn memory_judgment(relevance: u8, id: &str) -> String {
    format!(
        "      - relevance: {relevance}\n        target:\n          domain: memory\n          \
         id: {id}\n"
    )
}

#[test]
fn a_relevant_memory_below_the_cut_is_in_the_pool_but_not_on_the_page() {
    let (_tmp, paths, base) = seeded_home();
    let judgments = format!(
        "{}{}",
        memory_judgment(3, MEMORY_IDS[0]),
        memory_judgment(3, MEMORY_IDS[1])
    );
    let set = parse(&set_yaml(1, &judgments));
    let cfg = set.effective_config(&base).expect("effective config");
    let conn = connection::open(paths.db_path()).expect("open db");
    let capture = capture_first(&cfg, &conn, &set);

    assert!(
        capture.observation.candidates.len() >= 2,
        "both judged memories must be in the pool, got {}",
        capture.observation.candidates.len()
    );
    let page: Vec<_> = capture
        .observation
        .candidates
        .iter()
        .filter(|c| c.returned_position.is_some())
        .collect();
    assert_eq!(
        page.len(),
        1,
        "k = 1, so exactly one candidate is on the page"
    );
    assert_eq!(capture.matched.len(), 2);
    assert!(
        capture.matched.iter().all(|j| !j.pool_positions.is_empty()),
        "pool recall is full: retrieval produced both"
    );
    let on_page = capture
        .matched
        .iter()
        .filter(|j| j.pool_positions.contains(&1))
        .count();
    assert_eq!(
        on_page, 1,
        "only one of the two can occupy a one-result page"
    );
}

#[test]
fn a_judgment_matching_nothing_is_reported_without_growing_the_pool() {
    let (_tmp, paths, base) = seeded_home();
    let present = parse(&set_yaml(3, &memory_judgment(3, MEMORY_IDS[0])));
    let with_ghost = parse(&set_yaml(
        3,
        &format!(
            "{}{}",
            memory_judgment(3, MEMORY_IDS[0]),
            memory_judgment(3, "deadbeef")
        ),
    ));
    let cfg = present.effective_config(&base).expect("effective config");
    let conn = connection::open(paths.db_path()).expect("open db");

    let baseline = capture_first(&cfg, &conn, &present);
    let ghosted = capture_first(&cfg, &conn, &with_ghost);
    assert_eq!(
        baseline.observation.candidates.len(),
        ghosted.observation.candidates.len(),
        "a missing positive must never be inserted into the candidate pool"
    );
    assert_eq!(
        ghosted.unmatched_targets,
        vec!["memory:deadbeef".to_string()]
    );
    assert_eq!(ghosted.stale, 0, "an absent id is unmatched, not stale");
    assert_eq!(
        ghosted.matched.len(),
        2,
        "both judgments stay in the denominator"
    );
}

#[test]
fn a_stale_content_pin_is_excluded_and_counted_rather_than_scored_as_a_hit() {
    let (_tmp, paths, base) = seeded_home();
    let pinned = format!(
        "      - relevance: 3\n        target:\n          domain: memory\n          id: {}\n          \
         content_hash: {}\n",
        MEMORY_IDS[0],
        "0".repeat(64)
    );
    let set = parse(&set_yaml(3, &pinned));
    let cfg = set.effective_config(&base).expect("effective config");
    let conn = connection::open(paths.db_path()).expect("open db");
    let capture = capture_first(&cfg, &conn, &set);

    assert_eq!(capture.stale, 1, "the body moved under the judgment");
    assert!(
        capture.matched.is_empty(),
        "a stale judgment scores nothing at all"
    );
    assert!(
        capture.unmatched_targets.is_empty(),
        "stale is not unmatched"
    );
    assert!(
        !capture.observation.candidates.is_empty(),
        "the candidate itself is still observed"
    );
}

#[test]
fn the_captured_pool_prefix_is_the_page_find_returns() {
    let (_tmp, paths, base) = seeded_home();
    let set = parse(&set_yaml(2, &memory_judgment(3, MEMORY_IDS[0])));
    let cfg = set.effective_config(&base).expect("effective config");
    let conn = connection::open(paths.db_path()).expect("open db");
    let capture = capture_first(&cfg, &conn, &set);

    let scope = comemory::domains::retrieval::scope::TimeScope::none();
    let run = unified::find(
        &cfg,
        &conn,
        unified::UnifiedQuery {
            text: "activation decay paged search",
            vector: None,
            filters: Filters {
                repo: Some(REPO),
                kind: None,
                scope: &scope,
                domains: comemory::domains::retrieval::scope::Domains::of(&[Domain::Memory]),
            },
            domain_filters: DomainFilters::default(),
        },
        PageWindow {
            offset: 0,
            limit: 2,
        },
    )
    .expect("find");

    let observed: Vec<&str> = capture
        .observation
        .candidates
        .iter()
        .take(2)
        .map(|c| c.locator.title.as_str())
        .collect();
    let produced: Vec<&str> = run.hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(
        observed, produced,
        "the benchmark must measure the shipped ranking, not a second composition"
    );
    assert!(
        capture.page_matches_production,
        "and the capture must say so itself"
    );
}

#[test]
fn the_filter_record_names_every_dimension_and_the_lexical_scenario() {
    let (_tmp, paths, base) = seeded_home();
    let set = parse(&set_yaml(3, &memory_judgment(3, MEMORY_IDS[0])));
    let cfg = set.effective_config(&base).expect("effective config");
    let conn = connection::open(paths.db_path()).expect("open db");
    let filters = capture_first(&cfg, &conn, &set).observation.filters;

    assert_eq!(filters.domains, vec!["memory".to_string()]);
    assert_eq!(filters.repo.as_deref(), Some(REPO));
    assert_eq!(filters.kind, None);
    assert_eq!(
        filters.lang, None,
        "lang narrows the code leg, which is out of scope"
    );
    assert!(filters.path_globs.is_empty());
    assert_eq!(filters.since, None);
    assert_eq!(filters.until, None);
    assert_eq!(filters.as_of, None);
    assert_eq!(
        serde_json::to_value(&filters.vector).expect("serialize"),
        serde_json::json!({"kind": "lexical"}),
        "a set with no vectors model is the lexical scenario"
    );
}

#[test]
fn every_observed_memory_carries_its_body_digest_as_the_content_version() {
    let (_tmp, paths, base) = seeded_home();
    let set = parse(&set_yaml(3, &memory_judgment(3, MEMORY_IDS[0])));
    let cfg = set.effective_config(&base).expect("effective config");
    let conn = connection::open(paths.db_path()).expect("open db");
    let capture = capture_first(&cfg, &conn, &set);

    assert!(
        !capture.observation.candidates.is_empty(),
        "the query must produce candidates, or the loop below asserts nothing"
    );
    for candidate in &capture.observation.candidates {
        assert_eq!(candidate.identity.domain(), CandidateDomain::Memory);
        let CandidateIdentity::Memory(identity) = &candidate.identity else {
            panic!("a memory-only run cannot observe {:?}", candidate.identity);
        };
        assert_eq!(
            identity.content_hash.len(),
            64,
            "the content version is a full sha256"
        );
        // Read the column by the memory's own id — not by a locator field —
        // and fail hard when the row is missing. A fallback to the observed
        // digest would make the comparison below compare a value with itself.
        let stored: String = conn
            .query_row(
                "SELECT content_hash FROM memories WHERE id = ?1",
                [identity.memory_id.as_str()],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "memory {} must still be in the corpus: {e}",
                    identity.memory_id
                )
            });
        assert_eq!(
            stored, identity.content_hash,
            "the observed digest must equal memories.content_hash for {}",
            identity.memory_id
        );
        assert_eq!(candidate.text.sha256.len(), 64);
        assert!(
            candidate.tier.is_some(),
            "a memory candidate carries its ladder tier"
        );
    }
    assert_eq!(capture.text_unavailable, 0);
}
