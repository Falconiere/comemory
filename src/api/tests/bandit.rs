#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/bandit.rs`. Seeds a real corpus + golden file
//! via the binary, then calls `api::bandit::run` directly against a `Ctx`
//! opened on the same data-dir (the HTTP route — job, confirm-when-apply,
//! `AppState.cfg` reload — lives in `src/serve/routes/tests/learning.rs`).

use crate::test_common::cli_eval_support as support;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use support::{TOPICS, corpus_with_golden};
use tempfile::TempDir;

fn data_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory")
}

#[test]
fn run_without_apply_reports_and_never_writes_config() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let paths = Paths::new(data_dir(&home));
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::bandit::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        apply: false,
    };
    let report = api::bandit::run(&mut ctx, req).expect("bandit run");
    assert!(!report.applied);
    assert_eq!(report.golden_pairs, TOPICS.len());
    assert!(
        !paths.config_file().exists(),
        "apply:false must never write config.toml"
    );
}

#[test]
fn run_still_upserts_bandit_arms_when_apply_is_false() {
    // §Route map Notes: `bandit` is unconditionally mutating — even a
    // report-only run seeds/updates `bandit_arms`.
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let paths = Paths::new(data_dir(&home));
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::bandit::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        apply: false,
    };
    api::bandit::run(&mut ctx, req).expect("bandit run");

    let arm_rows: i64 = conn
        .query_row("SELECT count(*) FROM bandit_arms", [], |r| r.get(0))
        .expect("count bandit_arms");
    assert!(
        arm_rows > 0,
        "a report-only bandit run must still seed bandit_arms"
    );
}

#[test]
fn run_writes_one_eval_runs_row_with_a_real_recall_and_mrr() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let paths = Paths::new(data_dir(&home));
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::bandit::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        apply: false,
    };
    let report = api::bandit::run(&mut ctx, req).expect("bandit run");

    let rows = comemory::store::eval_runs::list(&conn, 10).expect("list eval_runs");
    assert_eq!(rows.len(), 1, "one row per run, never per scored candidate");
    let row = &rows[0];
    assert_eq!(row.kind, "bandit");
    assert_eq!(row.golden_pairs, report.golden_pairs as u64);
    assert!(
        (0.0..=1.0).contains(&row.recall),
        "recall must be a real recall@k fraction, got {}",
        row.recall
    );
    assert!(
        (0.0..=1.0).contains(&row.mrr),
        "mrr must be a real reciprocal-rank mean, got {}",
        row.mrr
    );
    assert!(!row.applied, "apply:false must record applied == 0");
    assert_eq!(
        row.knobs["rrf_k"].as_f64(),
        Some(f64::from(report.proposed.rrf_k)),
        "knobs JSON must carry the proposed/confirmed arm"
    );
}

#[test]
fn run_apply_refused_when_bandit_disabled_in_config() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let paths = Paths::new(data_dir(&home));
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let config = home.path().join(".comemory").join("config.toml");
    std::fs::write(&config, "[bandit]\nenabled = false\n").expect("write config.toml");
    let cfg = Config::defaults()
        .with_file(config.as_path())
        .expect("load config with bandit disabled");
    assert!(!cfg.bandit.enabled);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::bandit::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        apply: true,
    };
    let err = api::bandit::run(&mut ctx, req).expect_err("apply must be refused");
    assert!(
        matches!(err, Error::Config(_)),
        "expected Error::Config, got {err:?}"
    );
}

#[test]
fn golden_file_merge_produces_the_file_pairs_in_the_report() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let paths = Paths::new(data_dir(&home));
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::bandit::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: false,
        k: 3,
        apply: false,
    };
    let report = api::bandit::run(&mut ctx, req).expect("bandit run");
    assert_eq!(report.golden_pairs, TOPICS.len());
}
