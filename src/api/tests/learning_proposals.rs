#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/learning_proposals.rs`: proposals derived from
//! real `eval_runs` rows against the live config, and the apply/discard
//! transitions — including the `config.toml` the apply actually writes
//! (AC-13's api-layer half; the HTTP half lives in
//! `src/serve/routes/tests/learning_console.rs`).

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::eval::tune::TuneCandidate;
use comemory::store::{connection, eval_runs};
use tempfile::TempDir;

/// A knob set that differs from `Config::defaults()` on rrf_k and
/// graph_hops, and matches it on everything else — so the diff is exactly
/// two entries and the test can name them.
fn shifted() -> TuneCandidate {
    let base = Config::defaults();
    TuneCandidate {
        rrf_k: 30.0,
        decay: base.rank.decay,
        mmr_lambda: base.rank.mmr_lambda,
        bm25_weights: base.retrieval.bm25_weights,
        graph_hops: base.retrieval.graph_hops + 1,
        graph_seeds: base.retrieval.graph_seeds,
    }
}

/// The live config's own knobs — a run recording these proposes nothing.
fn identical() -> TuneCandidate {
    let base = Config::defaults();
    TuneCandidate {
        rrf_k: base.retrieval.rrf_k,
        decay: base.rank.decay,
        mmr_lambda: base.rank.mmr_lambda,
        bm25_weights: base.retrieval.bm25_weights,
        graph_hops: base.retrieval.graph_hops,
        graph_seeds: base.retrieval.graph_seeds,
    }
}

/// Insert one run carrying `knobs`.
fn seed_run(conn: &rusqlite::Connection, id: &str, kind: &str, at: &str, knobs: &TuneCandidate) {
    let json = serde_json::to_string(knobs).expect("serialize knobs");
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id,
            kind,
            at,
            golden_pairs: 12,
            k: 3,
            recall: 0.6,
            mrr: 0.4,
            knobs: &json,
            applied: false,
        },
    )
    .expect("insert eval_runs row");
}

/// A migrated database under a fresh temp home.
fn open(home: &TempDir) -> (Paths, rusqlite::Connection) {
    let paths = Paths::new(home.path().join(".comemory"));
    paths.ensure_dirs().expect("dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, conn)
}

#[test]
fn list_offers_only_open_runs_whose_knobs_actually_differ() {
    let home = TempDir::new().expect("tempdir");
    let (paths, mut conn) = open(&home);
    seed_run(
        &conn,
        "open",
        "tune",
        "2026-09-01T10:00:00.000000000Z",
        &shifted(),
    );
    seed_run(
        &conn,
        "same",
        "tune",
        "2026-09-01T09:00:00.000000000Z",
        &identical(),
    );
    seed_run(
        &conn,
        "plain",
        "eval",
        "2026-09-01T08:00:00.000000000Z",
        &shifted(),
    );
    seed_run(
        &conn,
        "gone",
        "bandit",
        "2026-09-01T07:00:00.000000000Z",
        &shifted(),
    );
    eval_runs::set_discarded(&conn, "gone").expect("discard");

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let proposals = api::learning_proposals::list(&mut ctx).expect("list");

    let ids: Vec<&str> = proposals.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["open"],
        "a matching, an eval-kind, and a discarded run are all excluded"
    );
    let names: Vec<&str> = proposals[0].knobs.iter().map(|k| k.name).collect();
    assert_eq!(names, vec!["rrf_k", "graph_hops"]);
    assert_eq!(proposals[0].knobs[0].from.as_f64(), Some(60.0));
    assert_eq!(proposals[0].knobs[0].to.as_f64(), Some(30.0));
    assert_ne!(
        proposals[0].knobs[1].from, proposals[0].knobs[1].to,
        "every listed knob must differ"
    );
}

#[test]
fn apply_writes_config_toml_stamps_the_row_and_retires_the_proposal() {
    let home = TempDir::new().expect("tempdir");
    let (paths, mut conn) = open(&home);
    seed_run(
        &conn,
        "prop1",
        "tune",
        "2026-09-01T10:00:00.000000000Z",
        &shifted(),
    );

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let applied = api::learning_proposals::apply(&mut ctx, "prop1").expect("apply");
    assert_eq!(applied.id, "prop1");
    assert!(applied.applied);

    let written = std::fs::read_to_string(paths.config_file()).expect("config.toml exists");
    assert!(
        written.contains("rrf_k = 30.0"),
        "the winner's knobs must reach config.toml, got:\n{written}"
    );

    let row = eval_runs::get(&conn, "prop1").expect("get").expect("row");
    assert!(row.applied, "the run is stamped applied");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    assert!(
        api::learning_proposals::list(&mut ctx)
            .expect("list")
            .is_empty(),
        "an applied run is no longer offered"
    );
}

#[test]
fn discard_retires_a_proposal_without_touching_config_toml() {
    let home = TempDir::new().expect("tempdir");
    let (paths, mut conn) = open(&home);
    seed_run(
        &conn,
        "prop2",
        "bandit",
        "2026-09-01T10:00:00.000000000Z",
        &shifted(),
    );

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let discarded = api::learning_proposals::discard(&mut ctx, "prop2").expect("discard");
    assert_eq!(discarded.id, "prop2");
    assert!(discarded.discarded);
    assert!(
        !paths.config_file().exists(),
        "discard must not create or rewrite config.toml"
    );

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    assert!(
        api::learning_proposals::list(&mut ctx)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn apply_refuses_an_unknown_id_a_second_apply_and_a_discarded_run() {
    let home = TempDir::new().expect("tempdir");
    let (paths, mut conn) = open(&home);
    seed_run(
        &conn,
        "once",
        "tune",
        "2026-09-01T10:00:00.000000000Z",
        &shifted(),
    );
    seed_run(
        &conn,
        "dropped",
        "tune",
        "2026-09-01T09:00:00.000000000Z",
        &shifted(),
    );
    eval_runs::set_discarded(&conn, "dropped").expect("discard");

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::learning_proposals::apply(&mut ctx, "nosuchrun").expect_err("unknown id");
    assert!(matches!(err, Error::NotFound(_)), "got {err:?}");

    api::learning_proposals::apply(&mut ctx, "once").expect("first apply");
    let err = api::learning_proposals::apply(&mut ctx, "once").expect_err("second apply");
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");

    let err = api::learning_proposals::apply(&mut ctx, "dropped").expect_err("discarded run");
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");

    let err = api::learning_proposals::discard(&mut ctx, "once").expect_err("applied run");
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
}

#[test]
fn a_real_tune_run_becomes_a_listed_proposal_ac13() {
    let home = TempDir::new().expect("tempdir");
    let (paths, mut conn) = open(&home);

    // A one-point `[tune]` grid that differs from the live knobs on rrf_k
    // alone: the winner is then deterministic (no dependence on which
    // candidate happens to score best on a small corpus) and the diff is
    // exactly one entry.
    let mut cfg = Config::defaults();
    cfg.tune.rrf_k_grid = vec![25.0];
    cfg.tune.decay_grid = vec![cfg.rank.decay];
    cfg.tune.mmr_lambda_grid = vec![cfg.rank.mmr_lambda];
    cfg.tune.bm25_grid = vec![cfg.retrieval.bm25_weights];
    cfg.tune.graph_hops_grid = vec![cfg.retrieval.graph_hops];
    cfg.tune.graph_seeds_grid = vec![cfg.retrieval.graph_seeds];
    cfg.tune.samples = 0;

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    // `tune` refuses fewer than MIN_GOLDEN_PAIRS (10) pairs, so the corpus
    // is ten real memories, each its own golden query.
    let mut pairs: Vec<String> = Vec::new();
    for n in 0..10 {
        let body = format!("tuning corpus memory number {n} about sqlite retrieval");
        let id = save_note(&mut ctx, &body);
        pairs.push(format!("- query: {body}\n  relevant: [{id}]\n"));
    }
    let golden_file = home.path().join("golden.yaml");
    std::fs::write(&golden_file, pairs.concat()).expect("write golden file");

    let report = api::tune::run(
        &mut ctx,
        api::tune::Request {
            golden: Some(golden_file.to_string_lossy().into_owned()),
            golden_only: true,
            k: 3,
            apply: false,
            seed: None,
        },
    )
    .expect("tune run");
    assert!(!report.applied, "apply:false never rewrites config.toml");

    let proposals = api::learning_proposals::list(&mut ctx).expect("list");
    assert_eq!(proposals.len(), 1, "the tune run is on offer");
    assert_eq!(proposals[0].kind, "tune");
    assert_eq!(proposals[0].golden_pairs, 10);
    let names: Vec<&str> = proposals[0].knobs.iter().map(|k| k.name).collect();
    assert_eq!(names, vec!["rrf_k"]);
    assert_eq!(proposals[0].knobs[0].from.as_f64(), Some(60.0));
    assert_eq!(proposals[0].knobs[0].to.as_f64(), Some(25.0));
    assert_ne!(
        proposals[0].knobs[0].from, proposals[0].knobs[0].to,
        "AC-13: at least one knob differs"
    );
}

/// Save one note through the real command core and return its id.
fn save_note(ctx: &mut Ctx<'_>, body: &str) -> String {
    let req = api::save::Request {
        body: body.to_string(),
        title: None,
        kind: comemory::memory::Kind::Note,
        repo: "comemory".to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    api::save::run(ctx, req, false, None).expect("seed save").id
}

#[test]
fn a_run_whose_knobs_are_not_a_full_knob_set_is_skipped_not_fatal() {
    let home = TempDir::new().expect("tempdir");
    let (paths, mut conn) = open(&home);
    eval_runs::insert(
        &conn,
        &eval_runs::NewRun {
            id: "legacy",
            kind: "tune",
            at: "2026-09-01T10:00:00.000000000Z",
            golden_pairs: 12,
            k: 3,
            recall: 0.6,
            mrr: 0.4,
            knobs: "{\"rrf_k\":30.0}",
            applied: false,
        },
    )
    .expect("insert partial-knobs row");
    seed_run(
        &conn,
        "modern",
        "tune",
        "2026-09-01T09:00:00.000000000Z",
        &shifted(),
    );

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let proposals = api::learning_proposals::list(&mut ctx).expect("list");
    let ids: Vec<&str> = proposals.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["modern"],
        "the unparsable row is skipped, not fatal"
    );

    let err = api::learning_proposals::apply(&mut ctx, "legacy").expect_err("apply legacy");
    assert!(
        matches!(err, Error::BadRequest(_)),
        "applying it names the problem instead of skipping it: {err:?}"
    );
}
