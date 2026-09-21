#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/domains/code/index_code.rs`. Real temp git repos built
//! via the shared `common/git_*` helpers; `crate::domains::code::index_code::run` is called
//! directly against a `Ctx::borrowed` connection (no CLI process spawned).
//! `cli::index_code::run` stays byte-compat tested against CLI stdout in
//! `tests/cli__index_code.rs` / `tests/cli__index_code_2.rs`; the HTTP job
//! route (`POST /api/v1/code/index`) lives in `tests/serve__routes__code.rs`.

use crate::test_common::{git_sample, git_worktree};

use comemory::config::{Config, Paths};
use comemory::domains::code::index_code::IndexMode;
use comemory::store::connection;
use comemory::utilities::context::Ctx;
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

    let resp = crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "sample".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
            mode: comemory::domains::code::index_code::IndexMode::Incremental,
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

    let req = || crate::domains::code::index_code::Request {
        repo: "sample".into(),
        path: repo.to_str().expect("utf8 path").to_string(),
        mode: comemory::domains::code::index_code::IndexMode::Incremental,
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let first = crate::domains::code::index_code::run(&mut ctx, req()).expect("first run");
        assert_eq!(first.files_indexed, 1);
    }
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let second = crate::domains::code::index_code::run(&mut ctx, req()).expect("second run");
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

    let req = |mode| crate::domains::code::index_code::Request {
        repo: "sample".into(),
        path: repo.to_str().expect("utf8 path").to_string(),
        mode,
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let first = crate::domains::code::index_code::run(&mut ctx, req(IndexMode::Incremental))
            .expect("first run");
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
        crate::domains::code::index_code::run(&mut ctx, req(IndexMode::Full)).expect("full run")
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

    let err = crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "x".into(),
            path: not_a_repo.to_str().expect("utf8 path").to_string(),
            mode: comemory::domains::code::index_code::IndexMode::Incremental,
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

    let err = crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "x".into(),
            path: not_a_repo.to_str().expect("utf8 path").to_string(),
            mode: comemory::domains::code::index_code::IndexMode::Incremental,
        },
    )
    .expect_err("must error on a non-git path");
    assert!(!err.to_string().is_empty());
    assert!(
        !db_path.exists(),
        "index-code on an invalid --path must not create comemory.db"
    );
}

/// A [`comemory::utilities::progress::ProgressSink`] that just records every call, for
/// asserting the seam `serve::jobs::worker::RegistryProgressSink` uses in
/// production — proven end-to-end over real HTTP in
/// `tests/serve__jobs_progress.rs`; this proves the plain function contract.
#[derive(Default)]
struct RecordingSink {
    progress: std::sync::Mutex<Vec<(u64, u64)>>,
    logs: std::sync::Mutex<Vec<String>>,
}

impl comemory::utilities::progress::ProgressSink for RecordingSink {
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

    let resp = crate::domains::code::index_code::run_with_progress(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "sample".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
            mode: comemory::domains::code::index_code::IndexMode::Incremental,
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

/// A linked worktree is a second checkout of one repository, never a
/// repository of its own. A pre-worktree-rule git hook (or any caller that
/// passes the checkout's own basename) must not be able to mint a new repo
/// label for one — that is what filled the console's Repositories list with
/// one row per `git worktree add`.
#[test]
fn run_files_a_linked_worktree_under_its_main_repo_label() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let main = git_sample::build_sample_repo(workspace.path());
    let wt = workspace.path().join("feature-42");
    git_worktree::add_worktree(&main, &wt, "feature-42");

    let (paths, cfg, mut conn) = ctx_over(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    // Exactly what the stale hook passes: the worktree directory's basename.
    let resp = crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "feature-42".into(),
            path: wt.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run from a linked worktree");

    assert_eq!(
        resp.repo, "sample-repo",
        "the worktree must index under its main worktree's label"
    );
    let minted: i64 = conn
        .query_row(
            "SELECT count(*) FROM repo_marker WHERE repo = 'feature-42'",
            [],
            |r| r.get(0),
        )
        .expect("count repo_marker");
    assert_eq!(minted, 0, "no repository may be created for a worktree");
    // The repo is recorded at its MAIN working tree: `git worktree remove`
    // deletes the checkout that was walked, and a stored root pointing there
    // would leave `utilities::repo_root` resolving file ids against nothing.
    let root_path: String = conn
        .query_row(
            "SELECT root_path FROM repo_marker WHERE repo = 'sample-repo'",
            [],
            |r| r.get(0),
        )
        .expect("read repo_marker.root_path");
    assert_eq!(
        std::path::PathBuf::from(&root_path),
        main.canonicalize().expect("canonical main"),
        "the recorded root must be the main worktree, not the linked one"
    );
    let symbols: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo = 'sample-repo'",
            [],
            |r| r.get(0),
        )
        .expect("count code_symbols");
    assert!(
        symbols >= 2,
        "the worktree's symbols land under the main label, got {symbols}"
    );
    // The run HISTORY carries the redirected label too. `run_with_progress`
    // mutates one local `Request` through `&mut` and then hands `record_run` a
    // shared borrow of that same value, so there is no pre-redirect snapshot
    // for the history to record — a split record here would mean the console's
    // run list disagreed with the index it describes.
    let runs: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT repo FROM index_runs ORDER BY repo")
            .expect("prepare index_runs");
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query index_runs");
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect index_runs")
    };
    assert_eq!(
        runs,
        vec!["sample-repo".to_string()],
        "index_runs must record the redirected label, not the worktree one"
    );
}

/// The redirect is narrow: a label that already has a `repo_marker` row is
/// left alone, because it is either a repo the operator connected
/// deliberately (`crate::domains::code::repo_admin::connect` allows a custom label) or a
/// worktree row minted before this rule existed. Rewriting either would
/// silently repoint an existing code index at a different checkout.
#[test]
fn run_leaves_an_already_known_label_alone_even_in_a_worktree() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let main = git_sample::build_sample_repo(workspace.path());
    let wt = workspace.path().join("legacy-row");
    git_worktree::add_worktree(&main, &wt, "legacy-row");

    let (paths, cfg, mut conn) = ctx_over(home.path());
    conn.execute(
        "INSERT INTO repo_marker(repo, root_path) VALUES('legacy-row', ?1)",
        [wt.to_str().expect("utf8 path")],
    )
    .expect("seed a pre-existing marker row");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "legacy-row".into(),
            path: wt.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run over a known label");

    assert_eq!(
        resp.repo, "legacy-row",
        "a label this store already knows is never rewritten"
    );
    // Nor is its recorded root. Stamping the main worktree here would repoint
    // an existing code index at a checkout it was never indexed from, and would
    // leave `legacy-row` and `sample-repo` both claiming one root — a state
    // `crate::domains::code::repo_admin::connect` refuses to create.
    let root_path: String = conn
        .query_row(
            "SELECT root_path FROM repo_marker WHERE repo = 'legacy-row'",
            [],
            |r| r.get(0),
        )
        .expect("read repo_marker.root_path");
    assert_eq!(
        std::path::PathBuf::from(&root_path),
        wt.canonicalize().expect("canonical worktree"),
        "a known label keeps the root it was actually indexed from"
    );
}

/// The main checkout keeps whatever label the caller asks for — the redirect
/// must not turn `index-code --repo <custom>` on a real repository into a
/// rename.
#[test]
fn run_keeps_a_custom_label_for_a_main_checkout() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = crate::domains::code::index_code::run(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "my-own-label".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
    )
    .expect("index_code run");
    assert_eq!(resp.repo, "my-own-label");
}

/// Regression (CI run 35554592090's `store_locked` flake): the walk's
/// transaction must reserve SQLite's writer at `BEGIN` rather than upgrade
/// into it after its opening `ensure_repo_format` read.
///
/// The holder below is the real shape of the race: a `POST /api/v1/code/index`
/// job runs while the search path commits its telemetry write on another
/// connection. With the deferred begin this module used to open, the walk
/// took a read snapshot, the holder's commit invalidated it, and the walk's
/// first write failed *instantly* with `SQLITE_BUSY` — the one case
/// `busy_timeout` cannot wait out, which is why a 5000ms timeout never
/// masked it. Under `BEGIN IMMEDIATE` the walk simply waits for the holder
/// and then runs, so this test's wall time is the holder's hold, not a
/// timeout. Nothing races: the holder already owns the writer before the
/// walk starts, and its one bounded sleep is what keeps the writer held
/// across the walk — no assertion here waits on a deadline.
#[test]
fn run_survives_a_writer_held_by_another_connection() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = git_sample::build_sample_repo(workspace.path());
    let (paths, cfg, mut conn) = ctx_over(home.path());
    let db_path = paths.db_path();

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let held = rusqlite::Connection::open(&db_path).expect("open holder connection");
        held.pragma_update(None, "busy_timeout", 5000_i64)
            .expect("set busy_timeout on holder");
        held.execute("BEGIN IMMEDIATE", [])
            .expect("holder begins immediate");
        held.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('holder_probe', '1') \
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            [],
        )
        .expect("holder write");
        ready_tx.send(()).expect("signal holder ready");
        // Long enough that the walk is provably inside its transaction while
        // this commit lands — the exact interleave that broke the job.
        std::thread::sleep(std::time::Duration::from_millis(300));
        held.execute("COMMIT", []).expect("holder commits");
    });
    ready_rx.recv().expect("holder ready");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let resp = crate::domains::code::index_code::run_with_progress(
        &mut ctx,
        crate::domains::code::index_code::Request {
            repo: "sample".into(),
            path: repo.to_str().expect("utf8 path").to_string(),
            mode: IndexMode::Incremental,
        },
        None,
    )
    .expect("a commit from another connection must not fail the walk");
    holder.join().expect("holder thread");

    assert_eq!(
        resp.files_indexed, 1,
        "the walk must index the fixture file, not roll back on the contended write"
    );
    let symbols: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE repo = ?1",
            ["sample"],
            |r| r.get(0),
        )
        .expect("count code_symbols");
    assert!(
        symbols >= 2,
        "main + helper symbols must be durable after the contended commit, got {symbols}"
    );
    let holder_probe: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'holder_probe'",
            [],
            |r| r.get(0),
        )
        .expect("holder probe row");
    assert_eq!(
        holder_probe, "1",
        "the holder's own commit must survive too — the walk waits for the writer, \
         it does not steal it"
    );
}
