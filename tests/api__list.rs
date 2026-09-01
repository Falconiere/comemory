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

/// Save a memory through the real binary with an explicit `--quality`
/// rating, for the `--sort quality` test.
fn save_quality(home: &tempfile::TempDir, body: &str, quality: u8) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", body, "--quality", &quality.to_string()])
        .assert()
        .success();
}

/// Run `comemory search <query>` through the real binary so its access
/// tracking bumps `access_count` / `last_accessed` on every returned hit —
/// the only way to exercise the `--sort accessed` ordering with real data.
fn search(home: &tempfile::TempDir, query: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["search", query])
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

/// A no-op-filter, default-limit, default-sort request.
fn request() -> api::list::Request {
    api::list::Request {
        repo: None,
        kind: None,
        tag: None,
        min_quality: None,
        q: None,
        limit: 50,
        offset: 0,
        sort: api::list::Sort::Created,
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
        tag: None,
        min_quality: None,
        q: None,
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

/// AC-11: rows carry `title` (first non-empty trimmed body line), `tags`,
/// `quality`, `created`, `access_count` — plus the legacy `id`/`kind`/
/// `repo`/`slug` fields, unchanged.
#[test]
fn run_rows_carry_new_fields_and_keep_legacy_fields() {
    let home = tempfile::tempdir().expect("tempdir");
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args([
            "save",
            "Title line here\n\nRest of the body goes on a second paragraph.",
            "--kind",
            "decision",
            "--repo",
            "gamma",
            "--tags",
            "alpha,beta",
            "--quality",
            "5",
        ])
        .assert()
        .success();

    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let page = api::list::run(&mut ctx, request()).expect("list run");
    assert_eq!(page.items.len(), 1);
    let row = &page.items[0];
    assert!(!row.id.is_empty(), "id must still be populated");
    assert_eq!(row.kind, "decision");
    assert_eq!(row.repo, "gamma");
    assert!(!row.slug.is_empty(), "slug must still be populated");
    assert_eq!(row.title, "Title line here");
    assert_eq!(row.tags, vec!["alpha".to_string(), "beta".to_string()]);
    assert_eq!(row.quality, 5);
    assert!(!row.created.is_empty(), "created must be populated");
    assert_eq!(row.access_count, 0);
}

/// AC-12: `sort: Created` (the default) stays newest-created-first.
#[test]
fn run_default_sort_is_newest_created_first() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let page = api::list::run(&mut ctx, request()).expect("list run");
    // `seeded_home` saves "alpha decision one", "alpha bug two", then
    // "beta decision three" in that order via sequential real `save` runs.
    assert_eq!(page.items[0].title, "beta decision three");
    assert_eq!(page.items[2].title, "alpha decision one");
}

/// AC-12: `sort: Quality` orders rows by descending quality.
#[test]
fn run_sort_quality_orders_descending() {
    let home = tempfile::tempdir().expect("tempdir");
    save_quality(&home, "middling note", 3);
    save_quality(&home, "top note", 5);
    save_quality(&home, "weak note", 1);

    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::list::Request {
        sort: api::list::Sort::Quality,
        ..request()
    };
    let page = api::list::run(&mut ctx, req).expect("list run");
    let qualities: Vec<u8> = page.items.iter().map(|r| r.quality).collect();
    assert_eq!(qualities, vec![5, 3, 1]);
}

/// AC-12: `sort: Accessed` puts the most-recently-searched memory first.
/// Bodies use disjoint vocabulary and distinct repos so a real
/// `comemory search` for a term unique to one body hits that memory alone
/// (the strict lexical tier short-circuits) rather than every memory.
#[test]
fn run_sort_accessed_puts_most_recently_searched_first() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "keyboard maintenance checklist", "note", "repo-a");
    save(&home, "umbrella storage guidelines", "note", "repo-b");
    save(&home, "lighthouse inspection log", "note", "repo-c");

    search(&home, "keyboard");

    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::list::Request {
        sort: api::list::Sort::Accessed,
        ..request()
    };
    let page = api::list::run(&mut ctx, req).expect("list run");
    assert!(
        page.items[0].title.starts_with("keyboard"),
        "most-recently-accessed row must lead: {:?}",
        page.items.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
    assert_eq!(page.items[0].access_count, 1);
}
