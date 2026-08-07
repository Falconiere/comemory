//! Mirror test for `src/api/eval.rs`. Seeds a real corpus + golden file via
//! the binary, then calls `api::eval::run` directly against a `Ctx` opened
//! on the same data-dir (the HTTP route — `POST /eval`, job, read-only
//! containment — lives in `tests/serve__routes__learning.rs`).

#[path = "common/cli_eval_support.rs"]
mod support;

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
    };
    let report = api::eval::run(&mut ctx, req).expect("eval run");
    assert_eq!(report.queries, 5);
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
    };
    let err = api::eval::run(&mut ctx, req).expect_err("no golden pairs at all");
    assert!(
        matches!(err, Error::Unavailable(_)),
        "expected Unavailable, got {err:?}"
    );
}
