#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/repo_admin.rs`: connect / patch / archive /
//! disconnect over REAL temp git repos indexed through `api::index_code`
//! and real memories saved through `api::save` — the disconnect case is
//! AC-18's "every code row goes, no memory row does".

use crate::test_common::git_sample;

use comemory::api::index_code::IndexMode;
use comemory::api::repo_admin::{ArchiveRequest, ConnectRequest, PatchRequest};
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::errors::Error;
use comemory::memory::Kind;
use comemory::store::connection;
use tempfile::TempDir;

fn ctx_over(home: &TempDir) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

fn as_str(path: &std::path::Path) -> String {
    path.to_str().expect("utf8 path").to_string()
}

fn canonical(path: &std::path::Path) -> String {
    as_str(&path.canonicalize().expect("canonicalize"))
}

fn connect_root(
    ctx: &mut Ctx<'_>,
    root: &std::path::Path,
    repo: Option<&str>,
) -> Result<comemory::api::repo_admin::ConnectResponse, Error> {
    api::repo_admin::connect(
        ctx,
        ConnectRequest {
            root: as_str(root),
            repo: repo.map(str::to_string),
            index_now: false,
        },
    )
}

fn index(ctx: &mut Ctx<'_>, repo: &str, path: &std::path::Path) {
    api::index_code::run(
        ctx,
        api::index_code::Request {
            repo: repo.into(),
            path: as_str(path),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run");
}

fn save_memory(ctx: &mut Ctx<'_>, body: &str, repo: &str) -> String {
    api::save::run(
        ctx,
        api::save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: repo.to_string(),
            tags: Vec::new(),
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .expect("save")
    .id
}

#[test]
fn connect_registers_the_root_and_defaults_the_label_to_the_basename() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = connect_root(&mut ctx, &repo, None).expect("connect");
    assert_eq!(resp.repo, "sample-repo");
    assert_eq!(resp.root_path, canonical(&repo));
    assert_eq!(resp.job_id, None, "the core never starts a job");

    let inventory = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos");
    assert_eq!(inventory.repos.len(), 1);
    assert_eq!(inventory.repos[0].repo, "sample-repo");
    assert!(!inventory.repos[0].archived);
}

#[test]
fn connect_is_idempotent_for_one_root_and_refuses_a_second() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let first = git_sample::build_sample_repo(&workspace.path().join("one"));
    let second = git_sample::build_sample_repo(&workspace.path().join("two"));
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    connect_root(&mut ctx, &first, Some("demo")).expect("first connect");
    connect_root(&mut ctx, &first, Some("demo")).expect("re-connecting the same root is a no-op");

    let err = connect_root(&mut ctx, &second, Some("demo")).expect_err("label collision");
    assert!(
        matches!(err, Error::BadRequest(ref m) if m.contains("already connected")),
        "unexpected error: {err}"
    );
}

#[test]
fn connect_refuses_a_root_that_does_not_exist() {
    let home = TempDir::new().expect("home");
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = connect_root(&mut ctx, &home.path().join("nope"), Some("demo"))
        .expect_err("nonexistent root");
    assert!(matches!(err, Error::BadRequest(_)), "error: {err}");
}

#[test]
fn patch_moves_the_root_and_refuses_every_unmodelled_field() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let first = git_sample::build_sample_repo(&workspace.path().join("one"));
    let second = git_sample::build_sample_repo(&workspace.path().join("two"));
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    connect_root(&mut ctx, &first, Some("demo")).expect("connect");

    let moved = api::repo_admin::patch(
        &mut ctx,
        "demo",
        PatchRequest {
            root: Some(as_str(&second)),
            ..PatchRequest::default()
        },
    )
    .expect("patch root");
    assert_eq!(
        moved.root_path.as_deref(),
        Some(canonical(&second).as_str())
    );

    let renamed = api::repo_admin::patch(
        &mut ctx,
        "demo",
        PatchRequest {
            name: Some("other".into()),
            ..PatchRequest::default()
        },
    )
    .expect_err("rename is unsupported");
    assert!(matches!(renamed, Error::Unsupported(_)), "error: {renamed}");

    let unknown = api::repo_admin::patch(&mut ctx, "ghost", PatchRequest::default())
        .expect_err("unknown repo");
    assert!(matches!(unknown, Error::NotFound(_)), "error: {unknown}");
}

#[test]
fn archive_flips_the_flag_and_repos_reports_the_archived_status() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    index(&mut ctx, "sample", &repo);

    let archived =
        api::repo_admin::archive(&mut ctx, "sample", ArchiveRequest::default()).expect("archive");
    assert!(archived.archived);
    let inventory = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos");
    assert_eq!(inventory.repos[0].status, "archived");
    assert!(inventory.repos[0].archived);

    let restored = api::repo_admin::archive(&mut ctx, "sample", ArchiveRequest { archived: false })
        .expect("un-archive");
    assert!(!restored.archived);
    let inventory = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos");
    assert_ne!(inventory.repos[0].status, "archived");

    let unknown = api::repo_admin::archive(&mut ctx, "ghost", ArchiveRequest::default())
        .expect_err("unknown repo");
    assert!(matches!(unknown, Error::NotFound(_)), "error: {unknown}");
}

#[test]
fn disconnect_drops_the_code_index_and_keeps_the_memories() {
    let home = TempDir::new().expect("home");
    let workspace = TempDir::new().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(&home);
    let memory_id = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index(&mut ctx, "sample", &repo);
        save_memory(&mut ctx, "a decision about the sample repo", "sample")
    };

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let dropped = api::repo_admin::disconnect(&mut ctx, "sample").expect("disconnect");
    assert_eq!(dropped.repo, "sample");
    assert!(dropped.symbols_removed > 0, "{dropped:?}");
    assert!(dropped.files_removed > 0, "{dropped:?}");

    let inventory = api::repos::run(&mut ctx, api::repos::Request::default()).expect("repos");
    assert!(inventory.repos.is_empty(), "the marker row goes too");

    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE id = ?1 AND deleted_at IS NULL",
            [&memory_id],
            |r| r.get(0),
        )
        .expect("count memories");
    assert_eq!(live, 1, "memories are retained (AC-18)");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let unknown = api::repo_admin::disconnect(&mut ctx, "sample").expect_err("already gone");
    assert!(matches!(unknown, Error::NotFound(_)), "error: {unknown}");
}
