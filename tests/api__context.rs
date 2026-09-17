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
use comemory::api;
use comemory::config::{Config, Paths};
use comemory::store::connection;
use comemory::utilities::context::Ctx;

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

/// Issue #152, API twin: the bundle's memory `score` is the pipeline's
/// `final_score` — the same number `api::search::run` ranks the same query
/// with — never the old `0.0` placeholder.
#[test]
fn run_carries_the_search_pipeline_score_onto_each_memory_row() {
    let home = tempfile::tempdir().expect("tempdir");
    save(
        &home,
        "run_migration applies pending schema changes in order",
    );
    save(
        &home,
        "schema changes land through run_migration and a backup",
    );
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();

    let search = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let req = api::search::Request {
            query: "run_migration".to_string(),
            k: None,
            offset: 0,
            repo: None,
            kind: None,
            vector: None,
            since: None,
            until: None,
            as_of: None,
        };
        api::search::run(&mut ctx, req, false).expect("search run")
    };
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result = api::context::run(&mut ctx, request("run_migration"), false).expect("context run");

    let expected: Vec<(&str, f64)> = search
        .hits
        .iter()
        .map(|h| (h.memory_id.as_str(), h.parts.final_score))
        .collect();
    let got: Vec<(&str, f64)> = result
        .bundle
        .memories
        .iter()
        .map(|m| (m.id.as_str(), m.score))
        .collect();
    assert_eq!(got.len(), 2, "both memories match: {got:?}");
    assert!(
        got.iter().all(|(_, s)| *s > 0.0),
        "no placeholder scores: {got:?}"
    );
    // Same ids in the same order; the activation prior reads the clock, so
    // the two runs agree to well under 1e-6 rather than bit-for-bit.
    assert_eq!(
        got.len(),
        expected.len(),
        "got {got:?}, expected {expected:?}"
    );
    for ((gid, gs), (eid, es)) in got.iter().zip(&expected) {
        assert_eq!(gid, eid, "got {got:?}, expected {expected:?}");
        assert!((gs - es).abs() < 1e-6, "got {got:?}, expected {expected:?}");
    }
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
