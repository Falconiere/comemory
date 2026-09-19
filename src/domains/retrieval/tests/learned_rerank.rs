#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::retrieval::learned_rerank`] — the one learned ordering
//! stage, against a real SQLite store and a real scorer child process.
//!
//! The applied path runs the shipped `integrations/reranker/comemory_rerank.py`
//! in its deterministic `lexical-overlap` mode, so the wire protocol, the
//! identity gate and the process runner are all exercised for real. `python3`
//! is a prerequisite, as it already is for
//! `src/utilities/tests/rerank_runner_2.rs`; an absent interpreter FAILS these
//! tests rather than skipping them.

use comemory::config::Config;
use comemory::eval::candidate_facts;
use comemory::eval::candidate_identity::CandidateDomain;
use comemory::retrieval::learned_rerank::{self, Candidates, LearnedPlan, LearnedStage};
use comemory::retrieval::rerank::Reranked;
use comemory::retrieval::scope::{Filters, TimeScope};
use comemory::retrieval::{diversify, pipeline};
use comemory::utilities::pagination::PageWindow;
use comemory::utilities::rerank_outcome::{RerankDeclined, RerankFailure, RerankOutcome};

/// The shipped reference backend, in its deterministic mode.
fn backend() -> Vec<String> {
    vec![
        "python3".to_string(),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/reranker/comemory_rerank.py"
        )
        .to_string(),
        "score".to_string(),
        "--scoring".to_string(),
        "lexical-overlap".to_string(),
    ]
}

/// A config with the learned stage pointed at `command`.
fn enabled(command: Vec<String>, prefix: usize) -> Config {
    let mut cfg = Config::defaults();
    cfg.rerank.enabled = true;
    cfg.rerank.command = command;
    cfg.rerank.model = "lexical-overlap@1".into();
    cfg.rerank.prefix = prefix;
    cfg
}

/// SimHash spread so the diversify stage cannot collapse two fixtures.
fn spread_simhash(nth: u64) -> u64 {
    (nth + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn open_db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    (dir, conn)
}

fn seed_body(conn: &rusqlite::Connection, id: &str, body: &str, nth: u64, quality: i64) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, author, quality, schema, content_hash,
                              body, created_at, updated_at, md_path, simhash)
         VALUES (?1, ?1, 'note', 'd', 'f', ?4, 1, ?1, ?2,
                 '2026-06-09T00:00:00Z', '2026-06-09T00:00:00Z', ?1, ?3)",
        rusqlite::params![id, body, spread_simhash(nth) as i64, quality],
    )
    .expect("seed memory");
    conn.execute(
        "INSERT INTO memory_fts(memory_id, body, tags) VALUES (?1, ?2, '')",
        rusqlite::params![id, body],
    )
    .expect("seed fts");
}

/// A three-memory corpus whose deterministic order and lexical-overlap order
/// disagree.
///
/// Jaccard similarity against `"sqlite busy timeout"` falls as a body adds
/// tokens, while `quality` — a real multiplicative rerank prior — is set to
/// push the lowest-overlap body up the deterministic list. The disagreement is
/// asserted, not assumed, by `a_real_backend_reorders_the_prefix_and_keeps_the_tail`.
fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, conn) = open_db();
    seed_body(
        &conn,
        "aaaa0001",
        "sqlite busy timeout pool fix with a longer body that dilutes overlap",
        0,
        5,
    );
    seed_body(&conn, "bbbb0002", "sqlite busy timeout pool", 1, 3);
    seed_body(&conn, "cccc0003", "sqlite busy timeout", 2, 1);
    (dir, conn)
}

/// The deterministic memory ranking for `query` over `conn`.
fn ranked(cfg: &Config, conn: &rusqlite::Connection, query: &str) -> Vec<Reranked> {
    let scope = TimeScope::default();
    let window = PageWindow::top_k(cfg);
    pipeline::rank(
        cfg,
        conn,
        query,
        None,
        Filters {
            scope: &scope,
            ..Filters::none()
        },
        pipeline::candidate_pool(cfg, window),
    )
    .expect("rank")
}

fn ids(hits: &[Reranked]) -> Vec<String> {
    hits.iter().map(|h| h.memory_id.clone()).collect()
}

/// Plan a call over the whole memory ranking.
fn plan_over(
    cfg: &Config,
    conn: &rusqlite::Connection,
    query: &str,
    hits: &[Reranked],
) -> Option<learned_rerank::LearnedCall> {
    let keys = learned_rerank::memory_keys(hits);
    let stage = LearnedStage::from_config(cfg);
    learned_rerank::plan_call(
        stage.as_ref(),
        conn,
        query,
        Candidates {
            memory: hits,
            keys: &keys,
            ..Candidates::default()
        },
    )
    .expect("plan")
}

#[test]
fn a_disabled_stage_is_none_and_plans_nothing() {
    let cfg = Config::defaults();
    assert!(
        LearnedStage::from_config(&cfg).is_none(),
        "the default configuration must construct no stage at all"
    );
    let (_dir, conn) = seeded();
    let hits = ranked(&cfg, &conn, "sqlite busy timeout");
    assert!(
        plan_over(&cfg, &conn, "sqlite busy timeout", &hits).is_none(),
        "a disabled stage plans no call"
    );
}

#[test]
fn an_empty_ranking_plans_no_call() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = seeded();
    assert!(
        plan_over(&cfg, &conn, "nothing whatsoever matches this", &[]).is_none(),
        "an empty candidate set spawns nothing"
    );
}

#[test]
fn the_wire_id_is_the_pool_key_and_the_reference_rides_beside_it() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = seeded();
    let query = "sqlite busy timeout";
    let hits = ranked(&cfg, &conn, query);
    let call = plan_over(&cfg, &conn, query, &hits).expect("a call");
    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &hits, &[], &[], stage.text_bytes()).expect("facts");
    let plan = call.plan();
    for (i, hit) in hits.iter().enumerate() {
        assert_eq!(
            plan.submitted[i],
            format!("memory:{}", hit.memory_id),
            "the id on the wire is the candidate pool's own key"
        );
        let key = (CandidateDomain::Memory, hit.memory_id.clone());
        assert_eq!(
            plan.refs[i],
            facts
                .get(&key)
                .expect("facts entry")
                .identity
                .candidate_ref(),
            "and the observation reference is carried beside it"
        );
    }
}

/// The reason the wire id is NOT the candidate observation reference: two
/// same-named functions in one file share `(repo, path, symbol, blob_oid)`, so
/// submitting references would repeat an id and #211 would refuse the whole
/// request with `DuplicateCandidateId`.
#[test]
fn two_same_named_symbols_in_one_file_still_score() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = open_db();
    for line in [10i64, 40] {
        let id = comemory::store::code_row::insert(
            &conn,
            &comemory::store::code_row::CodeSymbolRow {
                repo: "demo",
                path: "alpha.rs",
                blob_oid: "oid",
                symbol: "new",
                kind: "function",
                lang: "rust",
                line_start: line,
                line_end: line + 5,
                snippet: "fn new() { sqlite_busy_timeout() }",
                simhash: 0,
                parent_id: None,
            },
        )
        .expect("insert code symbol");
        comemory::store::fts::index_code(
            &conn,
            id,
            "new",
            "fn new() { sqlite_busy_timeout() }",
            "alpha.rs",
        )
        .expect("index code fts");
    }
    let hits = comemory::retrieval::code_search::search_code_hits(
        &cfg,
        &conn,
        "new",
        None,
        Some("demo"),
        None,
        50,
    )
    .expect("code search");
    assert_eq!(hits.len(), 2, "both same-named symbols must be candidates");

    let keys = learned_rerank::code_keys(&hits);
    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let call = learned_rerank::plan_call(
        Some(&stage),
        &conn,
        "new",
        Candidates {
            code: &hits,
            keys: &keys,
            ..Candidates::default()
        },
    )
    .expect("plan")
    .expect("a call");
    let plan = call.plan();
    assert_eq!(
        plan.refs[0], plan.refs[1],
        "the two observation references really do collide"
    );
    assert_ne!(
        plan.submitted[0], plan.submitted[1],
        "but the wire ids do not"
    );
    let outcome = call.score();
    assert!(
        outcome.is_applied(),
        "so the stage applies rather than declining: {:?}",
        outcome.failure()
    );
    let (ordered, report) = learned_rerank::apply(hits, &plan, &outcome);
    assert!(report.applied);
    assert_eq!(ordered.len(), 2);
}

#[test]
fn a_real_backend_reorders_the_prefix_and_keeps_the_tail() {
    let cfg = enabled(backend(), 2);
    let (_dir, conn) = seeded();
    let query = "sqlite busy timeout";
    let hits = ranked(&cfg, &conn, query);
    assert_eq!(hits.len(), 3, "the fixture must produce three candidates");
    let deterministic = ids(&hits);
    let call = plan_over(&cfg, &conn, query, &hits).expect("a call");
    let plan = call.plan();
    assert_eq!(plan.submitted.len(), 2, "only the prefix is submitted");
    let outcome = call.score();
    assert!(
        outcome.is_applied(),
        "the shipped backend must answer: {:?}",
        outcome.failure()
    );
    let (ordered, report) = learned_rerank::apply(hits, &plan, &outcome);
    assert!(report.applied);
    assert_eq!(report.model, "lexical-overlap@1");
    assert_eq!(report.adapter, None);
    assert_eq!(report.pool, 3);
    assert_eq!(report.prefix, 2);
    assert_eq!(report.scores.len(), 2);
    let after = ids(&ordered);
    assert_eq!(
        after[2], deterministic[2],
        "the tail below the prefix must be preserved exactly"
    );
    assert_ne!(
        after[..2],
        deterministic[..2],
        "the fixture must make the backend actually reorder the prefix, \
         or this test passes on an `apply` that ignores the outcome"
    );
    let mut head = after[..2].to_vec();
    head.sort();
    let mut expected = deterministic[..2].to_vec();
    expected.sort();
    assert_eq!(
        head, expected,
        "and the reorder must be a permutation of the prefix, never a different set"
    );
    for (i, s) in report.scores.iter().enumerate() {
        assert_eq!(s.rank, i + 1);
        assert!((1..=2).contains(&s.deterministic_rank));
        assert!(s.score.is_finite());
    }
}

#[test]
fn a_refusal_restores_the_entire_original_order() {
    let cfg = enabled(
        vec!["/nonexistent/comemory-scorer-does-not-exist".into()],
        50,
    );
    let (_dir, conn) = seeded();
    let query = "sqlite busy timeout";
    let hits = ranked(&cfg, &conn, query);
    let deterministic = ids(&hits);
    let call = plan_over(&cfg, &conn, query, &hits).expect("a call");
    let plan = call.plan();
    let outcome = call.score();
    assert!(matches!(
        outcome.failure(),
        Some(RerankFailure::Spawn { .. })
    ));
    let (ordered, report) = learned_rerank::apply(hits, &plan, &outcome);
    assert!(!report.applied);
    assert!(report.scores.is_empty(), "a refusal scored nothing");
    assert!(
        report
            .fallback
            .as_deref()
            .is_some_and(|f| f.contains("spawn failed")),
        "the refusal must name itself"
    );
    assert_eq!(
        ids(&ordered),
        deterministic,
        "a refusal must restore the complete original order"
    );
}

#[test]
fn an_order_that_is_not_a_permutation_is_ignored() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = seeded();
    let hits = ranked(&cfg, &conn, "sqlite busy timeout");
    let deterministic = ids(&hits);
    // A plan naming candidates the ranking does not contain: `order_ids` then
    // yields ids no slot answers to, and the deterministic order must stand.
    let plan = LearnedPlan {
        submitted: vec!["memory:zzzz:0".into(), "memory:yyyy:0".into()],
        refs: vec!["memory:zzzz:0".into(), "memory:yyyy:0".into()],
        model: "lexical-overlap@1".into(),
        adapter: None,
        request_id: "rr-20260918-deadbeef".into(),
    };
    let outcome = RerankOutcome::Declined(RerankDeclined {
        request_id: "rr-20260918-deadbeef".into(),
        failure: RerankFailure::EmptyCandidates,
        original_order: vec!["memory:zzzz:0".into(), "memory:yyyy:0".into()],
        stderr_excerpt: String::new(),
    });
    let (ordered, report) = learned_rerank::apply(hits, &plan, &outcome);
    assert!(!report.applied);
    assert_eq!(
        ids(&ordered),
        deterministic,
        "an order naming unknown candidates must not drop or reorder anything"
    );
}

#[test]
fn a_candidate_whose_row_vanished_keeps_its_place_with_empty_text() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = seeded();
    let hits = ranked(&cfg, &conn, "sqlite busy timeout");
    let keys = learned_rerank::memory_keys(&hits);
    // A key the facts map cannot answer — the shape `candidate_facts` leaves
    // behind when a row is re-indexed away between the ranking and the read.
    let mut orphaned = keys.clone();
    orphaned.push((CandidateDomain::Code, "999999".into()));
    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &hits, &[], &[], stage.text_bytes()).expect("facts");
    let call = stage
        .plan(
            "sqlite busy timeout",
            &orphaned,
            &facts,
            time::OffsetDateTime::now_utc(),
        )
        .expect("a call");
    let plan = call.plan();
    assert_eq!(
        plan.submitted.len(),
        orphaned.len(),
        "a candidate with no facts must still be submitted, never dropped"
    );
    assert_eq!(
        plan.submitted[orphaned.len() - 1],
        "code:999999",
        "every candidate is offered under its pool key, resolvable or not"
    );
    assert!(
        call.score().is_applied(),
        "an empty candidate text is scoreable, not a protocol error"
    );
}

/// The real race: a code row is re-indexed away between the deterministic
/// ranking and the facts read. `index_code` purges and reinserts a touched
/// file's rows, so the `code_symbols` row a ranking is holding can be gone by
/// the time its text is fetched.
#[test]
fn a_code_row_reindexed_away_is_still_submitted_at_its_position() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = open_db();
    let symbol_id = comemory::store::code_row::insert(
        &conn,
        &comemory::store::code_row::CodeSymbolRow {
            repo: "demo",
            path: "alpha.rs",
            blob_oid: "oid",
            symbol: "sqlite_busy_timeout",
            kind: "function",
            lang: "rust",
            line_start: 1,
            line_end: 10,
            snippet: "fn sqlite_busy_timeout() {}",
            simhash: 0,
            parent_id: None,
        },
    )
    .expect("insert code symbol");
    comemory::store::fts::index_code(
        &conn,
        symbol_id,
        "sqlite_busy_timeout",
        "fn sqlite_busy_timeout() {}",
        "alpha.rs",
    )
    .expect("index code fts");
    let hits = comemory::retrieval::code_search::search_code_hits(
        &cfg,
        &conn,
        "sqlite_busy_timeout",
        None,
        Some("demo"),
        None,
        50,
    )
    .expect("code search");
    assert_eq!(hits.len(), 1, "the ranking must hold the seeded symbol");

    // The re-index happens HERE, between the ranking and the facts read.
    conn.execute(
        "DELETE FROM code_symbols WHERE id = ?1",
        rusqlite::params![symbol_id],
    )
    .expect("purge the row");

    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &[], &hits, &[], stage.text_bytes()).expect("facts");
    let key = (CandidateDomain::Code, symbol_id.to_string());
    let entry = facts
        .get(&key)
        .expect("the candidate keeps its facts entry");
    assert!(
        !entry.text_available,
        "the vanished row is reported as unavailable, not silently dropped"
    );
    assert!(entry.text.text.is_empty(), "and its text is empty");

    let keys = learned_rerank::code_keys(&hits);
    let call = stage
        .plan(
            "sqlite_busy_timeout",
            &keys,
            &facts,
            time::OffsetDateTime::now_utc(),
        )
        .expect("a call");
    let plan = call.plan();
    assert_eq!(
        plan.submitted.len(),
        hits.len(),
        "the pool must not shrink because a row vanished"
    );
    let outcome = call.score();
    assert!(outcome.is_applied(), "and the scorer still answers");
    let (ordered, report) = learned_rerank::apply(hits, &plan, &outcome);
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0].symbol_id, symbol_id, "at its own position");
    assert_eq!(report.pool, 1);
}

#[test]
fn the_prefix_never_exceeds_the_ranking() {
    let cfg = enabled(backend(), 500);
    let (_dir, conn) = seeded();
    let query = "sqlite busy timeout";
    let hits = ranked(&cfg, &conn, query);
    let call = plan_over(&cfg, &conn, query, &hits).expect("a call");
    let plan = call.plan();
    assert_eq!(
        plan.submitted.len(),
        hits.len(),
        "a prefix larger than the pool submits the whole pool and no more"
    );
    let outcome = call.score();
    assert!(outcome.is_applied());
    let deterministic = ids(&hits);
    let (ordered, report) = learned_rerank::apply(hits, &plan, &outcome);
    assert_eq!(report.prefix, deterministic.len(), "the tail is empty");
    let mut got = ids(&ordered);
    got.sort();
    let mut want = deterministic;
    want.sort();
    assert_eq!(got, want, "and every candidate survives the reorder");
}

/// Both guard branches of [`apply`]: an order that is not a permutation of the
/// submitted prefix leaves the ranking alone AND says the deterministic order
/// stands, rather than claiming a reorder that did not happen.
#[test]
fn a_plan_the_ranking_cannot_satisfy_is_refused_and_says_so() {
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = seeded();
    let hits = ranked(&cfg, &conn, "sqlite busy timeout");
    let deterministic = ids(&hits);
    for (name, submitted) in [
        // More submitted candidates than the ranking holds.
        (
            "prefix larger than the pool",
            vec![
                "memory:aaaa0001".to_string(),
                "memory:bbbb0002".to_string(),
                "memory:cccc0003".to_string(),
                "memory:dddd0004".to_string(),
            ],
        ),
        // A repeated id, so no assignment of slots can be a permutation.
        (
            "a repeated candidate id",
            vec!["memory:aaaa0001".to_string(), "memory:aaaa0001".to_string()],
        ),
    ] {
        let plan = LearnedPlan {
            refs: submitted.clone(),
            submitted: submitted.clone(),
            model: "lexical-overlap@1".into(),
            adapter: None,
            request_id: "rr-20260918-deadbeef".into(),
        };
        let outcome = RerankOutcome::Declined(RerankDeclined {
            request_id: "rr-20260918-deadbeef".into(),
            failure: RerankFailure::EmptyCandidates,
            original_order: submitted,
            stderr_excerpt: String::new(),
        });
        let (ordered, report) = learned_rerank::apply(hits.clone(), &plan, &outcome);
        assert_eq!(ids(&ordered), deterministic, "{name} keeps every candidate");
        assert!(!report.applied, "{name} must not claim an applied reorder");
        assert!(
            report.scores.is_empty(),
            "{name} must report no learned order"
        );
        assert!(report.fallback.is_some(), "{name} must name its refusal");
    }
}

#[test]
fn the_fixture_is_not_collapsed_by_near_duplicate_detection() {
    // Guards the tests above: a collapsed fixture would silently have nothing
    // at the positions they assert on.
    let cfg = enabled(backend(), 50);
    let (_dir, conn) = seeded();
    let hits = ranked(&cfg, &conn, "sqlite busy timeout");
    let collapsed = diversify::diversify(hits.clone(), cfg.rank.near_dup_hamming, 1.0, hits.len());
    assert_eq!(collapsed.len(), hits.len());
}
