#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/context.rs`. Seeds a real memory via the
//! `comemory` binary, then calls `api::context::run` directly against a
//! `Ctx` opened on the same data-dir — proving the extracted command core
//! assembles the same bundle `comemory context` does (`cli::context::run`
//! is byte-compat tested against CLI stdout in `tests/cli__context.rs`).

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

fn request(query: &str) -> api::context::Request {
    api::context::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: None,
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

#[test]
fn run_assembles_a_bundle_for_the_matched_memory() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "run_migration applies pending schema changes");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result = api::context::run(&mut ctx, request("run_migration"), false).expect("context run");
    assert_eq!(result.bundle.query, "run_migration");
    assert_eq!(result.bundle.memories.len(), 1);
    assert!(result.bundle.memories[0].body.contains("run_migration"));
}

#[test]
fn run_reports_zero_hits_for_an_unmatched_query() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "unrelated body about caching");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result =
        api::context::run(&mut ctx, request("zzz_never_matches_zzz"), false).expect("context run");
    assert!(result.bundle.memories.is_empty());
}

#[test]
fn track_true_bumps_memory_access_count() {
    let home = tempfile::tempdir().expect("tempdir");
    save(&home, "advisory lock guards the migration runner");
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    let before: i64 = conn
        .query_row("SELECT access_count FROM memories LIMIT 1", [], |r| {
            r.get(0)
        })
        .expect("read access_count before");
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        api::context::run(&mut ctx, request("advisory lock"), true).expect("context run");
    }
    let after: i64 = conn
        .query_row("SELECT access_count FROM memories LIMIT 1", [], |r| {
            r.get(0)
        })
        .expect("read access_count after");
    assert!(after > before, "track=true must bump access_count");
}
