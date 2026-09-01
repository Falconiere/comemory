#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/index_runs.rs`. The history rows are written by
//! REAL `api::index_code` runs over real temp git repos (`common/git_*`) —
//! nothing seeds `index_runs` by hand, so a run that stops recording its
//! history fails here.

use crate::test_common::git_sample;

use comemory::api::index_code::IndexMode;
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

/// Index `repo` at `path` once, through the same core the CLI and the HTTP
/// job both call.
fn index(ctx: &mut Ctx<'_>, repo: &str, path: &std::path::Path, mode: IndexMode) {
    api::index_code::run(
        ctx,
        api::index_code::Request {
            repo: repo.into(),
            path: path.to_str().expect("utf8 path").to_string(),
            mode,
        },
    )
    .expect("index_code run");
}

#[test]
fn a_data_dir_without_a_database_reports_an_empty_page_and_creates_nothing() {
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let page = api::index_runs::run(&mut ctx, api::index_runs::Request::default())
        .expect("index_runs run");

    assert!(page.items.is_empty());
    assert_eq!(page.total, Some(0));
    assert!(!page.has_more);
    assert!(
        !paths.db_path().exists(),
        "a read must not create comemory.db"
    );
}

#[test]
fn a_real_index_run_shows_up_as_one_ok_row_with_files_indexed() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, "sample", &repo, IndexMode::Incremental);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let page = api::index_runs::run(&mut ctx, api::index_runs::Request::default())
        .expect("index_runs run");

    assert_eq!(page.items.len(), 1, "one run recorded");
    let row = &page.items[0];
    assert_eq!(row.repo, "sample");
    assert_eq!(row.outcome, "ok");
    assert_eq!(row.mode, "incremental");
    assert!(
        row.files_indexed > 0,
        "files_indexed: {}",
        row.files_indexed
    );
    assert!(row.symbols > 0, "symbols: {}", row.symbols);
    assert_eq!(page.total, Some(1));
    assert!(!page.has_more);
}

#[test]
fn the_repo_filter_narrows_the_history_to_one_label() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let first = git_sample::build_sample_repo(&workspace.path().join("one"));
    let second = git_sample::build_sample_repo(&workspace.path().join("two"));
    let (paths, cfg, mut conn) = ctx_over(&home);
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, "repo-a", &first, IndexMode::Incremental);
        index(&mut ctx, "repo-b", &second, IndexMode::Incremental);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let all = api::index_runs::run(&mut ctx, api::index_runs::Request::default())
        .expect("unfiltered run");
    assert_eq!(all.items.len(), 2);

    let scoped = api::index_runs::run(
        &mut ctx,
        api::index_runs::Request {
            repo: Some("repo-b".into()),
            ..api::index_runs::Request::default()
        },
    )
    .expect("filtered run");
    assert_eq!(scoped.items.len(), 1);
    assert_eq!(scoped.items[0].repo, "repo-b");
    assert_eq!(scoped.total, Some(1));
}

#[test]
fn the_window_pages_and_reports_has_more() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        // Three runs: the two later ones re-walk nothing, but each still
        // records its own history row.
        index(&mut ctx, "sample", &repo, IndexMode::Incremental);
        index(&mut ctx, "sample", &repo, IndexMode::Incremental);
        index(&mut ctx, "sample", &repo, IndexMode::Incremental);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let first = api::index_runs::run(
        &mut ctx,
        api::index_runs::Request {
            repo: None,
            limit: 2,
            offset: 0,
        },
    )
    .expect("first page");
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.total, Some(3));
    assert!(first.has_more);

    let last = api::index_runs::run(
        &mut ctx,
        api::index_runs::Request {
            repo: None,
            limit: 2,
            offset: 2,
        },
    )
    .expect("second page");
    assert_eq!(last.items.len(), 1);
    assert!(!last.has_more);
}
