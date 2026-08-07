#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/list.rs`. Seeds real memories via the
//! `comemory` binary (markdown + SQLite mirror), then calls
//! `api::list::run` directly against a `Ctx` opened on the same data-dir —
//! proving the extracted command core reproduces `comemory list`'s
//! filtering and paging (`cli::list::run` is byte-compat tested against
//! the CLI stdout in `tests/cli__list.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;

/// Save a memory through the real binary so both the markdown file and the
/// SQLite mirror row exist for `api::list::run` to read.
fn save(home: &tempfile::TempDir, body: &str, kind: &str, repo: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", body, "--kind", kind, "--repo", repo])
        .assert()
        .success();
}

/// Three memories across two repos and two kinds.
fn seeded_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "alpha decision one", "decision", "alpha");
    save(&home, "alpha bug two", "bug", "alpha");
    save(&home, "beta decision three", "decision", "beta");
    home
}

/// A no-op-filter, default-limit request.
fn request() -> api::list::Request {
    api::list::Request {
        repo: None,
        kind: None,
        limit: 50,
        offset: 0,
    }
}

#[test]
fn run_lists_every_live_memory() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let page = api::list::run(&mut ctx, request()).expect("list run");
    assert_eq!(page.total, Some(3));
    assert_eq!(page.items.len(), 3);
    assert!(!page.has_more);
    for item in &page.items {
        assert!(!item.id.is_empty());
        assert!(!item.slug.is_empty());
    }
}

#[test]
fn run_applies_repo_and_kind_filters() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::list::Request {
        repo: Some("alpha".to_string()),
        kind: Some("Decision".to_string()),
        ..request()
    };
    let page = api::list::run(&mut ctx, req).expect("list run");
    assert_eq!(page.total, Some(1));
    assert_eq!(page.items[0].repo, "alpha");
    assert_eq!(page.items[0].kind, "decision");
}

#[test]
fn run_pages_with_limit_and_offset() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::list::Request {
        limit: 2,
        offset: 0,
        ..request()
    };
    let page = api::list::run(&mut ctx, req).expect("list run");
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, Some(3));
    assert!(page.has_more);
}
