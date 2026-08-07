#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/unindex.rs`. Registers a real docs fixture
//! source via `comemory index`, then calls `api::unindex::run` directly
//! against a `Ctx` opened on the same data-dir (`cli::unindex::run` is
//! byte-compat tested against CLI stdout in `tests/cli__unindex.rs`; the
//! HTTP route lives in `tests/serve__routes__sources.rs`).

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use serde::Deserialize;
use tempfile::TempDir;

#[derive(Deserialize)]
struct IndexOutput {
    sources: Vec<IndexedSource>,
}

#[derive(Deserialize)]
struct IndexedSource {
    source_id: String,
}

/// Register one docs fixture source under `home`'s data dir via the real
/// binary; returns the registered source id and the workspace tempdir kept
/// alive for the duration of the test.
fn seeded_home() -> (TempDir, TempDir, String) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "index",
            docs.to_str().expect("utf8 path"),
            "--repo",
            "docs-corpus",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: IndexOutput = serde_json::from_slice(&out).expect("parse index --json");
    let source_id = report.sources[0].source_id.clone();
    (home, workspace, source_id)
}

#[test]
fn run_unregisters_by_source_id_and_removes_documents() {
    let (home, _workspace, source_id) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::unindex::run(
        &mut ctx,
        api::unindex::Request {
            target: source_id.clone(),
        },
    )
    .expect("unindex run");
    assert_eq!(resp.source_id, source_id);
    assert_eq!(resp.documents_removed, docs_fixtures::FIXTURE_COUNT);

    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
        .expect("count documents");
    assert_eq!(remaining, 0, "unindex must delete every document row");
}

#[test]
fn run_errors_on_an_unregistered_target() {
    let (home, _workspace, _source_id) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::unindex::run(
        &mut ctx,
        api::unindex::Request {
            target: "no-such-source".to_string(),
        },
    )
    .expect_err("unregistered target must error");
    assert!(err.to_string().contains("no registered source"));
}

#[test]
fn request_rejects_unknown_fields() {
    let err = serde_json::from_value::<api::unindex::Request>(serde_json::json!({
        "target": "abc",
        "bogus": 1,
    }))
    .expect_err("unknown field must be rejected");
    assert!(err.to_string().contains("unknown field"));
}
