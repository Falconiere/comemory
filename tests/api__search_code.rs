#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/search_code.rs`. Indexes a real git fixture repo
//! via `comemory index-code`, then calls `api::search_code::run` directly
//! against a `Ctx` opened on the same data-dir — proving the extracted
//! command core reproduces `comemory search-code`'s hit/telemetry shape and
//! honors the `track` parameter (`cli::search_code::run` is byte-compat
//! tested against CLI stdout in `tests/cli__search_code.rs`; the HTTP
//! surface's parity live in `tests/serve__routes__code.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

/// Build a one-file fixture repo with two functions sharing a subtoken.
fn build_code_repo(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("code-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() {}\nfn unrelated_helper() {}\n",
        )],
        "init",
    );
    repo
}

/// Index `repo` into the comemory data dir rooted at `home` via the real
/// binary — no mock data, so the FTS/rank_score columns match production.
fn index_repo(home: &tempfile::TempDir, repo: &std::path::Path) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

fn seeded_home() -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("tempdir");
    let workspace = tempfile::tempdir().expect("workspace");
    let repo = build_code_repo(workspace.path());
    index_repo(&home, &repo);
    (home, workspace)
}

fn request(query: &str) -> api::search_code::Request {
    api::search_code::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: None,
        lang: None,
        vector: None,
    }
}

#[test]
fn run_returns_the_matching_hit() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result =
        api::search_code::run(&mut ctx, request("alpha_router"), false).expect("search run");
    assert!(!result.hits.is_empty(), "expected a hit for alpha_router");
    assert_eq!(result.hits[0].repo, "r");
    assert!(!result.index_empty);
}

#[test]
fn unsupported_lang_is_rejected() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let mut req = request("alpha_router");
    req.lang = Some("not-a-real-language".to_string());
    match api::search_code::run(&mut ctx, req, false) {
        Ok(_) => panic!("unsupported lang must be rejected"),
        Err(e) => assert!(
            e.to_string().contains("unsupported lang"),
            "unexpected error: {e}"
        ),
    }
}

#[test]
fn lang_alias_narrows_hits() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let mut req = request("alpha_router");
    req.lang = Some("rs".to_string());
    let result = api::search_code::run(&mut ctx, req, false).expect("search run");
    assert!(
        result.hits.iter().all(|h| h.lang == "rust"),
        "the rs alias must canonicalize to rust: {:?}",
        result.hits.iter().map(|h| &h.lang).collect::<Vec<_>>()
    );
}

#[test]
fn track_false_never_logs_a_query_or_bumps_access() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let logged_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log before");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result =
        api::search_code::run(&mut ctx, request("alpha_router"), false).expect("search run");
    assert!(
        result.query_id.is_none(),
        "track=false must not report a query_id"
    );
    drop(ctx);

    let logged_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log after");
    assert_eq!(
        logged_before, logged_after,
        "track=false must not write a row"
    );
    let touched: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE access_count > 0",
            [],
            |r| r.get(0),
        )
        .expect("count accessed");
    assert_eq!(touched, 0, "track=false must not bump access_count");
}

#[test]
fn track_true_logs_a_query_and_bumps_access() {
    let (home, _workspace) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result =
        api::search_code::run(&mut ctx, request("alpha_router"), true).expect("search run");
    assert!(
        result.query_id.is_some(),
        "track=true must log the query and report its id"
    );
    drop(ctx);

    let touched: i64 = conn
        .query_row(
            "SELECT count(*) FROM code_symbols WHERE access_count > 0",
            [],
            |r| r.get(0),
        )
        .expect("count accessed");
    assert!(touched > 0, "track=true must bump access_count");
}

#[test]
fn empty_index_reports_index_empty() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result = api::search_code::run(&mut ctx, request("anything"), false).expect("search run");
    assert!(result.hits.is_empty());
    assert!(
        result.index_empty,
        "a never-indexed store must report index_empty"
    );
}
