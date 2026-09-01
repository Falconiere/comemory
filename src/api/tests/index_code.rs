#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/index_code.rs`. Real temp git repos built
//! via the shared `common/git_*` helpers; `api::index_code::run` is called
//! directly against a `Ctx::borrowed` connection (no CLI process spawned).
//! `cli::index_code::run` stays byte-compat tested against CLI stdout in
//! `tests/cli__index_code.rs` / `tests/cli__index_code_2.rs`; the HTTP job
//! route (`POST /api/v1/code/index`) lives in `tests/serve__routes__code.rs`.

use crate::test_common::git_sample;

use comemory::api::index_code::IndexMode;
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
            mode: comemory::api::index_code::IndexMode::Incremental,
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
        mode: comemory::api::index_code::IndexMode::Incremental,
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
fn full_mode_re_extracts_an_unchanged_file_and_drops_its_code_vectors() {
    // The documented cost of `--mode full`: it clears the indexed-file
    // cursor so every file is extracted again, and re-extraction replaces
    // the repo's symbol rows — taking the BYO `code_vec` rows with them.
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());

    let req = |mode| api::index_code::Request {
        repo: "sample".into(),
        path: repo.to_str().expect("utf8 path").to_string(),
        mode,
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let first = api::index_code::run(&mut ctx, req(IndexMode::Incremental)).expect("first run");
        assert_eq!(first.files_indexed, 1);
    }

    // A vector a caller embedded against one of those symbols, exactly as
    // `ingest-code` writes it.
    let symbol_id: i64 = conn
        .query_row(
            "SELECT id FROM code_symbols WHERE repo = 'sample' AND parent_id IS NULL \
             ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("a parent symbol to embed");
    let dim = comemory::store::vector::dim_code(&conn).expect("code dim");
    comemory::store::vector::insert_code(&conn, symbol_id, &vec![0.5; dim]).expect("insert vec");
    let vectors = |conn: &rusqlite::Connection| -> i64 {
        conn.query_row("SELECT COUNT(*) FROM code_vec", [], |r| r.get(0))
            .expect("count code_vec")
    };
    assert_eq!(vectors(&conn), 1, "the seeded vector is there to lose");

    let full = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::index_code::run(&mut ctx, req(IndexMode::Full)).expect("full run")
    };

    assert_eq!(
        full.files_indexed, 1,
        "full re-extracts the unchanged file that incremental skipped"
    );
    assert_eq!(full.mode, IndexMode::Full, "the response echoes the mode");
    assert_eq!(
        vectors(&conn),
        0,
        "re-extraction replaced the symbol rows, so the BYO vector is gone — \
         the loss `--mode full`'s help text warns about"
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
            mode: comemory::api::index_code::IndexMode::Incremental,
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
            mode: comemory::api::index_code::IndexMode::Incremental,
        },
    )
    .expect_err("must error on a non-git path");
    assert!(!err.to_string().is_empty());
    assert!(
        !db_path.exists(),
        "index-code on an invalid --path must not create comemory.db"
    );
}

/// A [`api::index_code::ProgressSink`] that just records every call, for
/// asserting the seam `serve::jobs::worker::RegistryProgressSink` uses in
/// production — proven end-to-end over real HTTP in
/// `tests/serve__jobs_progress.rs`; this proves the plain function contract.
#[derive(Default)]
struct RecordingSink {
    progress: std::sync::Mutex<Vec<(u64, u64)>>,
    logs: std::sync::Mutex<Vec<String>>,
}

impl api::index_code::ProgressSink for RecordingSink {
    fn on_progress(&self, done: u64, total: u64) {
        self.progress.lock().expect("lock").push((done, total));
    }

    fn on_log(&self, line: &str) {
        self.logs.lock().expect("lock").push(line.to_string());
    }
}

#[test]
fn run_with_progress_reports_on_progress_and_on_log_for_the_indexed_file() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let sink = RecordingSink::default();

    let resp = api::index_code::run_with_progress(
        &mut ctx,
        api::index_code::Request {
            repo: "sample".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
            mode: comemory::api::index_code::IndexMode::Incremental,
        },
        Some(&sink),
    )
    .expect("index_code run_with_progress");

    assert_eq!(resp.files_indexed, 1);
    let progress = sink.progress.lock().expect("lock");
    assert_eq!(
        progress.as_slice(),
        [(1, 1)],
        "the one-file fixture repo reports a single done==total report"
    );
    let logs = sink.logs.lock().expect("lock");
    assert_eq!(
        logs.as_slice(),
        ["src.rs"],
        "the one indexed file's relative path must be logged"
    );
}
