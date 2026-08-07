#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/search.rs`. Seeds real memories via the
//! `comemory` binary, then calls `api::search::run` directly against a
//! `Ctx` opened on the same data-dir — proving the extracted command core
//! reproduces `comemory search`'s hit/nav shape and honors the `track`
//! parameter (`cli::search::run` is byte-compat tested against CLI stdout
//! in `tests/cli__search.rs`; the HTTP surface's AC-2/AC-3 cross-checks
//! live in `tests/serve__routes__memories__search.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;

fn save(home: &tempfile::TempDir, body: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", body, "--kind", "decision", "--repo", "demo"])
        .assert()
        .success();
}

fn seeded_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "postgres pool exhausted under load spikes");
    save(&home, "redis cache eviction policy set to allkeys-lru");
    home
}

fn request(query: &str) -> api::search::Request {
    api::search::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: None,
        kind: None,
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

#[test]
fn run_returns_the_matching_hit_with_nav_metadata() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result = api::search::run(&mut ctx, request("postgres pool"), false).expect("search run");
    assert_eq!(result.hits.len(), 1);
    let hit = &result.hits[0];
    assert!(hit.body.contains("postgres"));
    let meta = result
        .nav
        .get(&hit.memory_id)
        .expect("nav entry for the hit");
    assert_eq!(meta.repo.as_deref(), Some("demo"));
}

#[test]
fn track_false_never_logs_a_query() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let logged_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM retrieval_log", [], |r| r.get(0))
        .expect("count retrieval_log before");

    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result = api::search::run(&mut ctx, request("postgres"), false).expect("search run");
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
}

#[test]
fn track_true_logs_a_query_and_reports_its_id() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result = api::search::run(&mut ctx, request("postgres"), true).expect("search run");
    assert!(
        result.query_id.is_some(),
        "track=true must log the query and report its id"
    );
}
