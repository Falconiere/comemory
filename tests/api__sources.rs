//! Mirror test for `src/api/sources.rs`. Registers a real document source
//! via `comemory index`, then edits `sources.toml` out from under the
//! mirror to prove `Request::reconcile` gates the `source_roots` write:
//! `mirror::reconcile`'s only write is "delete every `source_roots` row
//! absent from `sources.toml`" (there is no live filesystem probe yet, so
//! this is the actual write side effect `reconcile:false` must skip).
//! `cli::sources::run` is byte-compat tested against CLI stdout in
//! `tests/cli__sources.rs`; the HTTP route lives in
//! `tests/serve__routes__sources.rs`.

#[path = "common/docs_fixtures.rs"]
mod docs_fixtures;

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::TempDir;

/// Register one real docs fixture directory as a source in `home`; returns
/// the fixture workspace kept alive for the duration of the test.
fn seeded_home() -> (TempDir, TempDir) {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let docs = docs_fixtures::seed(workspace.path());
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index", docs.to_str().unwrap(), "--repo", "docs-corpus"])
        .assert()
        .success();
    (home, workspace)
}

/// Drop every entry from `sources.toml`, simulating an out-of-band edit
/// that leaves the mirror row registered but the durable registry empty.
fn clear_registry(home: &TempDir) {
    std::fs::write(home.path().join("sources.toml"), "format = 1\n").expect("rewrite sources.toml");
}

#[test]
fn run_reconcile_true_lists_the_registered_source_with_counts() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let rows = api::sources::run(&mut ctx, api::sources::Request { reconcile: true })
        .expect("sources run");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].repo.as_deref(), Some("docs-corpus"));
    assert_eq!(rows[0].indexed, docs_fixtures::FIXTURE_COUNT);
}

#[test]
fn run_reconcile_false_skips_the_mirror_delete() {
    let (home, _workspace) = seeded_home();
    clear_registry(&home);

    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let rows = api::sources::run(&mut ctx, api::sources::Request { reconcile: false })
        .expect("sources run");
    assert_eq!(
        rows.len(),
        1,
        "reconcile:false must not delete the now-unregistered mirror row"
    );
}

#[test]
fn run_reconcile_true_deletes_a_row_dropped_from_the_toml() {
    let (home, _workspace) = seeded_home();
    clear_registry(&home);

    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let rows = api::sources::run(&mut ctx, api::sources::Request { reconcile: true })
        .expect("sources run");
    assert!(
        rows.is_empty(),
        "reconcile:true must delete the mirror row absent from sources.toml"
    );
}
