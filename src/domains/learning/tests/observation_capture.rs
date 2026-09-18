#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/observation_capture.rs`, over the
//! real memories the shared benchmark fixture saves through
//! `domains::memories::save` and the real retrieval legs.
//!
//! The private bounds (`candidate_cut`, `embed_model`) are reached through
//! `super::*`: they decide what a captured observation contains and are not
//! observable from the public surface any other way.

use comemory::config::Config;
use comemory::domains::learning::evaluation::candidate_facts;
use comemory::domains::learning::observation_capture::{
    CaptureInput, armed, capture, find_filters,
};
use comemory::domains::retrieval::scope::{self, Domain, Domains, Filters};
use comemory::domains::retrieval::unified::{self, DomainFilters, UnifiedQuery};
use comemory::store::candidate_observations::{StoredCandidate, StoredObservation, fetch};
use comemory::store::connection;
use comemory::utilities::pagination::PageWindow;

use super::super::observation_capture::{candidate_cut, embed_model};
use crate::test_common::benchmark_corpus;

/// A query the fixture corpus answers with more than one memory: "access
/// tracking" appears in two of the three bodies. A single-candidate query
/// would let the page/pool and truncation assertions below pass vacuously.
const QUERY: &str = "access tracking";

/// The header of `observation_id`, which must exist.
fn header_of(conn: &rusqlite::Connection, observation_id: &str) -> StoredObservation {
    fetch(conn, observation_id)
        .expect("fetch")
        .expect("the observation exists")
        .0
}

/// The candidates of `observation_id`, which must exist.
fn candidates_of(conn: &rusqlite::Connection, observation_id: &str) -> Vec<StoredCandidate> {
    fetch(conn, observation_id)
        .expect("fetch")
        .expect("the observation exists")
        .1
}

/// Capture one query over the fixture corpus and hand back the connection so
/// the caller can read what landed.
fn capture_query(
    cfg: &Config,
    window: PageWindow,
) -> (tempfile::TempDir, rusqlite::Connection, String) {
    let (tmp, paths, _defaults) = benchmark_corpus::seeded_home();
    let conn = connection::open(paths.db_path()).expect("open db");
    let time_scope = scope::scope_from_flags(None, None, None).expect("scope");
    let filters = Filters {
        repo: None,
        kind: None,
        scope: &time_scope,
        domains: Domains::all(),
    };
    let domain_filters = DomainFilters {
        lang: None,
        path_globs: &[],
    };
    let query = UnifiedQuery {
        text: QUERY,
        vector: None,
        filters,
        domain_filters,
    };
    let legs = unified::run_legs(cfg, &conn, query, window).expect("run legs");
    let pool_size = legs.pool;
    let facts =
        candidate_facts::collect(&conn, &legs, cfg.observations.max_text_bytes).expect("facts");
    let pool = unified::fuse_legs(cfg, &conn, legs).expect("fuse");
    let id = capture(
        cfg,
        &conn,
        CaptureInput {
            query: QUERY,
            query_id: None,
            source: "find",
            filters: find_filters(cfg, filters, domain_filters, None),
            window,
            pool_size,
            pool: &pool,
            facts: &facts,
        },
    )
    .expect("capture");
    (tmp, conn, id)
}

#[test]
fn capture_is_armed_only_when_opted_in_and_allowed_to_write() {
    let mut cfg = Config::defaults();
    assert!(!armed(&cfg, true), "off by default even on a tracked run");
    cfg.observations.enabled = true;
    assert!(armed(&cfg, true));
    assert!(
        !armed(&cfg, false),
        "an untracked run — a read-only server, or the access-tracking test \
         hook — must write nothing however the config reads"
    );
}

#[test]
fn a_captured_observation_records_the_pool_with_the_contract_version() {
    let mut cfg = Config::defaults();
    cfg.observations.enabled = true;
    let window = PageWindow {
        offset: 0,
        limit: 2,
    };
    let (_tmp, conn, id) = capture_query(&cfg, window);

    let header = header_of(&conn, &id);
    assert_eq!(header.observation_version, 1);
    assert_eq!(header.query, QUERY);
    assert!(!header.knobs_hash.is_empty());
    assert!(!header.corpus_digest.is_empty());
    assert!(!header.truncated);

    let rows = candidates_of(&conn, &id);
    assert_eq!(
        rows.len() as i64,
        header.candidate_count,
        "the header must count exactly the rows written"
    );
    assert!(
        rows.len() >= 2,
        "the fixture query must pool more than one candidate, or every \
         page-versus-pool assertion in this file passes vacuously; got {}",
        rows.len()
    );
    assert_eq!(
        rows.iter().map(|r| r.pool_position).collect::<Vec<_>>(),
        (1..=rows.len() as i64).collect::<Vec<_>>(),
        "pool positions are retrieval's own order, 1-based and dense"
    );
    for row in &rows {
        assert!(
            !row.unresolved,
            "a live memory candidate always has a content snapshot"
        );
        assert!(
            row.candidate_ref.starts_with("memory:"),
            "unexpected reference {}",
            row.candidate_ref
        );
    }
}

#[test]
fn only_the_page_carries_a_returned_position() {
    let mut cfg = Config::defaults();
    cfg.observations.enabled = true;
    let window = PageWindow {
        offset: 0,
        limit: 1,
    };
    let (_tmp, conn, id) = capture_query(&cfg, window);
    let rows = candidates_of(&conn, &id);
    assert_eq!(rows[0].returned_position, Some(1));
    for row in rows.iter().skip(1) {
        assert_eq!(
            row.returned_position, None,
            "a candidate below the page cut is in the pool, not on the page"
        );
    }
}

#[test]
fn an_offset_page_numbers_its_returned_positions_from_one() {
    let mut cfg = Config::defaults();
    cfg.observations.enabled = true;
    let window = PageWindow {
        offset: 1,
        limit: 1,
    };
    let (_tmp, conn, id) = capture_query(&cfg, window);
    let rows = candidates_of(&conn, &id);
    assert!(
        rows.len() >= 2,
        "the fixture query pools at least two candidates"
    );
    assert_eq!(rows[0].returned_position, None, "skipped by the offset");
    assert_eq!(
        rows[1].returned_position,
        Some(1),
        "the first row of the second page is its own position 1"
    );
}

#[test]
fn the_recorded_filters_name_exactly_the_legs_in_scope() {
    let cfg = Config::defaults();
    let time_scope = scope::scope_from_flags(Some("2020-01-01"), None, None).expect("scope");
    let filters = Filters {
        repo: Some("demo"),
        kind: Some("decision"),
        scope: &time_scope,
        domains: Domains::of(&[Domain::Memory, Domain::Code]),
    };
    let globs = vec!["**/*.md".to_string()];
    let recorded = find_filters(
        &cfg,
        filters,
        DomainFilters {
            lang: Some("rust"),
            path_globs: &globs,
        },
        None,
    );
    assert_eq!(recorded.domains, vec!["memory", "code"]);
    assert_eq!(recorded.repo.as_deref(), Some("demo"));
    assert_eq!(recorded.kind.as_deref(), Some("decision"));
    assert_eq!(recorded.lang.as_deref(), Some("rust"));
    assert_eq!(recorded.path_globs, globs);
    assert!(
        recorded.since.is_some(),
        "a normalized --since bound is recorded"
    );
    assert!(
        recorded.until.is_none(),
        "an unset bound is null, not omitted"
    );
}

#[test]
fn a_supplied_vector_is_recorded_with_the_declared_embedder_identity() {
    use comemory::domains::learning::evaluation::candidate_observation::VectorScenario;
    let mut cfg = Config::defaults();
    cfg.embed.model = "ollama:nomic-embed-text".to_string();
    let time_scope = scope::scope_from_flags(None, None, None).expect("scope");
    let filters = Filters {
        repo: None,
        kind: None,
        scope: &time_scope,
        domains: Domains::all(),
    };
    let recorded = find_filters(
        &cfg,
        filters,
        DomainFilters {
            lang: None,
            path_globs: &[],
        },
        Some(&[0.1_f32, 0.2, 0.3]),
    );
    match recorded.vector {
        VectorScenario::Supplied { model, dim, digest } => {
            assert_eq!(model, "ollama:nomic-embed-text");
            assert_eq!(dim, 3);
            assert!(!digest.is_empty());
        }
        VectorScenario::Lexical => panic!("a supplied vector must not record as lexical"),
    }
}

#[test]
fn the_embedder_identity_falls_back_through_the_hint_to_unknown() {
    let mut cfg = Config::defaults();
    assert_eq!(
        embed_model(&cfg),
        "unknown",
        "a vector nobody named cannot be attributed to an embedder"
    );
    cfg.embed_hint = Some("  ".to_string());
    assert_eq!(embed_model(&cfg), "unknown", "blank is not an identity");
    cfg.embed_hint = Some("hint-model".to_string());
    assert_eq!(embed_model(&cfg), "hint-model");
    cfg.embed.model = "declared-model".to_string();
    assert_eq!(
        embed_model(&cfg),
        "declared-model",
        "[embed] model outranks the free-form hint"
    );
}

#[test]
fn the_candidate_bound_never_cuts_into_the_returned_page() {
    let mut cfg = Config::defaults();
    cfg.observations.max_candidates = 2;
    let head = PageWindow {
        offset: 0,
        limit: 10,
    };
    assert_eq!(
        candidate_cut(&cfg, head, 20),
        10,
        "the bound is raised to cover the whole page a caller was handed"
    );
    let deep = PageWindow {
        offset: 8,
        limit: 4,
    };
    assert_eq!(
        candidate_cut(&cfg, deep, 20),
        12,
        "offset + limit, not limit"
    );
    let unlimited = PageWindow {
        offset: 0,
        limit: 0,
    };
    assert_eq!(
        candidate_cut(&cfg, unlimited, 20),
        20,
        "limit 0 returns the whole tail, so nothing may be cut"
    );

    cfg.observations.max_candidates = 50;
    assert_eq!(
        candidate_cut(&cfg, head, 20),
        20,
        "the cut never exceeds the pool"
    );
}

#[test]
fn a_bound_below_the_pool_marks_the_observation_truncated() {
    let mut cfg = Config::defaults();
    cfg.observations.enabled = true;
    cfg.observations.max_candidates = 1;
    let window = PageWindow {
        offset: 0,
        limit: 1,
    };
    let (_tmp, conn, id) = capture_query(&cfg, window);
    let header = header_of(&conn, &id);
    let rows = candidates_of(&conn, &id);
    assert_eq!(rows.len(), 1);
    assert_eq!(header.candidate_count, 1);
    assert!(
        header.truncated,
        "the fixture corpus pools more than one candidate for this query, so \
         a bound of 1 must be recorded as a truncation"
    );
}
