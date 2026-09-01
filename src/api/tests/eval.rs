#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/eval.rs`. Seeds a real corpus + golden file via
//! the binary, then calls `api::eval::run` directly against a `Ctx` opened
//! on the same data-dir (the HTTP route — `POST /eval`, job, read-only
//! containment — lives in `src/serve/routes/tests/learning.rs`).

use crate::test_common::cli_eval_support as support;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::store::connection;
use support::corpus_with_golden;
use tempfile::TempDir;

fn data_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory")
}

#[test]
fn run_scores_the_real_pipeline_against_the_golden_file() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 10);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::eval::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        history: false,
        limit: 20,
        knobs: None,
    };
    let report = api::eval::run(&mut ctx, req).expect("eval run");
    assert_eq!(report.queries, 10);
    assert_eq!(report.k, 3);
    // Every golden query's relevant id is the query body itself: a
    // healthy lexical pipeline should recover most of them.
    assert!(
        report.recall_at_k > 0.5,
        "expected strong recall on an exact-body golden set, got {}",
        report.recall_at_k
    );
}

#[test]
fn golden_only_false_merges_an_empty_feedback_harvest_without_erroring() {
    // The db has no feedback rows yet, so the harvest leg contributes
    // nothing — this proves `golden::resolve` walks that path cleanly
    // (rather than only ever being exercised with `golden_only: true`).
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 5);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::eval::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: false,
        k: 3,
        history: false,
        limit: 20,
        knobs: None,
    };
    let report = api::eval::run(&mut ctx, req).expect("eval run");
    assert_eq!(report.queries, 5);
}

#[test]
fn run_writes_one_eval_runs_row_matching_the_report() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 10);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::eval::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        history: false,
        limit: 20,
        knobs: None,
    };
    let report = api::eval::run(&mut ctx, req).expect("eval run");

    let rows = comemory::store::eval_runs::list(&conn, 10).expect("list eval_runs");
    assert_eq!(rows.len(), 1, "one row per run, never per candidate");
    let row = &rows[0];
    assert_eq!(row.kind, "eval");
    assert_eq!(row.golden_pairs, report.queries as u64);
    assert_eq!(row.k, report.k as u64);
    assert_eq!(
        row.recall, report.recall_at_k,
        "row must carry the real recall"
    );
    assert_eq!(row.mrr, report.mrr, "row must carry the real MRR");
    assert!(!row.applied, "plain eval never rewrites config.toml");
    assert!(row.knobs.is_object(), "knobs must be a JSON object");
}

#[test]
fn running_eval_twice_writes_two_rows_newest_first() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 10);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let req = || api::eval::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        history: false,
        limit: 20,
        knobs: None,
    };
    api::eval::run(&mut ctx, req()).expect("first eval run");
    api::eval::run(&mut ctx, req()).expect("second eval run");

    let history_req = api::eval::Request {
        history: true,
        ..req()
    };
    let rows = api::eval::history(&mut ctx, &history_req).expect("history read");
    assert_eq!(rows.len(), 2, "two runs must write two rows");
    assert!(
        rows[0].at >= rows[1].at,
        "history must return newest-first: {} then {}",
        rows[0].at,
        rows[1].at
    );
}

#[test]
fn history_on_an_empty_table_returns_an_empty_vec_not_an_error() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(data_dir(&home));
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let history_req = api::eval::Request {
        golden: None,
        golden_only: false,
        k: 3,
        history: true,
        limit: 20,
        knobs: None,
    };
    let rows = api::eval::history(&mut ctx, &history_req).expect("history read on empty table");
    assert!(rows.is_empty());
}

#[test]
fn history_limit_caps_the_row_count() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, 10);

    let paths = Paths::new(data_dir(&home));
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    for _ in 0..3 {
        let req = api::eval::Request {
            golden: Some(golden.to_string_lossy().into_owned()),
            golden_only: true,
            k: 3,
            history: false,
            limit: 20,
            knobs: None,
        };
        api::eval::run(&mut ctx, req).expect("eval run");
    }

    let history_req = api::eval::Request {
        golden: None,
        golden_only: false,
        k: 3,
        history: true,
        limit: 2,
        knobs: None,
    };
    let rows = api::eval::history(&mut ctx, &history_req).expect("history");
    assert_eq!(rows.len(), 2, "--limit must cap the returned rows");
}

#[test]
fn missing_golden_and_no_feedback_is_unavailable() {
    let home = TempDir::new().expect("tempdir");
    let paths = Paths::new(data_dir(&home));
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::eval::Request {
        golden: None,
        golden_only: false,
        k: 3,
        history: false,
        limit: 20,
        knobs: None,
    };
    let err = api::eval::run(&mut ctx, req).expect_err("no golden pairs at all");
    assert!(
        matches!(err, Error::Unavailable(_)),
        "expected Unavailable, got {err:?}"
    );
}
