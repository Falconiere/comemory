#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/repos.rs` (+ `repos/git_state.rs`). Real git
//! repos indexed through `api::index_code::run`, real memory rows, real
//! freshness transitions — no mocks (Binding Rule 9). `comemory repos`'s
//! subprocess-level behavior (AC-3, AC-4, AC-4b, AC-5) is covered in
//! `tests/cli__repos.rs`; this file exercises `api::repos::run`'s SQL join
//! and the must-not-create-the-db invariant directly.

use crate::test_common::git_commit;
use crate::test_common::git_sample;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::TempDir;

fn ctx_over(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

fn seed_memory(conn: &rusqlite::Connection, id: &str, repo: &str) {
    conn.execute(
        "INSERT INTO memories(id, slug, kind, repo, body, created_at, updated_at,
                              content_hash, schema, md_path)
         VALUES (?1, ?2, 'decision', ?3, 'body', '2026-08-01T00:00:00Z',
                 '2026-08-01T00:00:00Z', ?1, 1, ?4)",
        rusqlite::params![
            id,
            format!("{id}-slug"),
            repo,
            format!("memories/{id}-slug.md"),
        ],
    )
    .expect("seed memory");
}

fn index_sample(ctx: &mut Ctx<'_>, repo: &str, path: &std::path::Path) {
    api::index_code::run(
        ctx,
        api::index_code::Request {
            repo: repo.into(),
            path: path.to_str().expect("utf8 path").to_string(),
            mode: comemory::api::index_code::IndexMode::Incremental,
        },
    )
    .expect("index_code run");
}

#[test]
fn a_data_dir_without_a_database_reports_an_empty_inventory_and_creates_nothing() {
    let home = TempDir::new().expect("home");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let resp = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos run");

    assert!(resp.repos.is_empty());
    assert!(
        !paths.db_path().exists(),
        "asking for repos must not create and migrate a database"
    );
}

#[test]
fn run_joins_file_symbol_and_memory_counters_and_reports_fresh() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo_path = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    seed_memory(&conn, "11111111", "sample");
    seed_memory(&conn, "22222222", "sample");
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index_sample(&mut ctx, "sample", &repo_path);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos run");

    assert_eq!(resp.repos.len(), 1);
    let row = &resp.repos[0];
    assert_eq!(row.repo, "sample");
    assert_eq!(row.files, 1);
    assert!(
        row.symbols >= 2,
        "main + helper symbols expected, got {}",
        row.symbols
    );
    assert_eq!(row.memories, 2);
    assert_eq!(row.status, "fresh");
    assert!(row.root_path.is_some());
    assert!(row.last_head.is_some());
    assert!(row.last_indexed_at.is_some());
    assert_eq!(row.changed_files, None);
}

#[test]
fn repo_filter_narrows_to_one_marker_row() {
    let home = TempDir::new().expect("home");
    let ws_a = TempDir::new().expect("workspace a");
    let ws_b = TempDir::new().expect("workspace b");
    let repo_a = git_sample::build_sample_repo(ws_a.path());
    let repo_b = git_sample::build_sample_repo(ws_b.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index_sample(&mut ctx, "repo-a", &repo_a);
        index_sample(&mut ctx, "repo-b", &repo_b);
    }

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::repos::run(
        &mut ctx,
        api::repos::Request {
            repo: Some("repo-a".into()),
        },
    )
    .expect("repos run");

    assert_eq!(resp.repos.len(), 1);
    assert_eq!(resp.repos[0].repo, "repo-a");
}

#[test]
fn a_commit_since_the_last_index_flips_status_to_stale_with_a_changed_file_count() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo_path = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index_sample(&mut ctx, "sample", &repo_path);
    }

    git_commit::commit_files(&repo_path, &[("extra.rs", "fn extra() {}\n")], "add extra");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos run");

    assert_eq!(resp.repos.len(), 1);
    let row = &resp.repos[0];
    assert_eq!(row.status, "stale");
    assert_eq!(row.changed_files, Some(1));
}

#[test]
fn rows_are_ordered_by_repo_label() {
    let home = TempDir::new().expect("home");
    let ws_b = TempDir::new().expect("workspace b");
    let ws_a = TempDir::new().expect("workspace a");
    let repo_b = git_sample::build_sample_repo(ws_b.path());
    let repo_a = git_sample::build_sample_repo(ws_a.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    index_sample(&mut ctx, "zzz-repo", &repo_b);
    index_sample(&mut ctx, "aaa-repo", &repo_a);

    let resp = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos run");

    let labels: Vec<&str> = resp.repos.iter().map(|r| r.repo.as_str()).collect();
    assert_eq!(labels, vec!["aaa-repo", "zzz-repo"]);
}
