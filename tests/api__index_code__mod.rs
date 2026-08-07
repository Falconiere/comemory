//! Mirror test for `src/api/index_code/mod.rs`. Real temp git repos built
//! via the shared `common/git_*` helpers; `api::index_code::run` is called
//! directly against a `Ctx::borrowed` connection (no CLI process spawned).
//! `cli::index_code::run` stays byte-compat tested against CLI stdout in
//! `tests/cli__index_code.rs` / `tests/cli__index_code_2.rs`; the HTTP job
//! route (`POST /api/v1/code/index`) lives in `tests/serve__routes__code.rs`.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/git_sample.rs"]
mod git_sample;

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use tempfile::tempdir;

fn ctx_over(home: &std::path::Path) -> (Paths, Config, rusqlite::Connection) {
    let paths = Paths::new(home);
    paths.ensure_dirs().expect("ensure dirs");
    let conn = connection::open(paths.db_path()).expect("open db");
    (paths, Config::defaults(), conn)
}

#[test]
fn run_indexes_symbols_and_reports_files_indexed() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = api::index_code::run(
        &mut ctx,
        api::index_code::Request {
            repo: "sample".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
        },
    )
    .expect("index_code run");
    assert_eq!(resp.repo, "sample");
    assert_eq!(resp.files_indexed, 1, "single src.rs file counted once");

    let symbols: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo = 'sample'",
            [],
            |r| r.get(0),
        )
        .expect("count code_symbols");
    assert!(
        symbols >= 2,
        "main + helper symbols expected, got {symbols}"
    );
}

#[test]
fn run_second_run_reports_zero_files_indexed_when_unchanged() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());

    let req = || api::index_code::Request {
        repo: "sample".into(),
        path: repo.to_str().expect("utf8 path").to_string(),
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let first = api::index_code::run(&mut ctx, req()).expect("first run");
        assert_eq!(first.files_indexed, 1);
    }
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let second = api::index_code::run(&mut ctx, req()).expect("second run");
    assert_eq!(
        second.files_indexed, 0,
        "unchanged blob OID must skip re-indexing"
    );
}

#[test]
fn run_on_a_non_git_directory_errors() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let not_a_repo = workspace.path().join("plain-dir");
    std::fs::create_dir_all(&not_a_repo).expect("mkdir");
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let err = api::index_code::run(
        &mut ctx,
        api::index_code::Request {
            repo: "x".into(),
            path: not_a_repo.to_str().expect("utf8 path").to_string(),
        },
    )
    .expect_err("must error on a non-git path");
    assert!(!err.to_string().is_empty());
}

#[test]
fn run_on_a_non_git_directory_never_creates_the_db() {
    // `Ctx::lazy` mirrors `cli::index_code::run`'s wiring: the connection
    // opens only on first `ctx.conn()` use. Since `run` validates the git
    // repo (`Repository::open`) before its own `ctx.conn()` call, an
    // invalid `--path` must fail with zero `comemory.db` side effects —
    // matching `main`'s original ordering (git validation before DB open).
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let not_a_repo = workspace.path().join("plain-dir");
    std::fs::create_dir_all(&not_a_repo).expect("mkdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let db_path = paths.db_path();
    assert!(!db_path.exists(), "db must not exist yet");

    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let err = api::index_code::run(
        &mut ctx,
        api::index_code::Request {
            repo: "x".into(),
            path: not_a_repo.to_str().expect("utf8 path").to_string(),
        },
    )
    .expect_err("must error on a non-git path");
    assert!(!err.to_string().is_empty());
    assert!(
        !db_path.exists(),
        "index-code on an invalid --path must not create comemory.db"
    );
}
