#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/learning/judge.rs`, against a real observation
//! captured from the real retrieval legs over the shared benchmark corpus.
//!
//! The refusals are the contract: a target the pool never returned, a pinned
//! content version that no longer holds, and an unknown contract version each
//! refuse the whole call with nothing written.

use comemory::config::Config;
use comemory::config::paths::Paths;
use comemory::domains::learning::evaluation::candidate_facts;
use comemory::domains::learning::evaluation::candidate_identity::parse_ref;
use comemory::domains::learning::judge::{Outcome, Request, run, target_of};
use comemory::domains::learning::observation_capture::{CaptureInput, capture, find_filters};
use comemory::domains::retrieval::scope::{self, Domains, Filters};
use comemory::domains::retrieval::unified::{self, DomainFilters, UnifiedQuery};
use comemory::store::candidate_judgments::fetch_for_observation;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use comemory::utilities::pagination::PageWindow;

use crate::test_common::benchmark_corpus;

/// A query the fixture corpus answers with more than one memory.
const QUERY: &str = "access tracking";

/// A captured observation over the fixture corpus, plus everything a test
/// needs to drive `judge` against it.
struct Captured {
    _tmp: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    observation_id: String,
    /// Every candidate reference the observation recorded, in pool order.
    refs: Vec<String>,
}

fn captured() -> Captured {
    let (tmp, paths, _defaults) = benchmark_corpus::seeded_home();
    let mut cfg = Config::defaults();
    cfg.observations.enabled = true;
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
    let window = PageWindow {
        offset: 0,
        limit: 2,
    };
    let legs = unified::run_legs(
        &cfg,
        &conn,
        UnifiedQuery {
            text: QUERY,
            vector: None,
            filters,
            domain_filters,
        },
        window,
    )
    .expect("run legs");
    let pool_size = legs.pool;
    let facts =
        candidate_facts::collect(&conn, &legs, cfg.observations.max_text_bytes).expect("facts");
    let pool = unified::fuse_legs(&cfg, &conn, legs).expect("fuse");
    let observation_id = capture(
        &cfg,
        &conn,
        CaptureInput {
            query: QUERY,
            query_id: None,
            source: "find",
            filters: find_filters(&cfg, filters, domain_filters, None),
            window,
            pool_size,
            pool: &pool,
            facts: &facts,
        },
    )
    .expect("capture");
    let refs = comemory::store::candidate_observations::fetch(&conn, &observation_id)
        .expect("fetch")
        .expect("the observation exists")
        .1
        .into_iter()
        .map(|c| c.candidate_ref)
        .collect::<Vec<_>>();
    assert!(
        refs.len() >= 2,
        "the fixture query must pool at least two candidates, or the refusal \
         tests below cannot distinguish a miss from an empty pool"
    );
    Captured {
        _tmp: tmp,
        paths,
        cfg,
        observation_id,
        refs,
    }
}

fn judge(
    c: &Captured,
    refs: Vec<String>,
    source: Option<&str>,
) -> comemory::prelude::Result<Outcome> {
    let mut ctx = Ctx::lazy(&c.paths, &c.cfg);
    run(
        &mut ctx,
        Request {
            observation: c.observation_id.clone(),
            refs,
            source: source.map(str::to_string),
        },
    )
}

fn recorded_relevances(c: &Captured) -> std::collections::HashMap<String, i64> {
    let conn = connection::open(c.paths.db_path()).expect("open");
    fetch_for_observation(&conn, &c.observation_id).expect("fetch")
}

#[test]
fn a_verdict_on_an_observed_candidate_is_recorded_as_manual() {
    let c = captured();
    let outcome = judge(&c, vec![format!("{}=3", c.refs[0])], None).expect("judge");
    match outcome {
        Outcome::Recorded(r) => {
            assert_eq!(r.recorded, 1);
            assert_eq!(
                r.provenance, "manual",
                "a typed verdict is a human one — the #130 explicit -> manual mapping"
            );
        }
        Outcome::Report(_) => panic!("a verdict must record, not report"),
    }
    assert_eq!(recorded_relevances(&c).get(&c.refs[0]), Some(&3));
}

#[test]
fn an_implicit_source_records_the_implicit_provenance() {
    let c = captured();
    let outcome = judge(&c, vec![format!("{}=1", c.refs[0])], Some("implicit")).expect("judge");
    match outcome {
        Outcome::Recorded(r) => assert_eq!(r.provenance, "implicit"),
        Outcome::Report(_) => panic!("a verdict must record"),
    }
}

#[test]
fn an_unknown_source_is_refused_before_anything_is_written() {
    let c = captured();
    let err = judge(&c, vec![format!("{}=1", c.refs[0])], Some("manual"))
        .expect_err("`manual` is not a wire source word");
    assert!(
        err.to_string().contains("expected explicit or implicit"),
        "unexpected error: {err}"
    );
    assert!(recorded_relevances(&c).is_empty());
}

#[test]
fn a_re_judgment_replaces_the_previous_verdict() {
    let c = captured();
    judge(&c, vec![format!("{}=3", c.refs[0])], None).expect("first");
    judge(&c, vec![format!("{}=0", c.refs[0])], None).expect("second");
    let stored = recorded_relevances(&c);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored.get(&c.refs[0]), Some(&0));
}

#[test]
fn no_verdict_reports_the_observation_without_writing() {
    let c = captured();
    judge(&c, vec![format!("{}=2", c.refs[1])], None).expect("seed a verdict");
    let outcome = judge(&c, Vec::new(), None).expect("report");
    let Outcome::Report(report) = outcome else {
        panic!("an empty verdict list must report");
    };
    assert_eq!(report.observation_id, c.observation_id);
    assert_eq!(report.query, QUERY);
    assert_eq!(report.observation_version, 1);
    assert_eq!(report.candidate_count as usize, c.refs.len());
    assert_eq!(report.candidates.len(), c.refs.len());
    assert_eq!(report.candidates[0].pool_position, 1);
    assert_eq!(report.candidates[0].returned_position, Some(1));
    assert_eq!(
        report.candidates[1].relevance,
        Some(2),
        "a recorded verdict is reported beside its candidate"
    );
    assert_eq!(
        report.candidates[0].relevance, None,
        "an unjudged candidate reports no relevance"
    );
    assert!(
        !report.candidates[0].title.is_empty(),
        "the captured locator carries a headline"
    );
}

#[test]
fn a_target_the_pool_never_returned_is_refused_as_a_recall_miss() {
    let c = captured();
    // A well-formed reference to a memory that exists in the corpus but was
    // not in this query's pool.
    let absent = format!("memory:{}:{}", "ffffffff", "0".repeat(64));
    let err = judge(
        &c,
        vec![format!("{}=3", c.refs[0]), format!("{absent}=3")],
        None,
    )
    .expect_err("a pool-recall miss must refuse the call");
    let msg = err.to_string();
    assert!(
        msg.contains("candidate-pool recall miss"),
        "unexpected: {msg}"
    );
    assert!(msg.contains(&absent), "the refusal must name the reference");
    assert!(
        recorded_relevances(&c).is_empty(),
        "all-or-nothing: the reference that DID match must not be written either"
    );
}

#[test]
fn a_pinned_version_that_no_longer_holds_is_refused_as_stale() {
    let c = captured();
    let identity = parse_ref(&c.refs[0]).expect("parse");
    let target = target_of(&identity);
    let stale = format!(
        "memory:{}:{}",
        target.id.expect("a memory target carries an id"),
        "a".repeat(64)
    );
    let err = judge(&c, vec![format!("{stale}=3")], None).expect_err("stale must refuse");
    let msg = err.to_string();
    assert!(msg.contains("is stale"), "unexpected: {msg}");
    assert!(
        msg.contains(identity.content_version()),
        "the refusal must name the version the observation actually saw: {msg}"
    );
    assert!(recorded_relevances(&c).is_empty());
}

#[test]
fn an_observation_written_at_an_unknown_contract_version_is_refused() {
    let c = captured();
    let conn = connection::open(c.paths.db_path()).expect("open");
    conn.execute(
        "UPDATE candidate_query_observations SET observation_version = 999 \
          WHERE observation_id = ?1",
        [&c.observation_id],
    )
    .expect("bump the version");
    let err = judge(&c, vec![format!("{}=3", c.refs[0])], None)
        .expect_err("an unknown contract version must refuse");
    let msg = err.to_string();
    assert!(msg.contains("999"), "unexpected: {msg}");
    assert!(msg.contains(&c.observation_id), "unexpected: {msg}");
    assert!(recorded_relevances(&c).is_empty());
}

#[test]
fn an_unknown_observation_id_is_unavailable_rather_than_silently_empty() {
    let c = captured();
    let mut ctx = Ctx::lazy(&c.paths, &c.cfg);
    let err = run(
        &mut ctx,
        Request {
            observation: "o-20260918-deadbeef".to_string(),
            refs: vec![format!("{}=3", c.refs[0])],
            source: None,
        },
    )
    .expect_err("an unknown observation must fail");
    assert!(
        err.to_string().contains("o-20260918-deadbeef"),
        "the error must name the id: {err}"
    );
}

#[test]
fn a_malformed_observation_id_is_refused_before_the_database_is_opened() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(tmp.path().join(".comemory"));
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let err = run(
        &mut ctx,
        Request {
            observation: "not-an-observation".to_string(),
            refs: Vec::new(),
            source: None,
        },
    )
    .expect_err("a malformed id must fail");
    assert!(err.to_string().contains("o-<yyyymmdd>-<8hex>"), "{err}");
    assert!(
        !paths.db_path().exists(),
        "a malformed invocation must not create a database"
    );
}

#[test]
fn a_malformed_verdict_argument_is_refused_naming_it() {
    let c = captured();
    for (bad, expected) in [
        (c.refs[0].clone(), "expected <candidate_ref>=<relevance>"),
        (format!("{}=x", c.refs[0]), "expected an integer"),
        (format!("{}=4", c.refs[0]), "out of range"),
        (format!("{}=-1", c.refs[0]), "out of range"),
        ("nonsense:a:b=1".to_string(), "unknown domain"),
        ("memory:onlyone=1".to_string(), "expected 2 components"),
    ] {
        let err = judge(&c, vec![bad.clone()], None)
            .expect_err("a malformed verdict must be refused")
            .to_string();
        assert!(err.contains(expected), "`{bad}` produced: {err}");
    }
    assert!(recorded_relevances(&c).is_empty());
}

#[test]
fn every_domain_target_round_trips_through_its_reference() {
    for reference in [
        "memory:aaaa0001:hash1",
        "code:demo:src/ranking.rs:activation_boost:oid1",
        "document:d1:guides/chunking.md:rev1:3",
    ] {
        let identity = parse_ref(reference).expect("parse");
        let target = target_of(&identity);
        let key = target
            .resolve("t")
            .expect("a round-tripped target validates");
        assert_eq!(
            key.matches(&identity),
            comemory::domains::learning::evaluation::judgment::MatchOutcome::Yes,
            "a target built from an identity must match that identity"
        );
        assert_eq!(target.domain, identity.domain());
    }
}

#[test]
fn a_corrupt_stored_candidate_is_named_apart_from_a_bad_argument() {
    let c = captured();
    let conn = connection::open(c.paths.db_path()).expect("open");
    conn.execute(
        "UPDATE candidate_observations SET candidate_ref = 'memory:onlyone' \
          WHERE observation_id = ?1 AND pool_position = 1",
        [&c.observation_id],
    )
    .expect("corrupt one stored reference");

    let err = judge(&c, vec![format!("{}=3", c.refs[1])], None)
        .expect_err("an unreadable stored row must fail the call");
    assert_eq!(
        err.to_string(),
        format!(
            "other: observation `{}` holds an unreadable candidate at pool position 1: \
             config: candidate ref `memory:onlyone`: expected 2 components, found 1. That \
             row was not written by this build's observation contract.",
            c.observation_id
        ),
        "the whole sentence is the contract: it names the observation, the pool \
         position, the underlying parse failure, and what that means"
    );
    assert!(
        !matches!(err, comemory::prelude::Error::Config(_)),
        "a bad `--ref` argument is Error::Config; a corrupt stored row must not \
         share that classification, or the two are indistinguishable by exit code"
    );
}
