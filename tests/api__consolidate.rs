//! Mirror test for `src/api/consolidate.rs`. Seeds near-duplicate memories
//! via the real binary, then calls `api::consolidate::run` directly against
//! a `Ctx` opened on the same data-dir — proving the scan/cluster/page
//! middle reproduces `comemory consolidate`'s report (`cli::consolidate::run`
//! is byte-compat tested against CLI stdout in `tests/cli__consolidate.rs`;
//! the HTTP route lives in `tests/serve__routes__maint__mod.rs`).

use assert_cmd::Command;
use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;

fn save(home: &tempfile::TempDir, body: &str, repo: &str) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["save", body, "--kind", "note", "--repo", repo])
        .assert()
        .success();
}

/// Three near-duplicate memories plus one unrelated control.
fn seeded_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    for suffix in ["runner", "worker", "daemon"] {
        save(
            &home,
            &format!("postgres advisory lock ordering fix for the migration {suffix}"),
            "demo",
        );
    }
    save(
        &home,
        "terraform state locking through a dynamodb table",
        "demo",
    );
    home
}

fn request() -> api::consolidate::Request {
    api::consolidate::Request {
        radius: None,
        repo: None,
        all: false,
        k: None,
        offset: 0,
    }
}

#[test]
fn run_clusters_near_duplicates_with_keeper_first() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let report = api::consolidate::run(&mut ctx, request()).expect("consolidate run");
    assert_eq!(report.radius, cfg.rank.near_dup_hamming);
    assert_eq!(report.scanned, 4);
    assert_eq!(report.clustered, 3);
    assert_eq!(report.clusters.total, Some(1));
    assert_eq!(report.clusters.items[0].members.len(), 3);
    assert_eq!(report.clusters.items[0].members[0].hamming_to_keeper, 0);
}

#[test]
fn run_radius_zero_clusters_only_identical_bodies() {
    let home = seeded_home();
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let req = api::consolidate::Request {
        radius: Some(0),
        ..request()
    };
    let report = api::consolidate::run(&mut ctx, req).expect("consolidate run");
    assert_eq!(report.radius, 0);
    assert_eq!(report.clusters.total, Some(0));
}

#[test]
fn run_repo_filter_scopes_the_scan() {
    let home = tempfile::tempdir().expect("tempdir");
    save(
        &home,
        "redis cache eviction policy tuning for the session store",
        "alpha",
    );
    save(
        &home,
        "redis cache eviction policy tuning for the session cache",
        "beta",
    );
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let scoped_req = api::consolidate::Request {
        repo: Some("alpha".to_string()),
        ..request()
    };
    let scoped = api::consolidate::run(&mut ctx, scoped_req).expect("consolidate run");
    assert_eq!(scoped.scanned, 1);
    assert_eq!(scoped.clusters.total, Some(0));
}

#[test]
fn run_pages_clusters_with_k_and_offset() {
    let home = tempfile::tempdir().expect("tempdir");
    for suffix in ["runner", "worker"] {
        save(
            &home,
            &format!("postgres advisory lock ordering fix for the migration {suffix}"),
            "demo",
        );
    }
    for suffix in ["store", "cache"] {
        save(
            &home,
            &format!("redis cache eviction policy tuning for the session {suffix}"),
            "demo",
        );
    }
    let paths = Paths::new(home.path());
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let full = api::consolidate::run(&mut ctx, request()).expect("consolidate run");
    assert!(full.clusters.total.unwrap_or(0) >= 2);

    let paged_req = api::consolidate::Request {
        k: Some(1),
        ..request()
    };
    let paged = api::consolidate::run(&mut ctx, paged_req).expect("consolidate run");
    assert_eq!(paged.clusters.items.len(), 1);
    assert!(paged.clusters.has_more);
}
