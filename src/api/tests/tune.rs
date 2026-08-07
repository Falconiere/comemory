#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/tune.rs`. Seeds a real corpus + golden file via
//! the binary, then calls `api::tune::run` directly against a `Ctx` opened
//! on the same data-dir (the HTTP route — job, confirm-when-apply,
//! `AppState.cfg` reload — lives in `src/serve/routes/tests/learning.rs`).

use crate::test_common::cli_eval_support as support;
use std::fmt::Write as _;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use support::{TOPICS, corpus_with_golden};
use tempfile::TempDir;

fn data_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory")
}

#[test]
fn run_without_apply_reports_and_writes_nothing() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());

    let paths = Paths::new(data_dir(&home));
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::tune::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        apply: false,
        seed: Some(42),
    };
    let resp = api::tune::run(&mut ctx, req).expect("tune run");
    assert!(!resp.applied, "apply:false must never write config.toml");
    assert_eq!(resp.report.golden_pairs, TOPICS.len());
    assert!(!paths.config_file().exists());
}

#[test]
fn run_is_deterministic_for_a_pinned_seed() {
    let home = TempDir::new().expect("tempdir");
    let golden = corpus_with_golden(&home, TOPICS.len());
    let paths = Paths::new(data_dir(&home));
    let cfg = Config::defaults();

    let run_once = |paths: &Paths, cfg: &Config, golden: &std::path::Path| {
        let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
        let mut ctx = Ctx::borrowed(paths, cfg, &mut conn);
        let req = api::tune::Request {
            golden: Some(golden.to_string_lossy().into_owned()),
            golden_only: true,
            k: 3,
            apply: false,
            seed: Some(42),
        };
        api::tune::run(&mut ctx, req).expect("tune run")
    };

    let first = run_once(&paths, &cfg, &golden);
    let second = run_once(&paths, &cfg, &golden);
    assert_eq!(
        serde_json::to_string(&first.report).expect("serialize"),
        serde_json::to_string(&second.report).expect("serialize"),
        "two --seed 42 runs must score the identical candidate set"
    );
}

/// The [`TOPICS`] corpus (each an exact-body golden pair, satisfying the
/// default 10-pair min-golden floor without needing the
/// `COMEMORY_TUNE_MIN_GOLDEN` test hook) plus one tag-discriminated pair: a
/// `postgres`-tagged memory whose `postgres` query only resolves once the
/// grid's default `(1.0, 3.0)` bm25 weights out-beat a deliberately
/// zeroed-out tags column in the starting `config.toml`.
fn tag_discriminated_fixture(home: &TempDir) -> std::path::PathBuf {
    let golden = corpus_with_golden(home, TOPICS.len());
    let tagged = support::run_json(
        home,
        &[
            "save",
            "kubernetes ingress certificate renewal notes",
            "--kind",
            "note",
            "--tags",
            "postgres",
        ],
    )["id"]
        .as_str()
        .expect("id")
        .to_string();
    let mut yaml = std::fs::read_to_string(&golden).expect("read golden file");
    let _ = writeln!(yaml, "- query: postgres\n  relevant: [{tagged}]");
    std::fs::write(&golden, yaml).expect("append tag-discriminated pair");
    let config = home.path().join(".comemory").join("config.toml");
    std::fs::write(&config, "[retrieval]\nbm25_weights = [1.0, 0.0]\n")
        .expect("write starting config.toml");
    golden
}

#[test]
fn run_apply_writes_config_only_when_the_winner_strictly_beats_baseline() {
    let home = TempDir::new().expect("tempdir");
    let golden = tag_discriminated_fixture(&home);

    let paths = Paths::new(data_dir(&home));
    let cfg = Config::defaults()
        .with_file(paths.config_file().as_path())
        .expect("load starting config");
    let mut conn = comemory::store::connection::open(paths.db_path()).expect("open db");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let req = api::tune::Request {
        golden: Some(golden.to_string_lossy().into_owned()),
        golden_only: true,
        k: 3,
        apply: true,
        seed: Some(42),
    };
    let resp = api::tune::run(&mut ctx, req).expect("tune run");

    assert!(
        resp.applied,
        "the zero-weight tags column must be beatable by the default grid"
    );
    let raw = std::fs::read_to_string(paths.config_file()).expect("read applied config.toml");
    assert!(
        raw.contains("[retrieval]") && raw.contains("bm25_weights"),
        "applied config.toml must carry the winner's bm25_weights, got: {raw:?}"
    );
    assert_ne!(
        raw.replace([' ', '\n'], ""),
        "[retrieval]bm25_weights=[1.0,0.0]".to_string(),
        "the deliberately-bad starting weights must have been rewritten"
    );
}
