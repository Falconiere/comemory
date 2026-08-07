//! Mirror test for `src/api/edges.rs`. Seeds a real supersede relation via
//! the `comemory` binary, then calls `api::edges::run` directly against a
//! `Ctx` opened on the same data-dir — proving the extracted command core
//! reproduces `comemory edges`'s triplet paging and the `allow_self_heal`
//! gate (`cli::edges::run` is byte-compat tested against CLI stdout in
//! `tests/cli__edges.rs`; the HTTP route lives in
//! `tests/serve__routes__graph.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use serde_json::Value;

fn save_id(home: &tempfile::TempDir, args: &[&str]) -> String {
    let mut argv = vec!["--json", "save"];
    argv.extend_from_slice(args);
    let out = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(&argv)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).expect("save --json");
    v["id"].as_str().expect("id").to_string()
}

fn seeded_home() -> (tempfile::TempDir, String, String) {
    let home = tempfile::tempdir().expect("tempdir");
    let old = save_id(
        &home,
        &["--kind", "decision", "--repo", "demo", "queue design v1"],
    );
    let new = save_id(
        &home,
        &[
            "--kind",
            "decision",
            "--repo",
            "demo",
            "--supersedes",
            &old,
            "queue design v2",
        ],
    );
    (home, old, new)
}

fn request(query: &str) -> api::edges::Request {
    api::edges::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
    }
}

#[test]
fn run_finds_the_supersede_triplet() {
    let (home, old, new) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let result =
        api::edges::run(&mut ctx, request("supersedes queue design"), true).expect("edges run");
    assert_eq!(result.hits.len(), 1);
    let hit = &result.hits[0];
    assert_eq!(hit.rel, "supersedes");
    assert_eq!(hit.src_id, new);
    assert_eq!(hit.dst_id, old);
    assert!(!result.has_more);
}

#[test]
fn allow_self_heal_false_leaves_an_empty_index_empty() {
    let (home, _old, _new) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    // Simulate the migration-0012 upgrade state: `edges` populated,
    // `edge_fts` deliberately empty.
    conn.execute("DELETE FROM edge_fts", [])
        .expect("empty edge_fts");

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result =
        api::edges::run(&mut ctx, request("supersedes queue design"), false).expect("edges run");
    assert!(
        result.hits.is_empty(),
        "allow_self_heal=false must not repopulate edge_fts: {:?}",
        result.hits.len()
    );
}

#[test]
fn allow_self_heal_true_repopulates_an_empty_index() {
    let (home, old, new) = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    conn.execute("DELETE FROM edge_fts", [])
        .expect("empty edge_fts");

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let result =
        api::edges::run(&mut ctx, request("supersedes queue design"), true).expect("edges run");
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].src_id, new);
    assert_eq!(result.hits[0].dst_id, old);
}
