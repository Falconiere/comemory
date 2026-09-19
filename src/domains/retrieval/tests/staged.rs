#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::retrieval::staged`] — the pause point, resolved over a
//! real `Ctx` around a real SQLite connection and a real scorer child process.

use comemory::config::Config;
use comemory::config::paths::Paths;
use comemory::eval::candidate_facts;
use comemory::eval::candidate_identity::CandidateDomain;
use comemory::retrieval::learned_rerank::LearnedStage;
use comemory::retrieval::staged::{FinishStep, Paused, Staged, resolve};
use comemory::utilities::context::Ctx;

/// A config whose stage runs the shipped deterministic backend.
fn enabled() -> Config {
    let mut cfg = Config::defaults();
    cfg.rerank.enabled = true;
    cfg.rerank.command = vec![
        "python3".into(),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/reranker/comemory_rerank.py"
        )
        .into(),
        "score".into(),
        "--scoring".into(),
        "lexical-overlap".into(),
    ];
    cfg.rerank.model = "lexical-overlap@1".into();
    cfg
}

/// A real store and the `Ctx` over it.
fn store() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = comemory::store::connection::open(dir.path().join("c.db")).expect("open");
    (dir, conn)
}

#[test]
fn the_ready_arm_resolves_without_running_anything() {
    let (dir, mut conn) = store();
    let cfg = Config::defaults();
    let paths = Paths::new(dir.path().to_path_buf());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let out = resolve(&mut ctx, Staged::Ready(7usize)).expect("resolve");
    assert_eq!(out, 7);
}

#[test]
fn the_paused_arm_runs_the_scorer_then_the_continuation() {
    let (dir, mut conn) = store();
    let cfg = enabled();
    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &[], &[], &[], stage.text_bytes()).expect("facts");
    // Two candidates whose facts are absent, so the wire ids are the degenerate
    // `<domain>:<id>` form — enough for the backend to score and echo.
    let keys = vec![
        (CandidateDomain::Memory, "aaaa0001".to_string()),
        (CandidateDomain::Memory, "bbbb0002".to_string()),
    ];
    let call = stage
        .plan(
            "pool timeout",
            &keys,
            &facts,
            time::OffsetDateTime::now_utc(),
        )
        .expect("a call");
    let staged = Staged::Paused(Paused::new(
        call,
        FinishStep::new(|_ctx, outcome| Ok(outcome.order_ids().len())),
    ));
    let paths = Paths::new(dir.path().to_path_buf());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let scored = resolve(&mut ctx, staged).expect("resolve");
    assert_eq!(scored, 2, "the continuation sees the scorer's own order");
}

#[test]
fn map_projects_both_arms() {
    let (dir, mut conn) = store();
    let cfg = enabled();
    let paths = Paths::new(dir.path().to_path_buf());
    let ready: Staged<usize> = Staged::Ready(3);
    let mapped = ready.map(|v| Ok(v * 2)).expect("map");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    assert_eq!(resolve(&mut ctx, mapped).expect("resolve"), 6);

    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &[], &[], &[], stage.text_bytes()).expect("facts");
    let keys = vec![(CandidateDomain::Memory, "aaaa0001".to_string())];
    let call = stage
        .plan("pool", &keys, &facts, time::OffsetDateTime::now_utc())
        .expect("a call");
    let paused: Staged<usize> = Staged::Paused(Paused::new(
        call,
        FinishStep::new(|_ctx, outcome| Ok(outcome.order_ids().len())),
    ));
    let mapped = paused.map(|v| Ok(v + 10)).expect("map");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    assert_eq!(
        resolve(&mut ctx, mapped).expect("resolve"),
        11,
        "the projection is applied after the scorer, not before it"
    );
}

#[test]
fn a_continuation_error_surfaces_after_inference() {
    let (dir, mut conn) = store();
    let cfg = enabled();
    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &[], &[], &[], stage.text_bytes()).expect("facts");
    let keys = vec![(CandidateDomain::Memory, "aaaa0001".to_string())];
    let call = stage
        .plan("pool", &keys, &facts, time::OffsetDateTime::now_utc())
        .expect("a call");
    let staged: Staged<usize> = Staged::Paused(Paused::new(
        call,
        FinishStep::new(|_ctx, _outcome| {
            Err(comemory::errors::Error::Other("phase three failed".into()))
        }),
    ));
    let paths = Paths::new(dir.path().to_path_buf());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = resolve(&mut ctx, staged).expect_err("the continuation's error must propagate");
    assert!(
        err.to_string().contains("phase three failed"),
        "verbatim, not swallowed: {err}"
    );
}

#[test]
fn a_map_error_is_deferred_to_phase_three() {
    let (dir, mut conn) = store();
    let cfg = enabled();
    let stage = LearnedStage::from_config(&cfg).expect("stage");
    let facts =
        candidate_facts::collect_parts(&conn, &[], &[], &[], stage.text_bytes()).expect("facts");
    let keys = vec![(CandidateDomain::Memory, "aaaa0001".to_string())];
    let call = stage
        .plan("pool", &keys, &facts, time::OffsetDateTime::now_utc())
        .expect("a call");
    let paused: Staged<usize> = Staged::Paused(Paused::new(
        call,
        FinishStep::new(|_ctx, outcome| Ok(outcome.order_ids().len())),
    ));
    // `map` on a Paused arm must NOT run `f` yet, so building the projection
    // cannot fail here even though `f` always fails.
    let mapped: Staged<usize> = paused
        .map(|_| Err(comemory::errors::Error::Other("projection failed".into())))
        .expect("mapping a paused value never runs the projection");
    let paths = Paths::new(dir.path().to_path_buf());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let err = resolve(&mut ctx, mapped).expect_err("and it surfaces after inference");
    assert!(err.to_string().contains("projection failed"), "got: {err}");
}

#[test]
fn a_paused_value_is_send_so_it_can_cross_a_blocking_task() {
    // The whole reason `Paused` exists: an adapter holding a shared lock has to
    // be able to move it into another thread after dropping the guard.
    fn assert_send<T: Send>() {}
    assert_send::<Staged<usize>>();
    assert_send::<Paused<usize>>();
    assert_send::<FinishStep<usize>>();
}
