//! Mirror test for `src/api/index.rs`. Real document fixtures (`common
//! /docs_fixtures.rs`, matching `tests/api__sources.rs`); `api::index::run`
//! is called directly against a `Ctx::borrowed` connection. `cli::index::run`
//! stays byte-compat tested against CLI stdout in `tests/cli__index.rs`;
//! the HTTP job route (`POST /api/v1/sources`) lives in
//! `tests/serve__routes__sources.rs`.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::TempDir;

fn ctx_over(home: &TempDir) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

#[test]
fn run_registers_and_indexes_real_fixtures() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let output = api::index::run(
        &mut ctx,
        api::index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: Some("docs-corpus".into()),
            strict: false,
        },
    )
    .expect("index run");
    assert_eq!(output.sources.len(), 1);
    let s = &output.sources[0];
    assert_eq!(s.indexed, docs_fixtures::FIXTURE_COUNT);
    assert_eq!(s.too_large, 0);
    assert!(s.errors.is_empty());
    assert_eq!(s.repo.as_deref(), Some("docs-corpus"));
}

#[test]
fn run_never_fails_on_strict_leaving_the_error_check_to_the_caller() {
    // Regression for the module doc's "strict decision": a too-large file
    // must still surface in `Output.sources[].errors`, but `run` itself
    // must return `Ok` even though `req.strict` is set — the CLI (not this
    // module) is responsible for turning that into an `Error::Document`.
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let output = api::index::run(
        &mut ctx,
        api::index::Request {
            path: vec![docs.to_str().expect("utf8 path").to_string()],
            repo: None,
            strict: true,
        },
    )
    .expect("run must succeed even though strict is set and files are unremarkable");
    let s = &output.sources[0];
    assert_eq!(s.indexed, docs_fixtures::FIXTURE_COUNT);
    assert!(s.errors.is_empty());
}

#[test]
fn run_on_a_nonexistent_path_is_a_usage_error() {
    let home = TempDir::new().expect("home");
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::index::run(
        &mut ctx,
        api::index::Request {
            path: vec!["/nonexistent/does/not/exist".to_string()],
            repo: None,
            strict: false,
        },
    )
    .expect_err("nonexistent path must fail");
    assert!(err.to_string().contains("cannot resolve"), "{err}");
}
